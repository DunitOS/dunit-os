/* File-backed mmap (POSIX-gap list). The kernel Mmap syscall now honours a real
 * fd + offset: a MAP_PRIVATE file mapping is eagerly populated from the file
 * (writes stay private, the sub-page tail past EOF reads zero), MAP_SHARED is a
 * documented EOPNOTSUPP (no coherent write-back yet, never a silent private
 * substitute). Stock musl mmap() already passes all six args, so this is a pure
 * userspace test. Checks: read-through matches the file, the zero-fill tail,
 * mmap leaves the fd offset untouched, a private write never reaches the file,
 * a page offset maps the right region, and MAP_SHARED is refused. */
#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <errno.h>
#include <stdio.h>

#define PATH "/tmp/mm.dat"
#define SIZE 6000               /* spans 2 pages, not a page multiple */

static void fail(const char *why)
{
	printf("[MMAP] FAIL %s\n[MMAP] DONE\n", why);
	_exit(0);
}

static unsigned char pat(int i) { return (unsigned char)((i * 31 + 7) & 0xff); }

int main(void)
{
	/* Lay down a known pattern. */
	unsigned char buf[SIZE];
	for (int i = 0; i < SIZE; i++)
		buf[i] = pat(i);
	int w = open(PATH, O_WRONLY | O_CREAT | O_TRUNC, 0644);
	if (w < 0 || write(w, buf, SIZE) != SIZE)
		fail("write");
	close(w);

	int fd = open(PATH, O_RDONLY);
	if (fd < 0)
		fail("open");
	struct stat st;
	if (fstat(fd, &st) != 0 || st.st_size != SIZE)
		fail("fstat");

	/* 1. read-through + zero-fill tail. */
	unsigned char *p = mmap(0, SIZE, PROT_READ, MAP_PRIVATE, fd, 0);
	if (p == MAP_FAILED)
		fail("mmap read");
	if (p[0] != pat(0) || p[1234] != pat(1234) || p[SIZE - 1] != pat(SIZE - 1))
		fail("content");
	if (p[SIZE] != 0 || p[8191] != 0)         /* past EOF, inside the last page */
		fail("zerofill");
	printf("[MMAP] read OK\n");

	/* 2. mmap must not move the fd's offset. */
	unsigned char head[4];
	if (read(fd, head, 4) != 4 || head[0] != pat(0) || head[3] != pat(3))
		fail("cursor");
	printf("[MMAP] cursor OK\n");
	munmap(p, SIZE);

	/* 3. a private write stays private — the file is untouched. */
	unsigned char *pw = mmap(0, SIZE, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
	if (pw == MAP_FAILED)
		fail("mmap rw");
	pw[0] = 0xEE;
	if (pw[0] != 0xEE)
		fail("private store");
	int fc = open(PATH, O_RDONLY);
	unsigned char fb;
	if (fc < 0 || read(fc, &fb, 1) != 1 || fb != pat(0))
		fail("private leaked to file");
	close(fc);
	munmap(pw, SIZE);
	printf("[MMAP] private-write OK\n");

	/* 4. a page-aligned offset maps the right slice. */
	unsigned char *q = mmap(0, 1000, PROT_READ, MAP_PRIVATE, fd, 4096);
	if (q == MAP_FAILED)
		fail("mmap offset");
	if (q[0] != pat(4096) || q[999] != pat(5095))
		fail("offset content");
	munmap(q, 1000);
	printf("[MMAP] offset OK\n");

	/* 5. MAP_SHARED is refused (not faked as private). */
	errno = 0;
	void *s = mmap(0, SIZE, PROT_READ, MAP_SHARED, fd, 0);
	if (s != MAP_FAILED)
		fail("shared accepted");
	if (errno != EOPNOTSUPP)
		fail("shared errno");
	printf("[MMAP] shared rejected OK\n");

	close(fd);
	printf("[MMAP] DONE\n");
	return 0;
}
