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

static int run_input(const char *name, char *const argv[], char *const env[],
               int expected_status, const char *expected_output, const char *input)
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
    const char *input_tail = input ? strchr(input, '\n') : NULL;
    if (input_tail) input_tail++;
    size_t initial_input = input ? (input_tail ? (size_t)(input_tail - input) : strlen(input)) : 0;
    long written = input ? syscall(SYS_PTY_WRITE, pty, input, initial_input) : 0;
    char output[4096];
    size_t used = 0;
    int io_error = input && written != (long)initial_input, drained = 0, reaped = 0, status = 0;
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
            output[used] = 0;
            if (input_tail && strstr(output, "\nEND\n")) {
                // Exercise a real idle wait before dispatching to the same worker.
                usleep(20000);
                size_t remaining = strlen(input_tail);
                if (syscall(SYS_PTY_WRITE, pty, input_tail, remaining) != (long)remaining) io_error = 1;
                input_tail = NULL;
                if (io_error) break;
            }
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
            && strstr(output, "DWM_EVENT ") == NULL
            && strstr(output, "ANIMATE ") == NULL
            && strstr(output, "COMMAND ") == NULL
            && strstr(output, "WIDGET ") == NULL
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

static int run(const char *name, char *const argv[], char *const env[], int expected_status, const char *expected_output)
{
    return run_input(name, argv, env, expected_status, expected_output, NULL);
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
    char *event_env[] = {"DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/events.lua",
        "DWM_LUA_WIDTH=1", "DWM_LUA_HEIGHT=1", "DWM_LUA_WINDOWS=0",
        "DWM_LUA_EVENT=window_minimize", "DWM_LUA_WINDOW=7", "DWM_LUA_TARGET=9", NULL};
    failed += run("sdk-event", noargs, event_env, 0,
        "DWM_EVENT 2 window_minimize 7 1\nANIMATE 7 9 genie 420 none 0 0"
        " 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\nEND\n");
    event_env[0] = "DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-property.lua";
    event_env[4] = "DWM_LUA_EVENT=window_open";
    failed += run("sdk-property", noargs, event_env, 0,
        "DWM_EVENT 2 window_open 7 1\nANIMATE 7 9 fade 250 opacity 16384 65535"
        " 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\nEND\n");
    event_env[0] = "DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-spring.lua";
    failed += run("sdk-spring", noargs, event_env, 0,
        "DWM_EVENT 2 window_open 7 1\nANIMATE 7 9 fade 250 opacity 0 65535"
        " 0 14980 32579 49435 63345 73222 78900 80872 80011 77329 73782 70152 66991 64608 63104 62421 62402 62841 63532 64297 65001 65562 65944 66147 66198 66138 66009 65852 65699 65569 65476 65420 65535\nEND\n");
    event_env[0] = "DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-properties.lua";
    failed += run("sdk-properties", noargs, event_env, 0,
        "DWM_EVENT 2 window_open 7 12\n"
        "ANIMATE 7 9 fade 200 opacity 16384 65535 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 x -100 200 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 y 20 300 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 width 100 800 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 height 80 600 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 scale 32768 131070 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 translation_x -30 40 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 translation_y 10 -20 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 blur 0 12 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 shadow 0 20 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 corner_radius 0 16 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "ANIMATE 7 9 fade 200 color 16711680 4294967295 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\n"
        "END\n");
    event_env[0] = "DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-invalid-property.lua";
    failed += run("sdk-invalid-property", noargs, event_env, 1, "");
    event_env[4] = "DWM_LUA_EVENT=window_minimize";
    event_env[0] = "DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-invalid.lua";
    failed += run("sdk-invalid-curve", noargs, event_env, 1, "");
    event_env[0] = "DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-commands.lua";
    event_env[4] = "DWM_LUA_EVENT=reload";
    failed += run("sdk-commands", noargs, event_env, 0,
        "DWM_EVENT 2 reload 7 0\nCOMMAND launcher\nCOMMAND launch:terminal\nCOMMAND workspace:2\nEND\n");
    event_env[0] = "DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-invalid-command.lua";
    failed += run("sdk-invalid-command", noargs, event_env, 1, "");
    event_env[0] = "DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-widget.lua";
    event_env[4] = "DWM_LUA_EVENT=widget_update";
    failed += run("sdk-widget", noargs, event_env, 0,
        "DWM_EVENT 2 widget_update 7 0\nWIDGET 9 4289379276 435055203432250ae29c93\nWIDGET 9 -1 -\nEND\n");
    event_env[0] = "DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-invalid-widget.lua";
    failed += run("sdk-invalid-widget", noargs, event_env, 1, "");
    event_env[0] = "DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-component.lua";
    event_env[4] = "DWM_LUA_EVENT=panel_hover";
    failed += run("sdk-component", noargs, event_env, 0,
        "DWM_EVENT 2 panel_hover 7 1\nANIMATE 0 9 fade 140 opacity 0 32768 0 2048 4096 6144 8192 10240 12288 14336 16384 18432 20480 22528 24576 26624 28672 30720 32768 34815 36863 38911 40959 43007 45055 47103 49151 51199 53247 55295 57343 59391 61439 63487 65535\nEND\n");
    char *daemon_env[] = {"DWM_LUA_SCRIPT=/assets/dwm/scripts/tests/event-state.lua",
        "DWM_LUA_WIDTH=1", "DWM_LUA_HEIGHT=1", "DWM_LUA_WINDOWS=0", "DWM_LUA_DAEMON=1", NULL};
    failed += run_input("sdk-daemon-state", noargs, daemon_env, 0,
        "DWM_EVENT 2 reload 0 0\nCOMMAND workspace:1\nEND\n"
        "DWM_EVENT 2 reload 0 0\nCOMMAND workspace:2\nEND\n",
        "EVENT reload 0 0\nEVENT reload 0 0\nSTOP\n");
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
