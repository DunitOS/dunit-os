/* On-device tcc compile probe (no #include, no libc): proves the Dunit tcc's
 * full front-to-back pipeline — preprocess, parse, codegen, write ELF object —
 * works ON the kernel. Compiled with `tcc -c` into /tmp (MemFS), no sysroot
 * (headers/crt/libc) needed. Packed into the image as /assets/tcc/probe.c. */
int tri(int n)
{
	int s = 0;
	for (int i = 1; i <= n; i++)
		s += i;
	return s;
}

int square(int x)
{
	return x * x;
}
