/* lseek over the Dunit Seek syscall (M6-D). Writes 10 bytes, then checks
 * SEEK_SET / SEEK_END / SEEK_CUR reposition the offset and that reads from the
 * new positions return the right bytes. Before Seek(80), lseek was an unmapped
 * sentinel -> ENOSYS and files were strictly sequential. Exercises the memfs
 * seek path (/tmp); the symmetric DunitFS path is implemented too. */
#define _GNU_SOURCE
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>

int main(void)
{
	const char *p = "/tmp/seek.txt";
	int fd = open(p, O_CREAT | O_RDWR | O_TRUNC, 0644);
	if (fd < 0) {
		printf("[SEEK] FAIL open\n");
		printf("[SEEK] DONE\n");
		return 0;
	}
	write(fd, "0123456789", 10);

	char b[8];
	off_t o = lseek(fd, 3, SEEK_SET);
	int n = read(fd, b, 3);
	int ok_set = (o == 3 && n == 3 && memcmp(b, "345", 3) == 0);

	o = lseek(fd, -2, SEEK_END);
	n = read(fd, b, 8);
	int ok_end = (o == 8 && n == 2 && memcmp(b, "89", 2) == 0);

	o = lseek(fd, 0, SEEK_CUR); /* at EOF now */
	int ok_cur = (o == 10);

	close(fd);

	if (ok_set && ok_end && ok_cur)
		printf("[SEEK] all OK\n");
	else
		printf("[SEEK] FAIL set=%d end=%d cur=%d\n", ok_set, ok_end, ok_cur);
	printf("[SEEK] DONE\n");
	return 0;
}
