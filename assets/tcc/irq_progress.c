/* Compiled by native TinyCC; DebugLog(5) exists only in boot-smoke kernels. */
#include <dunit/syscall.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#define STRINGIFY_INNER(value) #value
#define STRINGIFY(value) STRINGIFY_INNER(value)

/* A real SysV function boundary makes XMM registers caller-saved without
   relying on TinyCC's inline-asm clobber-name support. Every callee-saved GPR
   modified by the test is saved here and restored before returning to C. */
extern long irq_context_probe(const uint64_t *before, uint64_t *after,
                              uint64_t *gprs);
extern long irq_debug_probe(void);
__asm__(
    ".text\n"
    ".globl irq_context_probe\n"
    "irq_context_probe:\n"
    "push %rbx\n"
    "push %r12\n"
    "push %r13\n"
    "push %r14\n"
    "movups (%rdi), %xmm7\n"
    "mov %rsi, %r9\n"
    "mov %rdx, %r10\n"
    "mov $0x12345678, %rbx\n"
    "mov $0x23456789, %r12\n"
    "mov $0x3456789a, %r13\n"
    "mov $0x456789ab, %r14\n"
    "mov $" STRINGIFY(SYS_DEBUG_LOG) ", %rax\n"
    "mov $5, %rdi\n"
    "syscall\n"
    "movups %xmm7, (%r9)\n"
    "mov %rbx, 0(%r10)\n"
    "mov %r12, 8(%r10)\n"
    "mov %r13, 16(%r10)\n"
    "mov %r14, 24(%r10)\n"
    "pop %r14\n"
    "pop %r13\n"
    "pop %r12\n"
    "pop %rbx\n"
    "ret\n"
    ".globl irq_debug_probe\n"
    "irq_debug_probe:\n"
    "mov $" STRINGIFY(SYS_DEBUG_LOG) ", %rax\n"
    "mov $5, %rdi\n"
    "syscall\n"
    "ret\n"
);

int main(void) {
    uint64_t before[2] = {0x0123456789abcdefULL, 0xfedcba9876543210ULL};
    uint64_t after[2] = {0, 0};
    uint64_t gprs[4] = {0, 0, 0, 0};
    long result;
    errno = 123;
    result = irq_context_probe(before, after, gprs);
    if (result != 0x13572468 || after[0] != before[0] || after[1] != before[1] ||
        gprs[0] != 0x12345678 || gprs[1] != 0x23456789 ||
        gprs[2] != 0x3456789a || gprs[3] != 0x456789ab || errno != 123)
        return 1;
    if (irq_debug_probe() != 0x13572468) return 2;
    puts("[IRQ-PROGRESS-USER] syscall result GPR SIMD TLS preserved OK");
    return 0;
}
