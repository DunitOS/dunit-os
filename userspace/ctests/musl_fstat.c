/* fstat-by-fd over the native Dunit Fstat syscall (M6-D). The open+fstat+read
 * idiom (cp/tar/cat/loaders size a file via its fd) was impossible: fstat(fd)
 * returned ENOSYS. Now Fstat(81) stats the open descriptor. */
#define _GNU_SOURCE
#include <stdio.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>

int main(void)
{
	const char *p = "/tmp/fst.txt";
	int fd = open(p, O_CREAT | O_RDWR | O_TRUNC, 0644);
	if (fd < 0) {
		printf("[FSTAT] FAIL open\n");
		printf("[FSTAT] DONE\n");
		return 0;
	}
	write(fd, "hello fstat", 11);

	struct stat st;
	int r = fstat(fd, &st);
	if (r == 0 && st.st_size == 11 && S_ISREG(st.st_mode))
		printf("[FSTAT] fstat OK size=%ld\n", (long)st.st_size);
	else
		printf("[FSTAT] FAIL r=%d size=%ld mode=%o\n", r, (long)st.st_size,
		       (unsigned)st.st_mode);
	close(fd);
	printf("[FSTAT] DONE\n");
	return 0;
}
