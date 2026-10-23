/* One-shot layout policy evaluator. No compositor, IPC or syscall bindings. */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"
#include "../../lua_runtime/limits.h"
#include "../../lua_runtime/output.h"
#include "../../lua_runtime/libraries.h"

#define SOURCE_LIMIT (64 * 1024)
#define WINDOW_LIMIT 64
#define VIEWPORT_LIMIT 16384

struct rectangle { int x, y, w, h; };
struct request {
    const char *source;
    size_t source_len;
    int width, height, count;
    struct rectangle rectangles[WINDOW_LIMIT];
    int duration_ms;
    const char *easing;
};

static void field(lua_State *L, int index, const char *key)
{
    index = lua_absindex(L, index);
    lua_pushstring(L, key);
    lua_rawget(L, index);
}

static int integer_field(lua_State *L, int index, const char *key, int lo, int hi)
{
    field(L, index, key);
    if (!lua_isinteger(L, -1))
        return luaL_error(L, "field '%s' must be an integer", key);
    lua_Integer v = lua_tointeger(L, -1);
    lua_pop(L, 1);
    if (v < lo || v > hi)
        return luaL_error(L, "field '%s' is outside its allowed range", key);
    return (int)v;
}

static void exact_keys(lua_State *L, int index, const char *const *keys, size_t n)
{
    index = lua_absindex(L, index);
    lua_pushnil(L);
    while (lua_next(L, index)) {
        if (lua_type(L, -2) != LUA_TSTRING)
            luaL_error(L, "record keys must be strings");
        size_t length;
        const char *key = lua_tolstring(L, -2, &length);
        size_t i;
        for (i = 0; i < n; i++)
            if (strlen(keys[i]) == length && !memcmp(key, keys[i], length)) break;
        if (i == n) luaL_error(L, "unknown record field");
        lua_pop(L, 1);
    }
}

static void require_table(lua_State *L)
{
    if (lua_type(L, -1) != LUA_TTABLE) luaL_error(L, "expected a table");
}

static void validate(lua_State *L, struct request *r)
{
    const char *const root_keys[] = {"rectangles", "animation"};
    const char *const rect_keys[] = {"x", "y", "w", "h"};
    const char *const anim_keys[] = {"duration_ms", "easing"};
    require_table(L);
    exact_keys(L, -1, root_keys, 2);
    field(L, -1, "rectangles");
    require_table(L);
    /* Validate keys as well as raw length: holes, extra IDs and map entries
     * must not silently disappear from the policy snapshot. */
    int entries = 0;
    lua_pushnil(L);
    while (lua_next(L, -2)) {
        if (!lua_isinteger(L, -2) || lua_tointeger(L, -2) < 1
            || lua_tointeger(L, -2) > r->count)
            luaL_error(L, "rectangle indices must be contiguous window indices");
        entries++;
        lua_pop(L, 1);
    }
    if (entries != r->count) luaL_error(L, "rectangle count does not match windows");
    for (int i = 0; i < r->count; i++) {
        lua_rawgeti(L, -1, i + 1);
        require_table(L);
        exact_keys(L, -1, rect_keys, 4);
        struct rectangle *p = &r->rectangles[i];
        p->x = integer_field(L, -1, "x", 0, r->width - 1);
        p->y = integer_field(L, -1, "y", 0, r->height - 1);
        p->w = integer_field(L, -1, "w", 1, r->width);
        p->h = integer_field(L, -1, "h", 1, r->height);
        if (p->w > r->width - p->x || p->h > r->height - p->y)
            luaL_error(L, "rectangle extends beyond work area");
        lua_pop(L, 1);
    }
    lua_pop(L, 1);
    field(L, -1, "animation");
    require_table(L);
    exact_keys(L, -1, anim_keys, 2);
    r->duration_ms = integer_field(L, -1, "duration_ms", 0, 2000);
    field(L, -1, "easing");
    if (lua_type(L, -1) != LUA_TSTRING) luaL_error(L, "easing must be a string");
    size_t length;
    const char *easing = lua_tolstring(L, -1, &length);
    if (length == 6 && !memcmp(easing, "linear", 6)) r->easing = "linear";
    else if (length == 8 && !memcmp(easing, "ease_out", 8)) r->easing = "ease_out";
    else luaL_error(L, "unsupported easing");
    lua_pop(L, 2);
}

/* Everything which can allocate or raise a Lua error executes under pcall. */
static int evaluate(lua_State *L)
{
    struct request *r = lua_touserdata(L, 1);
    luaL_requiref(L, LUA_GNAME, luaopen_base, 1);
    lua_pop(L, 1);
    dunit_lua_open_pure_libraries(L);
    const char *removed[] = {
        "print", "warn", "dofile", "loadfile", "load", "collectgarbage",
        "pcall", "xpcall", "getmetatable", "setmetatable", "rawget", "rawset",
        "rawequal", "rawlen"
    };
    for (size_t i = 0; i < sizeof removed / sizeof removed[0]; i++) {
        lua_pushnil(L);
        lua_setglobal(L, removed[i]);
    }
    lua_createtable(L, 0, 3);
    lua_pushinteger(L, r->width); lua_setfield(L, -2, "width");
    lua_pushinteger(L, r->height); lua_setfield(L, -2, "height");
    lua_pushinteger(L, r->count); lua_setfield(L, -2, "count");
    lua_setglobal(L, "workarea");
    if (luaL_loadbufferx(L, r->source, r->source_len, "dwm-policy", "t") != LUA_OK)
        return lua_error(L);
    lua_call(L, 0, 1);
    validate(L, r);
    return 0;
}

