/* Pure, bounded DWM SDK. No filesystem, process, or compositor handles. */
#ifndef DUNIT_DWM_LUA_SDK_H
#define DUNIT_DWM_LUA_SDK_H
#include <math.h>

#define SDK_ACTION_LIMIT 32
#define SDK_HANDLER_LIMIT 64
#define SDK_CURVE_SAMPLES 33
struct sdk_animation {
    unsigned window, target;
    int duration;
    const char *effect;
    int curve[SDK_CURVE_SAMPLES];
    const char *property;
    long long from, to;
};
struct sdk_widget { unsigned target; long long color; size_t length; unsigned char text[256]; };
struct sdk_request {
    const char *event;
    const char *identity;
    unsigned window, target, panel, subscriptions;
    int report_subscriptions;
    int count, command_count, widget_count;
    struct sdk_widget widgets[16];
    char commands[SDK_ACTION_LIMIT][129];
    struct sdk_animation actions[SDK_ACTION_LIMIT];
};
static const char *const sdk_events[] = {
    "window_open", "window_close", "window_focus", "window_minimize",
    "window_restore", "workspace_change", "panel_click", "reload",
    "window_maximize", "panel_hover", "launcher_open", "launcher_close",
    "widget_update", "widget_click", "theme_change"
};
static int sdk_event_index(const char *event)
{
    for (unsigned i = 0; i < sizeof sdk_events / sizeof sdk_events[0]; i++)
        if (!strcmp(event, sdk_events[i])) return (int)i;
    const char *const aliases[] = {"window::created", "window::closed", "window::focused", "window::minimized",
        "window::restored", "workspace::changed", "panel::clicked", "config::reloaded",
        "window::maximized", "panel::hover", "launcher::opened", "launcher::closed",
        "widget::updated", "widget::clicked", "theme::changed"};
    for (unsigned i = 0; i < sizeof aliases / sizeof aliases[0]; i++)
        if (!strcmp(event, aliases[i])) return (int)i;
    if (!strcmp(event, "window::focus")) return 2;
    if (!strcmp(event, "panel::click")) return 6;
    if (!strcmp(event, "config::reload")) return 7;
    return -1;
}
static struct sdk_request *sdk_context(lua_State *L)
{
    return lua_touserdata(L, lua_upvalueindex(1));
}
static int sdk_on(lua_State *L)
{
    size_t length;
    const char *name = luaL_checklstring(L, 1, &length);
    if (strlen(name) != length) return luaL_error(L, "embedded zero in event name");
    int event = sdk_event_index(name);
    if (event < 0) return luaL_error(L, "unsupported event");
    luaL_checktype(L, 2, LUA_TFUNCTION);
    lua_getfield(L, LUA_REGISTRYINDEX, "dwm.handlers");
    lua_rawgeti(L, -1, event + 1);
    size_t count = lua_rawlen(L, -1);
    if (count >= SDK_HANDLER_LIMIT) return luaL_error(L, "too many event handlers");
    lua_pushvalue(L, 2);
    lua_rawseti(L, -2, (lua_Integer)count + 1);
    return 0;
}
struct sdk_property {
    const char *name;
    double minimum, maximum;
    int fixed;
};
static const struct sdk_property sdk_properties[] = {
    {"opacity", 0, 1, 1}, {"x", -16384, 16384, 0}, {"y", -16384, 16384, 0},
    {"width", 1, 16384, 0}, {"height", 1, 16384, 0}, {"scale", 0, 16, 1},
    {"translation_x", -16384, 16384, 0}, {"translation_y", -16384, 16384, 0},
    {"blur", 0, 1024, 0}, {"shadow", 0, 1024, 0}, {"corner_radius", 0, 1024, 0},
    {"color", 0, 4294967295.0, 0}
};
static long long sdk_scalar(lua_State *L, int index, const char *key, const struct sdk_property *property)
{
    field(L, index, key);
    if (lua_type(L, -1) != LUA_TNUMBER) return luaL_error(L, "scalar must be a number");
    double value = lua_tonumber(L, -1);
    lua_pop(L, 1);
    if (!isfinite(value) || value < property->minimum || value > property->maximum)
        return luaL_error(L, "scalar is outside the property range");
    if (!property->fixed && value != floor(value))
        return luaL_error(L, "pixel/color values must be integers");
    return property->fixed ? (long long)(value * 65535 + 0.5) : (long long)value;
}
static int sdk_register(lua_State *L)
{
    size_t length;
    const char *name = luaL_checklstring(L, 1, &length);
    if (!length || length > 64 || strlen(name) != length) return luaL_error(L, "invalid curve name");
    luaL_checktype(L, 2, LUA_TFUNCTION);
    lua_getfield(L, LUA_REGISTRYINDEX, "dwm.curves");
    lua_getfield(L, -1, name);
    int exists = !lua_isnil(L, -1);
    lua_pop(L, 1);
    lua_getfield(L, LUA_REGISTRYINDEX, "dwm.curve_count");
    int count = (int)lua_tointeger(L, -1);
    lua_pop(L, 1);
    if (!exists && count >= 32) return luaL_error(L, "too many registered curves");
    lua_pushvalue(L, 2); lua_setfield(L, -2, name);
    lua_pushinteger(L, count + !exists); lua_setfield(L, LUA_REGISTRYINDEX, "dwm.curve_count");
    return 0;
}
static int sdk_animate(lua_State *L)
{
    struct sdk_request *r = sdk_context(L);
    if (r->count >= SDK_ACTION_LIMIT) return luaL_error(L, "too many animation actions");
    luaL_checktype(L, 1, LUA_TTABLE);
    int record = lua_istable(L, 2) ? 2 : 1;
    const char *const keys[] = {"window", "target", "effect", "duration_ms", "easing", "property", "from", "to", "duration"};
    exact_keys(L, record, keys, sizeof keys / sizeof keys[0]);
    struct sdk_animation a = {0};
    a.property = "none";
    a.window = (unsigned)integer_field(L, 1, record == 2 ? "id" : "window", 0, 2147483647);
    field(L, record, "target");
    if (lua_isnil(L, -1)) a.target = r->target;
    else {
        lua_pop(L, 1);
        a.target = (unsigned)integer_field(L, record, "target", 0, 2147483647);
        lua_pushnil(L);
    }
    lua_pop(L, 1);
    field(L, record, "duration");
    int duration_alias = !lua_isnil(L, -1);
    lua_pop(L, 1);
    a.duration = integer_field(L, record, duration_alias ? "duration" : "duration_ms", 0, 2000);
    field(L, record, "property");
    if (!lua_isnil(L, -1)) {
        size_t length;
        const char *property = luaL_checklstring(L, -1, &length);
        const struct sdk_property *selected = NULL;
        for (unsigned i = 0; i < sizeof sdk_properties / sizeof sdk_properties[0]; i++)
            if (strlen(sdk_properties[i].name) == length && !memcmp(property, sdk_properties[i].name, length))
                selected = &sdk_properties[i];
        if (!selected) return luaL_error(L, "unsupported animation property");
        a.property = selected->name;
        a.from = sdk_scalar(L, record, "from", selected);
        a.to = sdk_scalar(L, record, "to", selected);
        a.effect = "fade";
    }
    lua_pop(L, 1);
    field(L, record, "effect");
    if (!lua_isnil(L, -1)) {
        size_t length;
        const char *effect = luaL_checklstring(L, -1, &length);
        a.effect = NULL;
        const char *const effects[] = {"fade", "slide", "scale", "spring", "genie"};
        for (unsigned i = 0; i < sizeof effects / sizeof effects[0]; i++)
            if (strlen(effects[i]) == length && !memcmp(effect, effects[i], length)) a.effect = effects[i];
    }
    if (!a.effect) return luaL_error(L, "unsupported animation effect");
    lua_pop(L, 1);
    if (strcmp(a.property, "none") && strcmp(a.effect, "fade"))
        return luaL_error(L, "scalar properties require fade effect");
    if (!a.window && (!strcmp(a.property, "none") || a.target < 1 || a.target > 2048))
        return luaL_error(L, "component animations require a scalar property and a component target");
    field(L, record, "easing");
    int function = lua_isfunction(L, -1);
    const char *easing = function ? NULL : luaL_optstring(L, -1, "ease_out");
    if (!function) {
        if (!lua_isnil(L, -1) && lua_rawlen(L, -1) != strlen(easing))
            return luaL_error(L, "embedded zero in curve name");
        lua_getfield(L, LUA_REGISTRYINDEX, "dwm.curves");
        lua_getfield(L, -1, easing);
        lua_remove(L, -2);
        if (lua_isfunction(L, -1)) { lua_remove(L, -2); function = 1; }
        else lua_pop(L, 1);
    }
    if (!function && strcmp(easing, "linear") && strcmp(easing, "ease_out"))
        return luaL_error(L, "unsupported easing");
    for (unsigned i = 0; i < SDK_CURVE_SAMPLES; i++) {
        double t = (double)i / (SDK_CURVE_SAMPLES - 1), value;
        if (function) {
            lua_pushvalue(L, -1);
            lua_pushnumber(L, t);
            lua_call(L, 1, 1);
            if (lua_type(L, -1) != LUA_TNUMBER) return luaL_error(L, "curve must return a number");
            value = lua_tonumber(L, -1);
            lua_pop(L, 1);
        } else value = !strcmp(easing, "linear") ? t : 1 - (1-t)*(1-t)*(1-t);
        if (!isfinite(value) || value < -1 || value > 2)
            return luaL_error(L, "curve samples must be finite within [-1,2]");
        a.curve[i] = (int)(value * 65535 + (value < 0 ? -0.5 : 0.5));
    }
    /* Damped springs approach their endpoint asymptotically. Pin only small
     * endpoint residuals (one percent); reject unrelated curves. */
    if(abs(a.curve[0])>655||abs(a.curve[SDK_CURVE_SAMPLES-1]-65535)>655)
        return luaL_error(L,"curve endpoints must approach 0 and 1 within one percent");
    a.curve[0]=0;a.curve[SDK_CURVE_SAMPLES-1]=65535;
    lua_pop(L, 1);
    r->actions[r->count++] = a;
    return 0;
}
static int sdk_command(lua_State *L)
{
    struct sdk_request *r = sdk_context(L);
    if (r->command_count >= SDK_ACTION_LIMIT) return luaL_error(L, "too many commands");
    size_t length;
    const char *action = luaL_checklstring(L, 1, &length);
    if (!length || length > 128 || strlen(action) != length) return luaL_error(L, "invalid command token");
    int valid = !strcmp(action, "launcher") || !strcmp(action, "close")
        || !strcmp(action, "maximize") || !strcmp(action, "minimize") || !strcmp(action, "retile");
    if (length > 7 && !memcmp(action, "launch:", 7)) {
        valid = 1;
        for (size_t i = 7; i < length; i++)
            if (!((action[i] >= 'a' && action[i] <= 'z') || (action[i] >= 'A' && action[i] <= 'Z')
                || (action[i] >= '0' && action[i] <= '9') || action[i] == '_' || action[i] == '-')) valid = 0;
    }
    if (length > 10 && !memcmp(action, "workspace:", 10)) {
        valid = 1;
        for (size_t i = 10; i < length; i++) if (action[i] < '0' || action[i] > '9') valid = 0;
    }
    if (!valid) return luaL_error(L, "command is outside the desktop action allowlist");
    memcpy(r->commands[r->command_count], action, length + 1);
    r->command_count++;
    return 0;
}
static int sdk_utf8(const unsigned char *text, size_t length)
{
    for (size_t i = 0; i < length;) {
        unsigned c = text[i++], value, minimum, remaining;
        if (!c) return 0;
        if (c < 128) continue;
        if (c >= 0xc2 && c <= 0xdf) { value = c & 31; remaining = 1; minimum = 0x80; }
        else if (c >= 0xe0 && c <= 0xef) { value = c & 15; remaining = 2; minimum = 0x800; }
        else if (c >= 0xf0 && c <= 0xf4) { value = c & 7; remaining = 3; minimum = 0x10000; }
        else return 0;
        if (remaining > length - i) return 0;
        while (remaining--) {
            unsigned next = text[i++];
            if ((next & 0xc0) != 0x80) return 0;
            value = (value << 6) | (next & 63);
        }
        if (value < minimum || value > 0x10ffff || (value >= 0xd800 && value <= 0xdfff)) return 0;
    }
    return 1;
}
static int sdk_widget_set(lua_State *L)
{
    struct sdk_request *r = sdk_context(L);
    if (r->widget_count >= 16) return luaL_error(L, "too many widget updates");
    lua_Integer target = luaL_checkinteger(L, 1);
    if (target < 1 || target > 512) return luaL_error(L, "widget target outside component range");
    luaL_checktype(L, 2, LUA_TTABLE);
    const char *const keys[] = {"text", "color"};
    exact_keys(L, 2, keys, 2);
    struct sdk_widget update = { .target = (unsigned)target, .color = -1 };
    field(L, 2, "text");
    if (lua_type(L, -1) != LUA_TSTRING) return luaL_error(L, "widget text must be a string");
    const char *text = lua_tolstring(L, -1, &update.length);
    if (update.length > sizeof update.text || !sdk_utf8((const unsigned char *)text, update.length))
        return luaL_error(L, "widget text must be valid UTF8 without NUL and at most 256 bytes");
    memcpy(update.text, text, update.length);
    lua_pop(L, 1);
    field(L, 2, "color");
    if (!lua_isnil(L, -1)) {
        if (!lua_isinteger(L, -1)) return luaL_error(L, "widget color must be a packed integer");
        lua_Integer color = lua_tointeger(L, -1);
        if (color < 0 || color > 4294967295LL) return luaL_error(L, "widget color outside packed range");
        update.color = (long long)color;
    }
    lua_pop(L, 1);
    r->widgets[r->widget_count++] = update;
    return 0;
}
static int sdk_require(lua_State *L)
{
    size_t length;
    const char *name = luaL_checklstring(L, 1, &length);
    if (length == 3 && !memcmp(name, "dwm", 3)) lua_getfield(L, LUA_REGISTRYINDEX, "dwm.module");
    else if (length == 9 && !memcmp(name, "animation", 9)) lua_getfield(L, LUA_REGISTRYINDEX, "dwm.animation");
    else return luaL_error(L, "module is unavailable");
    return 1;
}
static void sdk_open(lua_State *L, struct sdk_request *r)
{
    lua_newtable(L); lua_setfield(L, LUA_REGISTRYINDEX, "dwm.curves");
    lua_newtable(L);
    lua_pushcfunction(L, sdk_register); lua_setfield(L, -2, "register");
    lua_pushvalue(L, -1); lua_setglobal(L, "animation");
    lua_setfield(L, LUA_REGISTRYINDEX, "dwm.animation");
    const int events = (int)(sizeof sdk_events / sizeof sdk_events[0]);
    lua_createtable(L, events, 0);
    for (int i = 0; i < events; i++) {
        lua_newtable(L);
        lua_rawseti(L, -2, i + 1);
    }
    lua_setfield(L, LUA_REGISTRYINDEX, "dwm.handlers");
    lua_newtable(L);
    lua_getfield(L, LUA_REGISTRYINDEX, "dwm.animation"); lua_setfield(L, -2, "animation");
    lua_newtable(L);
    lua_pushlightuserdata(L, r);
    lua_pushcclosure(L, sdk_widget_set, 1); lua_setfield(L, -2, "set");
    lua_setfield(L, -2, "widget");
    lua_newtable(L);lua_pushinteger(L,513);lua_setfield(L,-2,"launcher");lua_pushinteger(L,514);lua_setfield(L,-2,"workspace");lua_pushinteger(L,515);lua_setfield(L,-2,"notifications");lua_setfield(L,-2,"targets");
    lua_pushinteger(L, 2); lua_setfield(L, -2, "api_version");
    lua_pushcfunction(L, sdk_on); lua_setfield(L, -2, "on");
    lua_pushlightuserdata(L, r);
    lua_pushcclosure(L, sdk_animate, 1); lua_setfield(L, -2, "animate");
    lua_pushlightuserdata(L, r);
    lua_pushcclosure(L, sdk_command, 1); lua_setfield(L, -2, "command");
    lua_setfield(L, LUA_REGISTRYINDEX, "dwm.module");
    lua_pushcfunction(L, sdk_require); lua_setglobal(L, "require");
}
static void sdk_dispatch(lua_State *L, struct sdk_request *r)
{
    int event = sdk_event_index(r->event);
    lua_getfield(L, LUA_REGISTRYINDEX, "dwm.handlers");
    lua_rawgeti(L, -1, event + 1);
    size_t count = lua_rawlen(L, -1);
    for (size_t i = 1; i <= count; i++) {
        lua_rawgeti(L, -1, (lua_Integer)i);
        lua_createtable(L, 0, 3);
        lua_pushinteger(L, r->window); lua_setfield(L, -2, "id");
        lua_pushstring(L,r->event);lua_setfield(L,-2,"name");
        if(r->identity){lua_pushstring(L,r->identity);lua_setfield(L,-2,r->window?"app":"component");}
        if(r->panel){lua_pushinteger(L,r->panel);lua_setfield(L,-2,"panel");}
        lua_createtable(L, 0, 1);
        lua_pushinteger(L, r->window); lua_setfield(L, -2, "id");
        lua_setfield(L, -2, "window");
        lua_pushinteger(L, r->target); lua_setfield(L, -2, "target");
        lua_call(L, 1, 0);
    }
    lua_pop(L,2);
    r->subscriptions=0;
    lua_getfield(L,LUA_REGISTRYINDEX,"dwm.handlers");
    for(unsigned i=0;i<sizeof sdk_events/sizeof sdk_events[0];i++){lua_rawgeti(L,-1,i+1);if(lua_rawlen(L,-1))r->subscriptions|=1u<<i;lua_pop(L,1);}
    lua_pop(L,1);
}
#endif
