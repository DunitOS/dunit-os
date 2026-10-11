/* poll/select over a pipe (POSIX-gap list). The kernel Poll syscall is a
 * non-blocking readiness snapshot; musl's poll() adds the timeout wait and
 * select() rides on poll(). Checks: empty pipe is not ready (non-blocking);
 * after a write the read end is POLLIN; select agrees; closing the write end
 * makes the read end POLLHUP/POLLIN (EOF is a ready event). */
#include <poll.h>
#include <sys/select.h>
#include <unistd.h>
#include <stdio.h>

int main(void)
{
	int p[2];
	if (pipe(p) != 0) { printf("[POLL] FAIL pipe\n[POLL] DONE\n"); return 0; }

	struct pollfd pf = { p[0], POLLIN, 0 };
	if (poll(&pf, 1, 0) == 0)
		printf("[POLL] empty nonblock OK\n");
	else
		printf("[POLL] empty nonblock FAIL\n");

	write(p[1], "x", 1);
	pf.revents = 0;
	if (poll(&pf, 1, 1000) == 1 && (pf.revents & POLLIN))
		printf("[POLL] pollin OK\n");
	else
		printf("[POLL] pollin FAIL re=%d\n", pf.revents);

	fd_set rs;
	FD_ZERO(&rs);
	FD_SET(p[0], &rs);
	struct timeval tv = { 1, 0 };
	if (select(p[0] + 1, &rs, 0, 0, &tv) == 1 && FD_ISSET(p[0], &rs))
		printf("[POLL] select OK\n");
	else
		printf("[POLL] select FAIL\n");

	char b;
	read(p[0], &b, 1); /* drain the "x" */

	close(p[1]);
	pf.revents = 0;
	if (poll(&pf, 1, 1000) == 1 && (pf.revents & (POLLIN | POLLHUP)))
		printf("[POLL] hup OK\n");
	else
		printf("[POLL] hup FAIL re=%d\n", pf.revents);

	close(p[0]);
	printf("[POLL] DONE\n");
	return 0;
}
