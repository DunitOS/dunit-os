/* W^X acceptance driver: the fact that this static musl binary loads and runs
 * at all proves the ELF loader still maps normal R-X text / RW-NX data after
 * the W^X tightening. It then spawns nx_child, which must be killed for trying
 * to execute on its (now non-executable) stack. */
#define _GNU_SOURCE
#include <stdio.h>
#include <spawn.h>
#include <sys/wait.h>

extern char **environ;

int main(void)
{
	pid_t pid = 0;
	char *argv[] = { "nx_child", 0 };
	int rc = posix_spawn(&pid, "/app/nx_child", 0, 0, argv, environ);
	if (rc != 0) {
		printf("[WX] nx_stack FAIL spawn rc=%d\n", rc);
		printf("[WX] DONE\n");
		return 0;
	}
	int status = 0;
	pid_t w = waitpid(pid, &status, 0);
	if (w == pid && WIFSIGNALED(status))
		printf("[WX] nx_stack signaled sig=%d OK\n", WTERMSIG(status));
	else if (w == pid && WIFEXITED(status))
		printf("[WX] nx_stack FAIL exited code=%d (stack was executable)\n", WEXITSTATUS(status));
	else
		printf("[WX] nx_stack FAIL w=%d\n", (int)w);
	printf("[WX] DONE\n");
	return 0;
}
