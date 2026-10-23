/* Official Lua compatibility probe; no alternate parser or OS bindings. */
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"
#include "../lua_runtime/output.h"

#include "../lua_runtime/limits.h"
#include "../lua_runtime/libraries.h"

static int initialize(lua_State *L)
{
    luaL_requiref(L, LUA_GNAME, luaopen_base, 1);
    lua_pop(L, 1);
    dunit_lua_open_pure_libraries(L);
    /* Base includes file-loading entrypoints: remove them explicitly. No
     * luaL_openlibs, os/io/package/debug/coroutine library is exposed. */
    const char *removed[] = {"dofile", "loadfile", "load", "collectgarbage"};
    for (size_t i = 0; i < sizeof removed / sizeof removed[0]; i++) {
        lua_pushnil(L);
        lua_setglobal(L, removed[i]);
    }
    return 0;
}

static int probe(const char *name, const char *source, int expected,
                 int expect_quota)
{
    struct dunit_lua_limits b = { .bytes = 256 * 1024, .instructions = 20000 };
    lua_State *L = lua_newstate(dunit_lua_alloc, &b, 0x44554e49u);
    if (!L) { dunit_lua_log(STDOUT_FILENO, "[LUA] FAIL %s: state allocation\n", name); return 1; }
    *(struct dunit_lua_limits **)lua_getextraspace(L) = &b;
    lua_pushcfunction(L, initialize);
    int status = lua_pcall(L, 0, 0, 0);
    if (status != LUA_OK) {
        dunit_lua_log(STDOUT_FILENO, "[LUA] FAIL %s: initialization\n", name);
        lua_close(L);
        return 1;
    }
    status = luaL_loadbufferx(L, source, strlen(source), name, "t");
    if (status == LUA_OK) {
        lua_sethook(L, dunit_lua_hook, LUA_MASKCOUNT, 1000);
        status = lua_pcall(L, 0, 0, 0);
        lua_sethook(L, NULL, 0, 0);
    }
    int failed = status != expected || b.quota_hit != expect_quota
        || b.peak > b.bytes;
    if (failed) {
        const char *message = lua_tostring(L, -1);
        dunit_lua_log(STDOUT_FILENO, "[LUA] FAIL %s: status=%d quota=%d error=%s\n",
               name, status, b.quota_hit, message ? message : "(none)");
    }
    lua_close(L);
    if (b.used != 0) failed = 1;
    if (!failed && dunit_lua_log(STDOUT_FILENO, "[LUA] %s OK\n", name)) failed = 1;
    return failed;
}

int main(void)
{
    int failures = 0;
    failures += probe("language",
        "assert(6 * 7 == 42 and 7 // 2 == 3 and 2 ^ 10 == 1024) "
        "local t = {answer=42, 10, 20}; t[3] = t[1]+t[2] "
        "assert(t.answer == 42 and #t == 3 and t[3] == 30) "
        "local ok, err = pcall(function() error('probe-error') end) "
        "assert(not ok and type(err) == 'string') "
        "assert(os == nil and io == nil and package == nil and debug == nil) "
        "assert(loadfile == nil and dofile == nil and load == nil)",
        LUA_OK, 0);
    failures += probe("libraries",
        "assert(math.floor(3.75) == 3 and math.sin(0) == 0) "
        "local t = {}; table.insert(t, 42) "
        "assert(string.format('%d', t[1]) == '42') "
        "assert(math.random == nil and math.randomseed == nil) "
        "assert(os == nil and io == nil and package == nil and debug == nil "
        "and coroutine == nil)", LUA_OK, 0);
    failures += probe("instruction-budget", "while true do end",
                      LUA_ERRRUN, 1);
    /* Doubling strings exceeds the allocator budget well before the hook's
     * instruction quota; no string library or unbounded C helper is needed. */
    failures += probe("allocation-budget",
        "local s='0123456789abcdef'; while true do s=s..s end",
        LUA_ERRMEM, 0);
    if (failures) { dunit_lua_log(STDOUT_FILENO, "[LUA] FAIL compatibility\n"); return 1; }
    return dunit_lua_log(STDOUT_FILENO, "[LUA] compatibility OK\n") ? 1 : 0;
}
