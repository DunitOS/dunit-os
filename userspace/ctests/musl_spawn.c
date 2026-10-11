/* Static musl process test for Dunit OS (M6-D): a C parent launches a C child
 * with posix_spawn (native SpawnArgs), passing argv, then reaps it with waitpid
 * (native WaitProcess) and checks the child's exit status. Also checks that
 * file actions (redirection) are refused with a documented ENOSYS (M6.1) rather
 * than silently ignored.
 */
#include <spawn.h>
#include <sys/wait.h>
#include <errno.h>
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

	/* M6.1: file actions are not yet carried by SpawnArgs — posix_spawn must
	 * REFUSE them with ENOSYS, not silently drop the redirection. */
	posix_spawn_file_actions_t fa;
	posix_spawn_file_actions_init(&fa);
	posix_spawn_file_actions_adddup2(&fa, 1, 2);
	pid_t p2 = 0;
	int rc2 = posix_spawn(&p2, "/app/musl_child", &fa, 0, argv, environ);
	posix_spawn_file_actions_destroy(&fa);
	if (rc2 == ENOSYS)
		printf("[MUSL-SPAWN] file-actions refused ENOSYS OK\n");
	else
		printf("[MUSL-SPAWN] FAIL file-actions rc=%d\n", rc2);
	return 0;
}

