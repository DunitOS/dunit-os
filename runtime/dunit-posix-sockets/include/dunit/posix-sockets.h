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
 * Link with -ldunit_posix_sockets and call with AF_INET.
 */
int dunit_socket_use_grant(uint32_t grant);

/*
 * Toggle userspace blocking behavior for an existing socket FD created via
 * socket(). Kernel descriptors remain nonblocking; this controls whether
 * connect/send/recv wait via poll() and SO_SNDTIMEO/SO_RCVTIMEO.
 */
int dunit_socket_set_nonblocking(int fd, int nonblocking);

#ifdef __cplusplus
}
#endif

#endif
