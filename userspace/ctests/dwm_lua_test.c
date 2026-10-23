/* Native PTY captures merged stdout/stderr; validate policy bytes or failure
 * diagnostics plus no policy header, and observe the actual child exit status.
 * Pinned libc lacks pipe/file-action adaptation, so use the generated ABI. */
#define _GNU_SOURCE
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>
#include "../lua_runtime/output.h"
#include "../../abi/include/dunit/syscall.h"

extern char **environ;

static int run(const char *name, char *const argv[], char *const env[],
               int expected_status, const char *expected_output)
{
    char envblock[1024];
    size_t envlen = 0;
    if (argv[1]) {
        const char *keys[] = {"DWM_LUA_SCRIPT", "DWM_LUA_WIDTH", "DWM_LUA_HEIGHT", "DWM_LUA_WINDOWS"};
        for (int i = 0; i < 4; i++) {
            int n = snprintf(envblock + envlen, sizeof envblock - envlen,
                             "%s=%s", keys[i], argv[i + 1]);
            if (n < 0 || (size_t)n >= sizeof envblock - envlen) return 1;
            envlen += (size_t)n + 1;
        }
    } else {
        for (size_t i = 0; env[i]; i++) {
            size_t n = strlen(env[i]) + 1;
            if (n > sizeof envblock - envlen) return 1;
            memcpy(envblock + envlen, env[i], n);
            envlen += n;
        }
    }
    long pty = syscall(SYS_PTY_CREATE);
    if (pty < 0) {
        dunit_lua_log(STDOUT_FILENO, "[DWM-LUA] FAIL %s: pty-create\n", name);
        return 1;
    }
    const char *path = "/app/dwm_lua";
    long pid = syscall(SYS_PTY_SPAWN, path, strlen(path), pty, envblock, envlen);
    if (pid < 0) {
        syscall(SYS_PTY_CLOSE, pty);
        dunit_lua_log(STDOUT_FILENO, "[DWM-LUA] FAIL %s: pty-spawn\n", name);
        return 1;
    }
    char output[4096];
    size_t used = 0;
    int io_error = 0, drained = 0, reaped = 0, status = 0;
    /* Bounded master polling; never block in libc waitpid on a hung child. */
    for (int tick = 0; tick < 1000; tick++) {
        char block[256];
        long n = syscall(SYS_PTY_READ, pty, block, sizeof block);
        if (n < 0) {
            if (errno == EPIPE) drained = 1;
            else { io_error = 1; break; }
        } else if (n > 0) {
            /* PTY may translate LF to CRLF. Normalize just transport CR. */
            for (long i = 0; i < n; i++) {
                if (block[i] == '\r') continue;
                if (used == sizeof output - 1) { io_error = 1; break; }
                output[used++] = block[i];
            }
            if (io_error) break;
        }
        if (!reaped) {
            pid_t w = waitpid((pid_t)pid, &status, WNOHANG);
            if (w == pid) reaped = 1;
            else if (w < 0) { io_error = 1; break; }
        }
        if (drained && reaped) break;
        if (!n || drained) usleep(10000);
    }
    if (!reaped || !drained || io_error) {
        /* Kill then boundedly reap even on malformed/flooded output. */
        syscall(SYS_KILL_PROCESS, pid);
        for (int tick = 0; !reaped && tick < 100; tick++) {
            if (waitpid((pid_t)pid, &status, WNOHANG) == pid) reaped = 1;
            else usleep(10000);
        }
        io_error = 1;
    }
    syscall(SYS_PTY_CLOSE, pty);
    output[used] = 0;
    int output_ok;
    if (!expected_status)
        output_ok = used == strlen(expected_output)
            && !memcmp(output, expected_output, used);
    else {
        /* This transport merges stderr: require an actual diagnostic and
         * reject any policy emission, instead of claiming zero stdout. */
        output_ok = strstr(output, "DWM_LUA ") == NULL
            && strstr(output, "RECT ") == NULL
            && (strstr(output, "dwm_lua:") != NULL
                || strstr(output, "usage: dwm_lua") != NULL);
    }
    if (io_error || !reaped || !WIFEXITED(status)
        || WEXITSTATUS(status) != expected_status || !output_ok) {
        dunit_lua_log(STDOUT_FILENO, "[DWM-LUA] FAIL %s: status=%d bytes=%zu\n",
                     name, status, used);
        return 1;
    }
    return dunit_lua_log(STDOUT_FILENO,
        "[DWM-LUA] %s status=%d output=validated OK\n", name, expected_status) ? 1 : 0;
}

int main(void)
{
    int failed = 0;
    char *columns[] = {"dwm_lua", "/assets/dwm/scripts/columns.lua", "1200", "800", "3", NULL};
    const char *columns_output = "DWM_LUA 1 1200 800 3\n"
        "RECT 1 0 0 400 800\nRECT 2 400 0 400 800\nRECT 3 800 0 400 800\n"
        "ANIMATION 140 ease_out\nEND\n";
    failed += run("columns", columns, environ, 0, columns_output);
    char *libraries[] = {"dwm_lua", "/assets/dwm/scripts/tests/libraries.lua", "1200", "800", "3", NULL};
    failed += run("libraries", libraries, environ, 0, columns_output);
    char *monocle[] = {"dwm_lua", "/assets/dwm/scripts/monocle.lua", "1200", "800", "2", NULL};
    failed += run("monocle", monocle, environ, 0, "DWM_LUA 1 1200 800 2\n"
        "RECT 1 0 0 1200 800\nRECT 2 0 0 1200 800\nANIMATION 0 linear\nEND\n");
    char *empty[] = {"dwm_lua", "/assets/dwm/scripts/columns.lua", "1200", "800", "0", NULL};
    failed += run("zero-windows", empty, environ, 0,
                  "DWM_LUA 1 1200 800 0\nANIMATION 140 ease_out\nEND\n");
    char *noargs[] = {"dwm_lua", NULL};
    char *env[] = {"DWM_LUA_SCRIPT=/assets/dwm/scripts/columns.lua",
        "DWM_LUA_WIDTH=1200", "DWM_LUA_HEIGHT=800", "DWM_LUA_WINDOWS=3", NULL};
    failed += run("environment", noargs, env, 0, columns_output);
    const char *names[] = {"outside", "missing", "quota", "memory", "syntax"};
    for (size_t i = 0; i < sizeof names / sizeof names[0]; i++) {
        char path[128];
        snprintf(path, sizeof path, "/assets/dwm/scripts/tests/%s.lua", names[i]);
        char *args[] = {"dwm_lua", path, "1200", "800", "1", NULL};
        failed += run(names[i], args, environ, 1, "");
    }
    char *invalid[] = {"dwm_lua", "/assets/dwm/scripts/columns.lua", "0", "800", "1", NULL};
    failed += run("invalid-arguments", invalid, environ, 2, "");
    if (failed) { dunit_lua_log(STDOUT_FILENO, "[DWM-LUA] FAIL regression\n"); return 1; }
    return dunit_lua_log(STDOUT_FILENO, "[DWM-LUA] regression OK\n") ? 1 : 0;
}
