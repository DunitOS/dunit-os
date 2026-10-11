/* Compiled on the HOST by the Dunit cross-tcc (toolchains/dunit-tcc), NOT by
 * dunit-cc/gcc — proof that the tcc toolchain emits a Dunit ELF that actually
 * runs on the kernel. sum(1..100) must be 5050 (exercises tcc codegen: loop +
 * arithmetic). Two separate printf lines so each marker sits within one kernel
 * [STDOUT] write (the kernel logs stdout per write; a marker must not span a
 * printf conversion). */
#include <stdio.h>

int main(void)
{
	long sum = 0;
	for (int i = 1; i <= 100; i++)
		sum += i;
	printf("[TCC-HELLO] ok: built by tcc, ran on Dunit\n");
	printf("[TCC-HELLO] sum=%ld\n", sum);
	return sum == 5050 ? 0 : 1;
}
