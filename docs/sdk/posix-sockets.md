# Opt-in POSIX IPv4 sockets for Dunit musl

Link a static musl program with `-ldunit_posix_sockets` and include
`<dunit/posix-sockets.h>`. The prepared Dunit sysroot includes the archive
and header. The DWM toolchain image installs the archive under `/system/lib`.
The archive supplies standard C socket symbols for programs that opt in.

A client selects a process-local `NET_CONNECT` grant using
`dunit_socket_use_grant(handle)`. The trusted boot catalog and supervisor
issue grants for UDP, TCP, or both. The application keeps the selected grant
open. Passing zero clears the selection for future sockets.

The adapter supports `AF_INET` with `SOCK_NONBLOCK` and optional
`SOCK_CLOEXEC`. Socket creation waits up to five seconds for acknowledgment
from netd, so the first `connect` can submit the connection request.
A connect returning EINPROGRESS completes through ordinary
`poll(POLLOUT)` and `getsockopt(SOL_SOCKET, SO_ERROR)`.

TCP `send` and `recv` use the same partial-I/O and EOF behavior as native
read/write. TCP supports `shutdown(SHUT_WR)`. UDP `sendto` and `recvfrom`
preserve datagram boundaries, and `MSG_TRUNC` returns the original datagram
length. `getsockname` and `getpeername` translate native endpoint records
to `sockaddr_in`. POSIX `poll`, `dup`, `fcntl` and `close` operate on
ordinary native descriptors. Revocation wakes pollers and old socket calls
report EPIPE. A terminal asynchronous error stays associated with the
descriptor for its lifetime.

The first adapter requires valid mapped C buffers for structures and
datagrams it packs locally. Native operations retain their separately
verified copyout retry contract. Blocking socket mode, listener operations,
IPv6, resolver functions and additional socket options remain open.

The C fixture uses the existing E1000 wire peer and the stock netd service.
It checks ungranted creation, process isolation, a mixed pipe/socket poll,
UDP truncation, TCP nonblocking connect, partial I/O, EOF and bridge
withdrawal. See [native verification](../verification/posix-sockets-2026-11-19.json).
