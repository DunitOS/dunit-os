#include "cpu_local.h"
static hal_cpu_local_t locals[HAL_CPU_CAPACITY] __attribute__((aligned(64)));
static uint8_t bsp_stack[16384] __attribute__((aligned(16)));
extern uint64_t kernel_cpu_alloc(uint64_t size, uint64_t alignment);
int hal_cpu_local_prepare(uint64_t index) {
  if (index == 0 || index >= HAL_CPU_CAPACITY)
    return 0;
  uint64_t stack = kernel_cpu_alloc(sizeof bsp_stack, 16);
  if (!stack)
    return 0;
  locals[index].bootstrap_stack_top = stack + sizeof bsp_stack;
  return 1;
}
static volatile uint32_t initialized;
static void msr(uint32_t reg, uint64_t value) {
  __asm__ volatile("wrmsr" ::"c"(reg), "a"((uint32_t)value),
                   "d"((uint32_t)(value >> 32))
                   : "memory");
}
void hal_cpu_local_init(uint64_t index) {
  if (index >= HAL_CPU_CAPACITY)
    for (;;)
      __asm__ volatile("cli;hlt");
  hal_cpu_local_t *cpu = &locals[index];
  cpu->self_ptr = (uint64_t)cpu;
  cpu->index = index;
  if (index == 0)
    cpu->bootstrap_stack_top = (uint64_t)&bsp_stack[sizeof bsp_stack];
  if (!cpu->bootstrap_stack_top)
    for (;;)
      __asm__ volatile("cli;hlt");
  msr(0xc0000101, (uint64_t)cpu);
  msr(0xc0000102, 0);
  __atomic_store_n(&initialized, 1, __ATOMIC_RELEASE);
}
uint64_t hal_cpu_index(void) {
  if (!__atomic_load_n(&initialized, __ATOMIC_ACQUIRE))
    return 0;
  uint64_t index;
  __asm__ volatile("movq %%gs:%c1,%0"
                   : "=r"(index)
                   : "i"(offsetof(hal_cpu_local_t, index)));
  return index;
}
