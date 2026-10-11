/* Static musl process test for Dunit OS (M6-D): a C parent launches a C child
 * with posix_spawn (native SpawnArgs), passing argv, then reaps it with waitpid
 * (native WaitProcess) and checks the child's exit status.
 */
#include <spawn.h>
#include <sys/wait.h>
#include <stdio.h>

extern char **environ;

int main(void)
{
	char *argv[] = { "musl_child", "hi", 0 };
	pid_t pid = 0;

	int rc = posix_spawn(&pid, "/app/musl_child", 0, 0, argv, environ);
	if (rc != 0) { printf("[MUSL-SPAWN] FAIL spawn rc=%d\n", rc); return 1; }

	int status = 0;
	pid_t w = waitpid(pid, &status, 0);

	printf("[MUSL-SPAWN] child pid=%d waited=%d exited=%d code=%d\n",
	       (int)pid, (int)w, WIFEXITED(status), WEXITSTATUS(status));
	if (w == pid && WIFEXITED(status) && WEXITSTATUS(status) == 7)
		printf("[MUSL-SPAWN] OK\n");
	else
		printf("[MUSL-SPAWN] FAIL\n");
	return 0;
}
