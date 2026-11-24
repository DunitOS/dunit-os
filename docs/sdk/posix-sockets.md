# Opt-in POSIX IPv4 sockets for Dunit musl

Link a static musl program with `-ldunit_posix_sockets` and include
`<dunit/posix-sockets.h>`. The prepared Dunit sysroot includes the archive
and header. The DWM toolchain image installs the archive under `/system/lib`.
The archive supplies standard C socket symbols for programs that opt in.

A client selects a process-local `NET_CONNECT` grant using
`dunit_socket_use_grant(handle)`. The trusted boot catalog and supervisor
issue grants for UDP, TCP, or both. The application keeps the selected grant
open. Passing zero clears the selection for future sockets.

The adapter supports `AF_INET` with blocking and nonblocking (`SOCK_NONBLOCK`)
modes plus optional `SOCK_CLOEXEC`. Socket creation waits up to five seconds for
acknowledgment from netd, so the first `connect` can submit the connection
request. On a nonblocking socket, `connect` returning `EINPROGRESS` completes
through ordinary `poll(POLLOUT)` and `getsockopt(SOL_SOCKET, SO_ERROR)`. On a
blocking socket, `connect`, `send`, `sendto`, `recv`, `recvfrom` and `shutdown`
wait through `poll` until completion or timeout (`SO_SNDTIMEO` / `SO_RCVTIMEO`).
`dunit_socket_set_nonblocking(fd, nonblocking)` switches userspace blocking mode
on an open socket, and `MSG_DONTWAIT` forces nonblocking behavior per call.

TCP `send` and `recv` use the same partial-I/O and EOF behavior as native
read/write (blocking `send` loops until all bytes are accepted or an error/timeout
occurs). TCP supports `shutdown(SHUT_WR)`. UDP `sendto` and `recvfrom`
preserve datagram boundaries, and `MSG_TRUNC` returns the original datagram
length. `getsockopt` supports `SO_ERROR`, `SO_RCVTIMEO`, `SO_SNDTIMEO`,
`SO_TYPE`, `SO_DOMAIN` and `SO_PROTOCOL`; `setsockopt` supports `SO_RCVTIMEO`
and `SO_SNDTIMEO`. `getsockname` and `getpeername` translate native endpoint
records to `sockaddr_in`. POSIX `poll`, `dup`, `fcntl` and `close` operate on
ordinary native descriptors. Revocation wakes pollers and old socket calls
report EPIPE. A terminal asynchronous error stays associated with the
descriptor for its lifetime.

The adapter requires valid mapped C buffers for structures and
datagrams it packs locally. Native operations retain their separately
verified copyout retry contract. Listener operations,
IPv6, resolver functions and additional socket options remain open.

The C fixture uses the existing E1000 wire peer and the stock netd service.
It checks ungranted creation, process isolation, a mixed pipe/socket poll,
UDP truncation, TCP nonblocking connect, partial I/O, EOF and bridge
withdrawal. See [native verification](../verification/posix-sockets-2026-11-19.json).
