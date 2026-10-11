/* posix_spawn file actions = redirection (M6-D). The kernel now carries file
 * actions in the SpawnArgs block and applies open/dup2/close to the child's fd
 * table before it runs. Here the parent redirects a child's stdout to a file
 * (open a temp fd on /tmp/redir.out, dup2 it onto fd 1, close the temp fd),
 * spawns /app/musl_child, waits, then reads the file back and checks the
 * child's stdout landed there — i.e. `child > file` works. */
#include <spawn.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <stdio.h>

extern char **environ;
#define OUT "/tmp/redir.out"

static void done(const char *msg)
{
	printf("[REDIR] %s\n[REDIR] DONE\n", msg);
}

int main(void)
{
	posix_spawn_file_actions_t fa;
	posix_spawn_file_actions_init(&fa);
	posix_spawn_file_actions_addopen(&fa, 7, OUT, O_WRONLY | O_CREAT | O_TRUNC, 0644);
	posix_spawn_file_actions_adddup2(&fa, 7, 1);
	posix_spawn_file_actions_addclose(&fa, 7);

	char *argv[] = { "musl_child", "hi", 0 };
	pid_t pid = 0;
	int rc = posix_spawn(&pid, "/app/musl_child", &fa, 0, argv, environ);
	posix_spawn_file_actions_destroy(&fa);
	if (rc != 0) { done("FAIL spawn"); return 0; }

	int status = 0;
	if (waitpid(pid, &status, 0) != pid || !WIFEXITED(status) || WEXITSTATUS(status) != 7) {
		done("FAIL child status");
		return 0;
	}

	int fd = open(OUT, O_RDONLY);
	if (fd < 0) { done("FAIL open out"); return 0; }
	char buf[256];
	int n = read(fd, buf, sizeof buf - 1);
	close(fd);
	if (n <= 0) { done("FAIL read out"); return 0; }
	buf[n] = 0;

	/* musl_child prints "[CHILD] argc=2 argv1=hi" to stdout — which the file
	 * actions routed into OUT instead of the console. */
	if (strstr(buf, "argc=2 argv1=hi"))
		done("child stdout redirected to file OK");
	else
		done("FAIL content");
	return 0;
}
