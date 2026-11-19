#ifndef DUNIT_POSIX_SOCKETS_H
#define DUNIT_POSIX_SOCKETS_H

#include <stdint.h>
#include <sys/socket.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Select an existing process-local NET_CONNECT grant for future socket()
 * calls. Zero clears the selection. The caller retains ownership of the
 * grant and must keep it open while selected. Existing sockets own their
 * own descriptors.
 *
 * Link with -ldunit_posix_sockets and request SOCK_NONBLOCK with AF_INET.
 */
int dunit_socket_use_grant(uint32_t grant);

#ifdef __cplusplus
}
#endif

#endif
