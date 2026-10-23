/* Dunit Lua installation defaults. Runtime LUA_PATH_5_5/LUA_PATH and
 * LUA_CPATH_5_5/LUA_CPATH override these through upstream Lua's normal API.
 * No dynamic-library loader is enabled on a static-only Dunit target. */
#ifndef DUNIT_LUA_CONFIG_H
#define DUNIT_LUA_CONFIG_H
#define LUA_PATH_DEFAULT "/system/share/lua/5.5/?.lua;/system/share/lua/5.5/?/init.lua;./?.lua;./?/init.lua"
#define LUA_CPATH_DEFAULT ""
#endif
