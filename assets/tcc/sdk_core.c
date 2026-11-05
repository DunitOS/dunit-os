/* First M7.10 slice: compile/link/run on Dunit using only the umbrella header. */
#include <dunit.h>
#include <dunit.h> /* The public entry header must be repeatable. */

#define CHECK(test, reason) do { if (!(test)) { \
    puts("[CSDK-CORE] FAIL: " reason); return 1; } } while (0)

int main(int argc, char **argv, char **envp)
{
    if (argc == 2 && strcmp(argv[1], "child") == 0) return 7;
    CHECK(getpid() > 0, "getpid");
    CHECK(DUNIT_EBADF == EBADF && DUNIT_ENOENT == ENOENT, "errno definitions");
    CHECK(DUNIT_RIGHT_READ != DUNIT_RIGHT_WRITE, "rights definitions");
    CHECK(SYS_GET_PID != SYS_EXIT, "syscall definitions");
    CHECK(DUNIT_TRANSFER_ACTION_COPY != DUNIT_TRANSFER_ACTION_MOVE, "transfer definitions");

    const char *path = "/tmp/c-sdk-core.data";
    const char payload[] = "native C SDK";
    char buffer[sizeof payload] = {0};
    struct stat st;
    int fd = open(path, O_CREAT | O_TRUNC | O_RDWR, 0600);
    CHECK(fd >= 0, "open");
    CHECK(write(fd, payload, sizeof payload - 1) == sizeof payload - 1, "write");
    CHECK(fstat(fd, &st) == 0 && st.st_size == sizeof payload - 1, "fstat");
    CHECK(lseek(fd, 0, SEEK_SET) == 0, "seek");
    CHECK(read(fd, buffer, sizeof buffer) == sizeof payload - 1, "read");
    CHECK(strcmp(buffer, payload) == 0, "file contents");
    CHECK(close(fd) == 0 && unlink(path) == 0, "close/unlink");
    errno = 0;
    CHECK(stat(path, &st) == -1 && errno == ENOENT, "missing-file errno");
    errno = 0;
    CHECK(read(-1, buffer, 1) == -1 && errno == EBADF, "bad-fd errno");
    puts("[CSDK-CORE] file and errno OK");

    pid_t child;
    char *args[] = {"/tmp/c-sdk-core", "child", NULL};
    CHECK(posix_spawn(&child, args[0], NULL, NULL, args, envp) == 0, "spawn");
    int status = 0;
    CHECK(waitpid(child, &status, 0) == child, "wait");
    CHECK(WIFEXITED(status) && WEXITSTATUS(status) == 7, "child exit status");
    puts("[CSDK-CORE] spawn and wait OK");
    puts("[CSDK-CORE] ok");
    return 0;
}