static int number(const char *text, int minimum, int maximum, int *out)
{
    char *end;
    if (!text) return 0;
    errno = 0;
    long n = strtol(text, &end, 10);
    if (errno || end == text || *end || n < minimum || n > maximum) return 0;
    *out = (int)n;
    return 1;
}

int main(int argc, char **argv)
{
    struct request r = {0};
    const char *path = argc == 5 ? argv[1] : getenv("DWM_LUA_SCRIPT");
    const char *width = argc == 5 ? argv[2] : getenv("DWM_LUA_WIDTH");
    const char *height = argc == 5 ? argv[3] : getenv("DWM_LUA_HEIGHT");
    const char *count = argc == 5 ? argv[4] : getenv("DWM_LUA_WINDOWS");
    if ((argc != 1 && argc != 5) || !path || !*path
        || !number(width, 1, VIEWPORT_LIMIT, &r.width)
        || !number(height, 1, VIEWPORT_LIMIT, &r.height)
        || !number(count, 0, WINDOW_LIMIT, &r.count)) {
        dunit_lua_log(STDERR_FILENO, "usage: dwm_lua script.lua work_width work_height window_count\n");
        return 2;
    }
    FILE *file = fopen(path, "rb");
    if (!file) { dunit_lua_log(STDERR_FILENO, "dwm_lua: cannot open script\n"); return 2; }
    char *source = malloc(SOURCE_LIMIT + 1);
    if (!source) { fclose(file); return 2; }
    size_t length = 0;
    /* Keep every native Read below MAX_USER_COPY. Read one extra byte only
     * after the limit is filled so oversize scripts are still rejected. */
    while (length < SOURCE_LIMIT + 1) {
        size_t chunk = SOURCE_LIMIT + 1 - length;
        if (chunk > 4096) chunk = 4096;
        size_t got = fread(source + length, 1, chunk, file);
        length += got;
        if (got < chunk) break;
    }
    int read_failed = ferror(file);
    fclose(file);
    if (read_failed || length > SOURCE_LIMIT) {
        dunit_lua_log(STDERR_FILENO, "dwm_lua: source read failed or exceeds 64 KiB\n");
        free(source);
        return 2;
    }
    r.source = source;
    r.source_len = length;
    struct dunit_lua_limits limits = { .bytes = 1024 * 1024, .instructions = 100000 };
    /* This CLI processes locally trusted policy only. A production parent must
     * supply hard process limits and a suitable seed before accepting hostile
     * input; count hooks cannot interrupt compilation or arbitrary C calls. */
    lua_State *L = lua_newstate(dunit_lua_alloc, &limits, 0x44574d35u);
    if (!L) { free(source); return 1; }
    *(struct dunit_lua_limits **)lua_getextraspace(L) = &limits;
    lua_pushcfunction(L, evaluate);
    lua_pushlightuserdata(L, &r);
    lua_sethook(L, dunit_lua_hook, LUA_MASKCOUNT, 1000);
    int status = lua_pcall(L, 1, 0, 0);
    lua_sethook(L, NULL, 0, 0);
    if (status != LUA_OK) {
        const char *message = lua_type(L, -1) == LUA_TSTRING
            ? lua_tostring(L, -1) : "non-string Lua error";
        dunit_lua_log(STDERR_FILENO, "dwm_lua: %s\n", message);
    }
    lua_close(L);
    free(source);
    if (status != LUA_OK || limits.used) return 1;
    /* Nothing reaches stdout before the complete result has passed validation.
     * A consumer must buffer through END and check exit status before applying. */
    char response[4096];
    int n = snprintf(response, sizeof response, "DWM_LUA 1 %d %d %d\n",
                     r.width, r.height, r.count);
    if (n < 0 || (size_t)n >= sizeof response) return 1;
    size_t used = (size_t)n;
    for (int i = 0; i < r.count; i++) {
        const struct rectangle *p = &r.rectangles[i];
        n = snprintf(response + used, sizeof response - used,
                     "RECT %d %d %d %d %d\n", i + 1, p->x, p->y, p->w, p->h);
        if (n < 0 || (size_t)n >= sizeof response - used) return 1;
        used += (size_t)n;
    }
    n = snprintf(response + used, sizeof response - used,
                 "ANIMATION %d %s\nEND\n", r.duration_ms, r.easing);
    if (n < 0 || (size_t)n >= sizeof response - used) return 1;
    used += (size_t)n;
    return dunit_lua_write(STDOUT_FILENO, response, used) ? 1 : 0;
}
