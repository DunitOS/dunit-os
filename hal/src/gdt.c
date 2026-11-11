#include "cpu_local.h"
#include "hal.h"
#include <stdint.h>

typedef struct {
  uint16_t limit_low;
  uint16_t base_low;
  uint8_t base_middle;
  uint8_t access;
  uint8_t granularity;
  uint8_t base_high;
} __attribute__((packed)) gdt_entry_t;

typedef struct {
  uint16_t limit;
  uint64_t base;
} __attribute__((packed)) gdt_ptr_t;

typedef struct {
  uint32_t reserved0;
  uint64_t rsp0;
  uint64_t rsp1;
  uint64_t rsp2;
  uint64_t reserved1;
  uint64_t ist1;
  uint64_t ist2;
  uint64_t ist3;
  uint64_t ist4;
  uint64_t ist5;
  uint64_t ist6;
  uint64_t ist7;
  uint64_t reserved2;
  uint16_t reserved3;
  uint16_t iomap_base;
} __attribute__((packed)) tss_t;

typedef struct {
  gdt_entry_t gdt[7];
  gdt_ptr_t pointer;
  tss_t tss;
  uint8_t irq_stack[16384] __attribute__((aligned(16)));
  uint8_t df_stack[16384] __attribute__((aligned(16)));
  uint8_t nmi_stack[16384] __attribute__((aligned(16)));
} cpu_gdt_t;
static cpu_gdt_t bsp_state;
static cpu_gdt_t *states[HAL_CPU_CAPACITY];
extern uint64_t kernel_cpu_alloc(uint64_t size, uint64_t alignment);
int hal_gdt_prepare(uint64_t index) {
  if (index == 0 || index >= HAL_CPU_CAPACITY)
    return 0;
  states[index] = (cpu_gdt_t *)kernel_cpu_alloc(sizeof(cpu_gdt_t), 16);
  return states[index] != 0;
}

extern void gdt_flush(uint64_t);
extern void gdt_load_tss(uint16_t);

static void gdt_set_gate(cpu_gdt_t *state, int num, uint64_t base,
                         uint64_t limit, uint8_t access, uint8_t gran) {
  state->gdt[num].base_low = (base & 0xFFFF);
  state->gdt[num].base_middle = (base >> 16) & 0xFF;
  state->gdt[num].base_high = (base >> 24) & 0xFF;
  state->gdt[num].limit_low = (limit & 0xFFFF);
  state->gdt[num].granularity = (limit >> 16) & 0x0F;
  state->gdt[num].granularity |= gran & 0xF0;
  state->gdt[num].access = access;
}

static void gdt_set_tss_gate(cpu_gdt_t *state, int num, uint64_t base,
                             uint32_t limit) {
  state->gdt[num].limit_low = limit & 0xFFFF;
  state->gdt[num].base_low = base & 0xFFFF;
  state->gdt[num].base_middle = (base >> 16) & 0xFF;
  state->gdt[num].access = 0x89;
  state->gdt[num].granularity = ((limit >> 16) & 0x0F);
  state->gdt[num].base_high = (base >> 24) & 0xFF;

  uint64_t *upper = (uint64_t *)&state->gdt[num + 1];
  *upper = (base >> 32) & 0xFFFFFFFF;
}

void hal_init_gdt(void) {
  uint64_t index = hal_cpu_index();
  if (index == 0)
    states[0] = &bsp_state;
  cpu_gdt_t *state = states[index];
  if (!state)
    for (;;)
      __asm__ volatile("cli;hlt");
  state->pointer.limit = (sizeof(gdt_entry_t) * 7) - 1;
  state->pointer.base = (uint64_t)&state->gdt;
  gdt_set_gate(state, 0, 0, 0, 0, 0);
  gdt_set_gate(state, 1, 0, 0xFFFFFFFF, 0x9A, 0xA0);
  gdt_set_gate(state, 2, 0, 0xFFFFFFFF, 0x92, 0xA0);
  gdt_set_gate(state, 3, 0, 0xFFFFFFFF, 0xFA, 0xA0);
  gdt_set_gate(state, 4, 0, 0xFFFFFFFF, 0xF2, 0xA0);
  state->tss.rsp0 = (uint64_t)&state->irq_stack[sizeof state->irq_stack];
  state->tss.ist1 = (uint64_t)&state->df_stack[sizeof state->df_stack];
  state->tss.ist2 = (uint64_t)&state->nmi_stack[sizeof state->nmi_stack];
  state->tss.iomap_base = sizeof(tss_t);
  gdt_set_tss_gate(state, 5, (uint64_t)&state->tss, sizeof(tss_t) - 1);
  gdt_flush((uint64_t)&state->pointer);
  gdt_load_tss(0x28);
}
