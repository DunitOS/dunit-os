/* Static musl test for the tcc-prep file ops: access(), ftruncate(), mkstemp().
 *
 * access(): existence check (Dunit has no perms yet, so R/W/X == F_OK).
 * ftruncate(): shrink then grow an open file, verifying the size via fstat and
 *   that growth zero-fills.
 * mkstemp(): atomic unique temp create (relies on the new O_EXCL). */
#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <stdio.h>

static int fileop_access(void)
{
	if (access("/app", F_OK) != 0) return 0;              /* dir exists */
	if (access("/app/zzz_does_not_exist", F_OK) == 0) return 0; /* must fail */
	return 1;
}

static int fileop_ftruncate(void)
{
	int fd = open("/tmp/ft.tmp", O_RDWR | O_CREAT | O_TRUNC, 0600);
	if (fd < 0) return 0;
	if (write(fd, "hello", 5) != 5) { close(fd); return 0; }

	struct stat st;
	if (ftruncate(fd, 2) != 0) { close(fd); return 0; }    /* shrink */
	if (fstat(fd, &st) != 0 || st.st_size != 2) { close(fd); return 0; }

	if (ftruncate(fd, 8) != 0) { close(fd); return 0; }    /* grow (zero-fill) */
	if (fstat(fd, &st) != 0 || st.st_size != 8) { close(fd); return 0; }

	/* The grown region must read back as zeros; the first 2 bytes are "he". */
	char buf[8];
	if (lseek(fd, 0, SEEK_SET) != 0) { close(fd); return 0; }
	if (read(fd, buf, 8) != 8) { close(fd); return 0; }
	if (buf[0] != 'h' || buf[1] != 'e' || buf[2] != 0 || buf[7] != 0) { close(fd); return 0; }
	close(fd);
	unlink("/tmp/ft.tmp");
	return 1;
}

static int fileop_mkstemp(void)
{
	char tmpl[] = "/tmp/fileopXXXXXX";
	int fd = mkstemp(tmpl);
	if (fd < 0) return 0;
	if (strstr(tmpl, "XXXXXX") != NULL) { close(fd); return 0; } /* template filled */
	if (write(fd, "x", 1) != 1) { close(fd); return 0; }
	close(fd);
	unlink(tmpl);
	return 1;
}

/* Exercise ftruncate on DunitFS (/persist) too, not just MemFS (/tmp): the
 * extent-based shrink (read+free+rewrite) and grow (zero-extend) are a distinct
 * code path. Returns 1 OK, 0 FAIL, -1 SKIP (if /persist is not writable here). */
static int fileop_ftruncate_persist(void)
{
	int fd = open("/persist/ft_persist.tmp", O_RDWR | O_CREAT | O_TRUNC, 0600);
	if (fd < 0) return -1; /* /persist not mounted/writable in this boot */
	if (write(fd, "abcdefghij", 10) != 10) { close(fd); unlink("/persist/ft_persist.tmp"); return 0; }

	struct stat st;
	if (ftruncate(fd, 4) != 0 || fstat(fd, &st) != 0 || st.st_size != 4) { close(fd); return 0; }
	if (ftruncate(fd, 12) != 0 || fstat(fd, &st) != 0 || st.st_size != 12) { close(fd); return 0; }

	char buf[12];
	if (lseek(fd, 0, SEEK_SET) != 0 || read(fd, buf, 12) != 12) { close(fd); return 0; }
	int ok = (buf[0] == 'a' && buf[3] == 'd' && buf[4] == 0 && buf[11] == 0);
	close(fd);
	unlink("/persist/ft_persist.tmp");
	return ok ? 1 : 0;
}

int main(void)
{
	if (fileop_access()) printf("[FILEOPS] access OK\n");
	else printf("[FILEOPS] access FAIL errno=%d\n", errno);

	if (fileop_ftruncate()) printf("[FILEOPS] ftruncate OK\n");
	else printf("[FILEOPS] ftruncate FAIL errno=%d\n", errno);

	int p = fileop_ftruncate_persist();
	if (p == 1) printf("[FILEOPS] ftruncate_persist OK\n");
	else if (p < 0) printf("[FILEOPS] ftruncate_persist SKIP\n");
	else printf("[FILEOPS] ftruncate_persist FAIL errno=%d\n", errno);

	if (fileop_mkstemp()) printf("[FILEOPS] mkstemp OK\n");
	else printf("[FILEOPS] mkstemp FAIL errno=%d\n", errno);

	printf("[FILEOPS] DONE\n");
	return 0;
}
