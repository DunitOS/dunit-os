/* Dunit fcntl subset (gap list, step 1): F_DUPFD is real (built on Dup), the
 * flag commands are documented ENOSYS. Dup stdout (fd 1) to a floor of 10, write
 * through the duplicate (it must reach stdout), confirm the new fd honoured the
 * floor, then confirm F_GETFL reports ENOSYS rather than a fake value. */
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>
#include <stdio.h>

int main(void)
{
	int nf = fcntl(1, F_DUPFD, 10);
	if (nf >= 10) {
		const char *m = "[FCNTL] dupfd write via dup OK\n";
		write(nf, m, strlen(m));          /* reaches stdout through the dup */
		close(nf);
		printf("[FCNTL] F_DUPFD floor OK\n");
	} else {
		printf("[FCNTL] F_DUPFD FAIL nf=%d\n", nf);
	}

	errno = 0;
	int fl = fcntl(1, F_GETFL);
	if (fl == -1 && errno == ENOSYS)
		printf("[FCNTL] F_GETFL ENOSYS OK\n");
	else
		printf("[FCNTL] F_GETFL FAIL fl=%d errno=%d\n", fl, errno);

	printf("[FCNTL] DONE\n");
	return 0;
}
