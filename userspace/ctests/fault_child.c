/* Fault-isolation probe for the hardening suite: a CPL=3 process that raises a
 * stack-segment fault (#SS) by making RSP non-canonical and touching the stack.
 * The kernel must TERMINATE this process (not panic the whole system). The
 * parent (musl_harden) reaps it and checks it died by signal. */

int main(void)
{
	/* 0xdead000000000000 is non-canonical (bits 63:48 != sign-extension of
	 * bit 47), so a stack reference through it raises #SS from user mode. */
	__asm__ volatile(
		"movabs $0xdead000000000000, %%rsp\n\t"
		"pushq $0\n\t"
		:
		:
		: "memory");
	return 0; /* unreachable — we fault on the push above */
}
