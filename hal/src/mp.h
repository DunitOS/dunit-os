#ifndef DUNIT_MP_H
#define DUNIT_MP_H
#include "cpu_local.h"
#include <stdint.h>
uint64_t hal_mp_apic_physical(void);
int hal_mp_prepare(uint64_t mapped);
int hal_mp_start(void);
uint64_t hal_mp_count(void);
void hal_mp_snapshot(uint64_t index, uint64_t *out);
int hal_mp_interrupt(uint64_t vector);
int hal_mp_run_pmm_probe(void);
#endif
