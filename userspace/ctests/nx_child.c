/* W^X probe: a CPL=3 process that tries to execute bytes on its own stack. With
 * a non-executable (NX) stack the instruction fetch faults and the kernel kills
 * this process; with an executable stack the planted `ret` returns cleanly and
 * the process exits 0 (which the parent flags as a FAIL — NX was missing). */

typedef void (*fnptr)(void);

int main(void)
{
	unsigned char code[16];
	code[0] = 0xC3; /* x86 `ret` */
	__asm__ volatile("" ::: "memory");
	fnptr f = (fnptr)(void *)code;
	f(); /* NX stack -> #PF here (process killed); else returns cleanly */
	return 0; /* reached ONLY if the stack was executable (NX hole) */
}
