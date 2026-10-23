#ifndef DUNIT_LUA_LIBRARIES_H
#define DUNIT_LUA_LIBRARIES_H

#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"

/* Pure computation libraries only. Upstream math initialization seeds its
 * internal PRNG with time/address data; the Dunit port supports time(), but
 * that seed is not trusted entropy. Exclude both PRNG entry points entirely. */
static void dunit_lua_open_pure_libraries(lua_State *L)
{
    luaL_requiref(L, LUA_MATHLIBNAME, luaopen_math, 1);
    lua_pushnil(L); lua_setfield(L, -2, "random");
    lua_pushnil(L); lua_setfield(L, -2, "randomseed");
    lua_pop(L, 1);
    luaL_requiref(L, LUA_STRLIBNAME, luaopen_string, 1);
    lua_pop(L, 1);
    luaL_requiref(L, LUA_TABLIBNAME, luaopen_table, 1);
    lua_pop(L, 1);
}

#endif
