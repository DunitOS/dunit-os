#ifndef DUNIT_LUA_LIMITS_H
#define DUNIT_LUA_LIMITS_H

#include <stddef.h>
#include <stdlib.h>
#include "lua.h"
#include "lauxlib.h"

/* Limits Lua-owned allocation requests, including our alignment header.
 * Native stack and malloc metadata remain process-level resources. */
struct dunit_lua_limits {
    size_t used, peak, bytes;
    unsigned instructions;
    int quota_hit;
};

union dunit_lua_allocation {
    max_align_t alignment;
    size_t bytes;
};

static void *dunit_lua_alloc(void *ud, void *ptr, size_t osize, size_t nsize)
{
    struct dunit_lua_limits *b = ud;
    union dunit_lua_allocation *old = ptr
        ? (union dunit_lua_allocation *)ptr - 1 : NULL;
    size_t previous = old ? old->bytes : 0;
    (void)osize;
    if (!nsize) {
        b->used -= previous;
        free(old);
        return NULL;
    }
    if (nsize > (size_t)-1 - sizeof *old) return NULL;
    size_t bytes = nsize + sizeof *old;
    if (bytes > b->bytes || b->used - previous > b->bytes - bytes)
        return NULL;
    union dunit_lua_allocation *p = realloc(old, bytes);
    /* Lua requires shrinking allocations to succeed. Keep old physical size
     * accounted for if the host allocator refuses a shrink. */
    if (!p) return old && bytes <= previous ? ptr : NULL;
    p->bytes = bytes;
    b->used = b->used - previous + bytes;
    if (b->peak < b->used) b->peak = b->used;
    return p + 1;
}

static void dunit_lua_hook(lua_State *L, lua_Debug *ar)
{
    struct dunit_lua_limits *b = *(struct dunit_lua_limits **)lua_getextraspace(L);
    (void)ar;
    if (b->instructions <= 1000) {
        b->quota_hit = 1;
        luaL_error(L, "instruction quota exceeded");
    }
    b->instructions -= 1000;
}

#endif
