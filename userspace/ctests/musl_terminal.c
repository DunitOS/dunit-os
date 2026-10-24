#include <sys/ioctl.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <stdio.h>
#include <termios.h>
#include <spawn.h>
#include <sys/wait.h>
#include <string.h>
#define CHECK(x) do { if (!(x)) { printf("[TERMINAL-ABI] FAIL line=%d errno=%d\n", __LINE__, errno); return 1; } } while (0)
int main(int argc, char **argv) {
    if (argc > 1) {
        struct winsize inherited;
        CHECK(isatty(0)==1 && isatty(1)==1);
        CHECK(ioctl(1,TIOCGWINSZ,&inherited)==0 && inherited.ws_row==42 && inherited.ws_col==132);
        puts("[TERMINAL-ABI] inherited PTY stdout OK");
        return 8;
    }
    struct winsize size = {0}, got;
    CHECK(isatty(0) == 1);
    CHECK(ioctl(0, TIOCGWINSZ, &size) == 0);
    CHECK(size.ws_row == 0 && size.ws_col == 0);
    size.ws_row = 42; size.ws_col = 132; size.ws_xpixel = 1056; size.ws_ypixel = 756;
    CHECK(ioctl(0, TIOCSWINSZ, &size) == 0);
    memset(&got, 0, sizeof got);
    int fd = dup(1); CHECK(fd >= 0);
    CHECK(ioctl(fd, TIOCGWINSZ, &got) == 0);
    CHECK(memcmp(&size, &got, sizeof got) == 0);
    CHECK(close(fd) == 0);
    int count = -1; CHECK(ioctl(0, FIONREAD, &count) == 0 && count == 3);
    char bytes[3]; CHECK(read(0, bytes, 3) == 3 && memcmp(bytes, "abc", 3) == 0);
    CHECK(ioctl(0, FIONREAD, &count) == 0 && count == 0);
    errno = 0; CHECK(ioctl(0, TIOCGWINSZ, (void *)1) == -1 && errno == EFAULT);
    struct termios attributes;
    errno = 0; CHECK(tcgetattr(0, &attributes) == -1 && errno == EOPNOTSUPP);
    errno = 0; CHECK(ioctl(999, TIOCGWINSZ, &got) == -1 && errno == EBADF);
    fd = open("/tmp/ioctl-regular", O_CREAT | O_RDWR, 0600); CHECK(fd >= 0);
    errno = 0; CHECK(ioctl(fd, TIOCGWINSZ, &got) == -1 && errno == ENOTTY);
    CHECK(close(fd) == 0); CHECK(unlink("/tmp/ioctl-regular") == 0);
    char payload[1024]; memset(payload, '@', sizeof payload);
    for (int i=0;i<32;i++) {
        size_t done=0;
        while(done<sizeof payload){ssize_t n=write(1,payload+done,sizeof payload-done);CHECK(n>0);done+=n;}
    }
    pid_t child; char *args[]={"musl_terminal","inherit",0};
    CHECK(posix_spawn(&child,"/app/musl_terminal",0,0,args,0)==0);
    int status; CHECK(waitpid(child,&status,0)==child && WIFEXITED(status) && WEXITSTATUS(status)==8);
    puts("[TERMINAL-ABI] child geometry, queue, dup, errors PASS");
    return 0;
}
