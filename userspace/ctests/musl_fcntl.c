/* Dunit fcntl subset. F_DUPFD (built on Dup) + F_GETFD/F_SETFD (FD_CLOEXEC) +
 * F_GETFL/F_SETFL (status flags via the Fcntl syscall; O_NONBLOCK round-trips
 * and is honoured on pipe reads). Operates on stdout (fd 1): dup it to a floor,
 * write through the duplicate, then set/get CLOEXEC and a status flag. */
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
		write(nf, m, strlen(m));
		close(nf);
		printf("[FCNTL] F_DUPFD floor OK\n");
	} else {
		printf("[FCNTL] F_DUPFD FAIL nf=%d\n", nf);
	}

	if (fcntl(1, F_SETFD, FD_CLOEXEC) == 0 && (fcntl(1, F_GETFD) & FD_CLOEXEC))
		printf("[FCNTL] cloexec OK\n");
	else
		printf("[FCNTL] cloexec FAIL\n");

	int fl = fcntl(1, F_GETFL);
	if (fl >= 0 && fcntl(1, F_SETFL, fl | O_NONBLOCK) == 0
	    && (fcntl(1, F_GETFL) & O_NONBLOCK))
		printf("[FCNTL] setfl nonblock OK\n");
	else
		printf("[FCNTL] setfl FAIL fl=%d\n", fl);

	printf("[FCNTL] DONE\n");
	return 0;
}
