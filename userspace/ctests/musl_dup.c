/* dup / dup2 over the native Dunit Dup syscall (M6-D). Covers the three things
 * that broke before (dup/dup2 were ENOSYS):
 *   1. dup2 redirects stdout to a file and dup/dup2 restores it;
 *   2. a dup shares the underlying open file's offset;
 *   3. closing one of a dup'd pair keeps the other working (VFS refcount). */
#define _GNU_SOURCE
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>

int main(void)
{
	/* 1: redirect stdout -> file, then restore it. */
	int saved = dup(1);
	int f = open("/tmp/dup.txt", O_CREAT | O_RDWR | O_TRUNC, 0644);
	dup2(f, 1);
	write(1, "redir-ok", 8);
	dup2(saved, 1);
	close(saved);
	lseek(f, 0, SEEK_SET);
	char b[16];
	int n = read(f, b, sizeof b);
	if (n < 0) n = 0;
	int ok_redir = (n == 8 && memcmp(b, "redir-ok", 8) == 0);
	close(f);

	/* 2 + 3: dup shares the offset; closing one keeps the other alive. */
	int g = open("/tmp/dup2.txt", O_CREAT | O_RDWR | O_TRUNC, 0644);
	write(g, "ABCDEF", 6);
	int g2 = dup(g);
	lseek(g, 0, SEEK_SET);
	char c[4];
	int m = read(g2, c, 3);
	int ok_share = (m == 3 && memcmp(c, "ABC", 3) == 0);
	close(g);
	int m2 = read(g2, c, 3);
	int ok_refcount = (m2 == 3 && memcmp(c, "DEF", 3) == 0);
	close(g2);

	if (ok_redir && ok_share && ok_refcount)
		printf("[DUP] all OK\n");
	else
		printf("[DUP] FAIL redir=%d share=%d refcount=%d\n", ok_redir, ok_share,
		       ok_refcount);
	printf("[DUP] DONE\n");
	return 0;
}
