#include <stdio.h>

/* Full on-device build target: tcc compiles AND links this on Dunit (against
 * the /system sysroot — headers + libc.a + crt + libtcc1.a), then the kernel
 * runs the result. Pure-literal printf so the marker is one [STDOUT] write. */
int main(void)
{
	printf("[TCC-ONDEV] hello compiled and linked on Dunit by tcc\n");
	return 0;
}
