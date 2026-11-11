/* Isolated AP bring-up only: no process, heap, VFS or userspace dispatch on
 * APs. */
#include "mp.h"
#include "hal.h"
#include "limine.h"
extern struct LIMINE_MP(response) * get_mp_response(void);
extern void hal_load_idt(void);
extern void hal_mp_switch_stack(uint64_t index, struct LIMINE_MP(info) * info);
extern uint64_t kernel_mp_ticks(void);
uint64_t hal_mp_cr3;
uint64_t hal_mp_stack_tops[HAL_CPU_CAPACITY];
extern uint64_t kernel_cpu_alloc(uint64_t size, uint64_t alignment);
extern int hal_cpu_local_prepare(uint64_t index);
extern int hal_gdt_prepare(uint64_t index);
enum { PROBE = 241, TIMER = 242, REPLY = 243, SPURIOUS = 255 };
typedef struct {
  uint64_t apic, ready, request, ack, ticks, work;
  uint8_t padding[16];
} __attribute__((aligned(64))) cpu_slot;
static cpu_slot slots[HAL_CPU_CAPACITY];
static uint64_t count = 1;
static volatile uint32_t prepared;
static uint64_t mmio;
static int x2;
static uint64_t rdmsr(uint32_t r) {
  uint32_t a, d;
  __asm__ volatile("rdmsr" : "=a"(a), "=d"(d) : "c"(r));
  return ((uint64_t)d << 32) | a;
}
static void wrmsr(uint32_t r, uint64_t v) {
  __asm__ volatile("wrmsr" ::"c"(r), "a"((uint32_t)v), "d"((uint32_t)(v >> 32))
                   : "memory");
}
static uint32_t load(uint32_t offset) {
  return x2 ? (uint32_t)rdmsr(0x800 + offset / 16)
            : *(volatile uint32_t *)(mmio + offset);
}
static void store(uint32_t offset, uint32_t value) {
  if (x2)
    wrmsr(0x800 + offset / 16, value);
  else
    *(volatile uint32_t *)(mmio + offset) = value;
}
static void pause_cpu(void) { __asm__ volatile("pause"); }
static int elapsed(uint64_t start, uint64_t ticks) {
  return kernel_mp_ticks() - start >= ticks;
}
static int wait_ticks(uint64_t ticks) {
  uint64_t t = kernel_mp_ticks();
  for (uint64_t n = 0; n < 500000000; n++) {
    if (elapsed(t, ticks))
      return 1;
    pause_cpu();
  }
  return 0;
}
uint64_t hal_mp_apic_physical(void) {
  struct LIMINE_MP(response) *r = get_mp_response();
  if (!r)
    return 0;
  x2 = (rdmsr(0x1b) & (1ull << 10)) != 0;
  return x2 ? 0 : rdmsr(0x1b) & 0xffffff000ull;
}
static void apic_init(int bsp) {
  wrmsr(0x1b, rdmsr(0x1b) | (1ull << 11));
  store(0xf0, 0x100 | SPURIOUS);
  store(0x80, 0);
  store(0x350, bsp ? 0x700 : 0x10000);
  store(0x360, bsp ? 0x400 : 0x10000);
  store(0x320, 0x10000 | TIMER);
  store(0x370, 0x10000);
  store(0x280, 0);
  store(0xb0, 0);
}
int hal_mp_prepare(uint64_t mapped) {
  struct LIMINE_MP(response) *r = get_mp_response();
  if (!r)
    return 1;
  if (!r->cpu_count || r->cpu_count > HAL_CPU_CAPACITY)
    return 0;
  x2 = (rdmsr(0x1b) & (1ull << 10)) != 0;
  mmio = mapped;
  if (!x2 && !mmio)
    return 0;
  slots[0].apic = r->bsp_lapic_id;
  count = r->cpu_count;
  for (uint64_t i = 1; i < count; i++) {
    uint64_t stack = kernel_cpu_alloc(65536, 16);
    if (!stack || !hal_cpu_local_prepare(i) || !hal_gdt_prepare(i))
      return 0;
    hal_mp_stack_tops[i] = stack + 65536;
  }
  apic_init(1);
  __atomic_store_n(&prepared, 1, __ATOMIC_RELEASE);
  return 1;
}
static int timer_init(void) {
  store(0x3e0, 3);
  store(0x320, 0x10000 | TIMER);
  if (!wait_ticks(1))
    return 0; /* align calibration with a PIT edge */
  store(0x380, 0xffffffffu);
  if (!wait_ticks(5))
    return 0;
  uint32_t passed = 0xffffffffu - load(0x390);
  if (!passed)
    return 0;
  store(0x320, 0x20000 | TIMER);
  store(0x380, passed / 5);
  return 1;
}
static int send(uint32_t apic, uint8_t vector) {
  uint64_t flags;
  __asm__ volatile("pushfq;pop %0;cli" : "=r"(flags)::"memory");
  if (x2)
    wrmsr(0x830, ((uint64_t)apic << 32) | vector);
  else {
    unsigned n;
    for (n = 0; n < 1000000 && (load(0x300) & 0x1000); n++)
      pause_cpu();
    if (n == 1000000) {
      __asm__ volatile("push %0;popfq" ::"r"(flags) : "memory");
      return 0;
    }
    store(0x310, apic << 24);
    store(0x300, vector);
  }
  __asm__ volatile("push %0;popfq" ::"r"(flags) : "memory");
  return 1;
}
static void ap_enter(struct LIMINE_MP(info) * info) {
  uint64_t i = info->extra_argument;
  if (i >= count)
    for (;;)
      __asm__ volatile("cli;hlt");
  hal_mp_switch_stack(i, info);
}
void hal_mp_ap_main(struct LIMINE_MP(info) * info) {
  uint64_t i = info->extra_argument;
  hal_cpu_local_init(i);
  hal_init_gdt();
  hal_load_idt();
  syscall_init();
  apic_init(0);
  __asm__ volatile("sti");
  if (!timer_init())
    for (;;)
      __asm__ volatile("cli;hlt");
  uint64_t sum = 0;
  for (unsigned n = 0; n < 65536; n++)
    sum += (uint64_t)n * (i + 1);
  __atomic_store_n(&slots[i].work, sum, __ATOMIC_RELAXED);
  __atomic_store_n(&slots[i].ready, 1, __ATOMIC_RELEASE);
  for (;;)
    __asm__ volatile("hlt");
}
int hal_mp_interrupt(uint64_t vector) {
  if (!__atomic_load_n(&prepared, __ATOMIC_ACQUIRE))
    return 0;
  if (vector != PROBE && vector != TIMER && vector != REPLY &&
      vector != SPURIOUS)
    return 0;
  uint64_t i = hal_cpu_index();
  if (i >= count)
    return 0;
  if (vector == TIMER) {
    uint64_t ticks = __atomic_add_fetch(&slots[i].ticks, 1, __ATOMIC_RELAXED);
    /* A parked AP has no runnable tasks. Once its timer has been proved,
     * stop periodic wakeups; an IPI still wakes HLT normally. */
    if (i && ticks >= 2 && __atomic_load_n(&slots[i].request, __ATOMIC_ACQUIRE))
      store(0x320, 0x10000 | TIMER);
  }
  if (vector == PROBE) {
    uint64_t request = __atomic_load_n(&slots[i].request, __ATOMIC_ACQUIRE);
    __atomic_store_n(&slots[i].ack, request, __ATOMIC_RELEASE);
    if (__atomic_load_n(&slots[i].ticks, __ATOMIC_RELAXED) >= 2)
      store(0x320, 0x10000 | TIMER);
    send((uint32_t)slots[0].apic, REPLY);
  }
  if (vector == REPLY)
    __atomic_fetch_add(&slots[0].ack, 1, __ATOMIC_RELEASE);
  if (vector != SPURIOUS)
    store(0xb0, 0);
  return 1;
}
int hal_mp_start(void) {
  struct LIMINE_MP(response) *r = get_mp_response();
  if (!r) {
    slots[0].ready = 1;
    return 1;
  }
  __asm__ volatile("mov %%cr3,%0" : "=r"(hal_mp_cr3));
  uint64_t slot = 1;
  for (uint64_t n = 0; n < r->cpu_count; n++) {
    struct LIMINE_MP(info) *cpu = r->cpus[n];
    if (cpu->lapic_id == r->bsp_lapic_id)
      continue;
    if (!x2 && cpu->lapic_id > 255)
      return 0;
    slots[slot].apic = cpu->lapic_id;
    cpu->extra_argument = slot;
    __atomic_store_n(&cpu->goto_address, ap_enter, __ATOMIC_RELEASE);
    slot++;
  }
  if (slot != count)
    return 0;
  if (!timer_init())
    return 0;
  slots[0].ready = 1;
  uint64_t start = kernel_mp_ticks();
  for (uint64_t spins = 0;; spins++) {
    int all = 1;
    for (uint64_t n = 1; n < count; n++)
      if (!__atomic_load_n(&slots[n].ready, __ATOMIC_ACQUIRE))
        all = 0;
    if (all)
      break;
    if (elapsed(start, 1000) || spins == 500000000)
      return 0;
    pause_cpu();
  }
  /* APIC IRR coalesces equal pending vectors: validate one round trip at a
   * time rather than treating interrupt edges as a reliable message count. */
  for (uint64_t n = 1; n < count; n++) {
    __atomic_store_n(&slots[n].request, 1, __ATOMIC_RELEASE);
    if (!send((uint32_t)slots[n].apic, PROBE))
      return 0;
    start = kernel_mp_ticks();
    for (uint64_t spins = 0;; spins++) {
      if (__atomic_load_n(&slots[n].ack, __ATOMIC_ACQUIRE) == 1 &&
          __atomic_load_n(&slots[0].ack, __ATOMIC_ACQUIRE) == n)
        break;
      if (elapsed(start, 1000) || spins == 500000000)
        return 0;
      pause_cpu();
    }
  }
  start = kernel_mp_ticks();
  for (uint64_t spins = 0;; spins++) {
    int all = 1;
    for (uint64_t n = 0; n < count; n++)
      if (__atomic_load_n(&slots[n].ticks, __ATOMIC_RELAXED) < 2)
        all = 0;
    if (all)
      break;
    if (elapsed(start, 1000) || spins == 500000000)
      return 0;
    pause_cpu();
  }
  /* BSP timekeeping/preemption remains on PIT; no duplicate periodic IRQ. */
  store(0x320, 0x10000 | TIMER);
  return 1;
}
uint64_t hal_mp_count(void) { return count; }
void hal_mp_snapshot(uint64_t i, uint64_t *out) {
  if (i >= count)
    return;
  out[0] = slots[i].apic;
  out[1] = __atomic_load_n(&slots[i].ready, __ATOMIC_ACQUIRE);
  out[2] = __atomic_load_n(&slots[i].ack, __ATOMIC_ACQUIRE);
  out[3] = __atomic_load_n(&slots[i].ticks, __ATOMIC_RELAXED);
  out[4] = __atomic_load_n(&slots[i].work, __ATOMIC_RELAXED);
}
