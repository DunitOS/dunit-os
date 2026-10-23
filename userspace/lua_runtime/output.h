#ifndef DUNIT_LUA_OUTPUT_H
#define DUNIT_LUA_OUTPUT_H

#include <errno.h>
#include <stdarg.h>
#include <stdio.h>
#include <unistd.h>

/* The terminal serial sink prefixes each Write, so format a complete record
 * before issuing it. A short write is an output failure, never success. */
static inline int dunit_lua_write(int fd, const char *data, size_t size)
{
    ssize_t n;
    do { n = write(fd, data, size); } while (n < 0 && errno == EINTR);
    return n >= 0 && (size_t)n == size ? 0 : -1;
}

static inline int dunit_lua_log(int fd, const char *format, ...)
{
    char record[1024];
    va_list ap;
    va_start(ap, format);
    int n = vsnprintf(record, sizeof record, format, ap);
    va_end(ap);
    if (n < 0 || (size_t)n >= sizeof record) return -1;
    return dunit_lua_write(fd, record, (size_t)n);
}

#endif
