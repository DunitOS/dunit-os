/* Capability-model acceptance driver (M6.5). Launched from the kernel console
 * via `exec musl_cap`, so it holds the privileged (system-control) capability.
 * It confirms a privileged process may use a read-only terminal diagnostic,
 * then spawns an UNPRIVILEGED child (posix_spawn) that probes the gates
 * (see cap_child.c): read-only diag allowed, mutating diag + cross-process kill
 * denied with EPERM. */
#define _GNU_SOURCE
#include <stdio.h>
#include <unistd.h>
#include <sys/syscall.h>
#include <spawn.h>
#include <sys/wait.h>

extern char **environ;

int main(void)
{
	char buf[128];

	long ro = syscall(78, "lsblk", (long)5, buf, (long)sizeof buf);
	printf("[CAP] parent diag_ro %s (r=%ld)\n", ro >= 0 ? "OK" : "FAIL", ro);

	pid_t pid = 0;
	char *argv[] = { "cap_child", 0 };
	int rc = posix_spawn(&pid, "/app/cap_child", 0, 0, argv, environ);
	if (rc != 0) {
		printf("[CAP] FAIL spawn rc=%d\n", rc);
		printf("[CAP] DONE\n");
		return 0;
	}
	int st = 0;
	waitpid(pid, &st, 0);
	printf("[CAP] DONE\n");
	return 0;
}
