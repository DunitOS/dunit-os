/* Kernel-hardening acceptance suite (M6.5): verifies the kernel refuses hostile
 * or buggy userspace input with a documented error AND keeps running, rather
 * than panicking/aborting the whole system. Each check prints its own marker;
 * a surviving kernel reaches [HARDEN] DONE.
 *
 *   1. SpawnArgs (syscall 79) with an in-range but unmapped block pointer must
 *      return EFAULT (used to deref an unmapped page in ring 0 -> kernel panic).
 *   2. DrawRect (syscall 12) from a non-display-owner must return EACCES.
 *   3. An unbounded RAM-file write must hit EFBIG (used to grow until the kernel
 *      heap was exhausted -> abort).
 *   4. A user process that raises #SS must be killed, the kernel surviving.
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <errno.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <spawn.h>
#include <sys/wait.h>

extern char **environ;

static void test_spawn_badptr(void)
{
	const char *path = "/app/musl_child";
	void *bad = (void *)0x400000000000UL; /* in user range, unmapped */
	long r = syscall(79, path, strlen(path), bad, 8UL, 1UL);
	if (r == -1 && errno == EFAULT)
		printf("[HARDEN] spawn_badptr EFAULT OK\n");
	else
		printf("[HARDEN] spawn_badptr FAIL r=%ld errno=%d\n", r, errno);
}

static void test_draw_rect_perm(void)
{
	long r = syscall(12, 0L, 0L, 1L, 1L, 0xffL);
	if (r == -1 && errno == EACCES)
		printf("[HARDEN] draw_rect EACCES OK\n");
	else
		printf("[HARDEN] draw_rect FAIL r=%ld errno=%d\n", r, errno);
}

static void test_memfs_cap(void)
{
	int fd = open("/tmp/hog", O_CREAT | O_WRONLY | O_TRUNC, 0644);
	if (fd < 0) {
		printf("[HARDEN] memfs_cap FAIL open errno=%d\n", errno);
		return;
	}
	static char buf[65536];
	memset(buf, 'A', sizeof buf);
	long total = 0;
	int hit = 0;
	/* POSIX write may return a short transfer; keep issuing writes until
	 * the bound is actually crossed, rather than assuming 64 KiB/call. */
	for (long i = 0; i < 16384; i++) {
		long w = write(fd, buf, sizeof buf);
		if (w < 0) { hit = (errno == EFBIG); break; }
		if (w == 0) break;
		total += w;
	}
	close(fd);
	if (hit)
		printf("[HARDEN] memfs_cap EFBIG after %ld MiB OK\n", total / (1024 * 1024));
	else
		printf("[HARDEN] memfs_cap FAIL total=%ld errno=%d\n", total, errno);
}

static void test_fault_isolation(void)
{
	pid_t pid = 0;
	char *argv[] = { "fault_child", 0 };
	int rc = posix_spawn(&pid, "/app/fault_child", 0, 0, argv, environ);
	if (rc != 0) {
		printf("[HARDEN] fault_child FAIL spawn rc=%d\n", rc);
		return;
	}
	int status = 0;
	pid_t w = waitpid(pid, &status, 0);
	if (w == pid && WIFSIGNALED(status))
		printf("[HARDEN] fault_child signaled sig=%d OK\n", WTERMSIG(status));
	else if (w == pid && WIFEXITED(status))
		printf("[HARDEN] fault_child FAIL exited code=%d\n", WEXITSTATUS(status));
	else
		printf("[HARDEN] fault_child FAIL w=%d\n", (int)w);
}

int main(void)
{
	test_spawn_badptr();
	test_draw_rect_perm();
	test_memfs_cap();
	test_fault_isolation();
	printf("[HARDEN] DONE\n");
	return 0;
}
