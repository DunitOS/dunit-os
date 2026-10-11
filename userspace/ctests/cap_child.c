/* Unprivileged probe for the capability model: posix_spawn'd by musl_cap, so it
 * does NOT hold the privileged (system-control) capability. It confirms:
 *   - a read-only terminal diagnostic is allowed to any process;
 *   - a MUTATING terminal diagnostic (mkfs.dunit) is denied with EPERM;
 *   - killing a process that is not its child (pid 1) is denied with EPERM. */
#define _GNU_SOURCE
#include <stdio.h>
#include <errno.h>
#include <unistd.h>
#include <sys/syscall.h>

/* Dunit syscall numbers (see abi/syscalls.abi): 18 = KillProcess, 78 = TerminalDiag. */
int main(void)
{
	char buf[64];

	long ro = syscall(78, "lsblk", (long)5, buf, (long)sizeof buf);
	if (ro >= 0)
		printf("[CAP] child diag_ro OK\n");
	else
		printf("[CAP] child diag_ro FAIL r=%ld errno=%d\n", ro, errno);

	errno = 0;
	long mut = syscall(78, "mkfs.dunit", (long)10, buf, (long)sizeof buf);
	if (mut == -1 && errno == EPERM)
		printf("[CAP] child diag_mut_denied OK\n");
	else
		printf("[CAP] child diag_mut_denied FAIL r=%ld errno=%d\n", mut, errno);

	errno = 0;
	long k = syscall(18, (long)1);
	if (k == -1 && errno == EPERM)
		printf("[CAP] child kill_denied OK\n");
	else
		printf("[CAP] child kill_denied FAIL r=%ld errno=%d\n", k, errno);

	return 0;
}
