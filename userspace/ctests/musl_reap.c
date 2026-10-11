/* waitpid reap-order correctness (M6-D): a parent spawns a SLOW child first
 * (sleeps, exits 3) then a FAST child (exits 7). POSIX wait(-1) must return the
 * child that has actually terminated — the fast one — even though the slow one
 * is listed first and still running. The old kernel picked the first-listed
 * child and returned EAGAIN while it ran, so a finished sibling could not be
 * reaped (breaking shell / make -j reap loops). Also checks that waiting with
 * no children left returns ECHILD. */
#define _GNU_SOURCE
#include <stdio.h>
#include <errno.h>
#include <spawn.h>
#include <sys/wait.h>

extern char **environ;

int main(void)
{
	pid_t a = 0, b = 0;
	char *aargv[] = { "slow_child", 0 };
	char *bargv[] = { "musl_child", "x", 0 };

	if (posix_spawn(&a, "/app/slow_child", 0, 0, aargv, environ) != 0) {
		printf("[REAP] FAIL spawn slow\n");
		printf("[REAP] DONE\n");
		return 0;
	}
	if (posix_spawn(&b, "/app/musl_child", 0, 0, bargv, environ) != 0) {
		printf("[REAP] FAIL spawn fast\n");
		printf("[REAP] DONE\n");
		return 0;
	}

	int st = 0;
	pid_t w1 = waitpid(-1, &st, 0);
	int c1 = WIFEXITED(st) ? WEXITSTATUS(st) : -1;
	if (w1 == b && c1 == 7)
		printf("[REAP] first_is_fast OK\n");
	else
		printf("[REAP] first_is_fast FAIL pid=%d code=%d (slow=%d fast=%d)\n",
		       (int)w1, c1, (int)a, (int)b);

	int st2 = 0;
	pid_t w2 = waitpid(-1, &st2, 0);
	int c2 = WIFEXITED(st2) ? WEXITSTATUS(st2) : -1;
	if (w2 == a && c2 == 3)
		printf("[REAP] second_is_slow OK\n");
	else
		printf("[REAP] second_is_slow FAIL pid=%d code=%d\n", (int)w2, c2);

	errno = 0;
	int st3 = 0;
	pid_t w3 = waitpid(-1, &st3, 0);
	if (w3 == -1 && errno == ECHILD)
		printf("[REAP] echild OK\n");
	else
		printf("[REAP] echild FAIL rc=%d errno=%d\n", (int)w3, errno);

	printf("[REAP] DONE\n");
	return 0;
}
