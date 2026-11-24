# Owner-bound application sockets

The first N4 slice implements outbound IPv4 TCP and UDP with native
descriptors, followed by DNS (`PROTO_DNS`) and listener authority. The trusted
boot catalog selects the grant recipients using `net_connect` scope bits (`1`
UDP outbound, `2` TCP outbound, `4` UDP bind, `8` TCP listen/accept, `16` low
ports `< 1024`).
The catalog marks the stock network service with `net_service = true` on
its device grant. The kernel checks the actual grant object and process
identity. The service remains responsible for Ethernet, routes and transport.

Use the [native ABI](../../abi/README.md#outbound-ipv4-application-sockets) and low-level
`libdunit` wrappers. Connect submits an operation. An EAGAIN result means
wait for write readiness and retry the same endpoint. TCP write accepts a
bounded prefix and may also return EAGAIN. TCP read yields queued bytes and
then EOF after peer FIN. `RECV_FROM` returns one UDP datagram and reports
its original length in the header. A failed copyout leaves the datagram
available for retry. Endpoint and error queries use fixed-width records.

Kernel and service cap the active registry at eight sockets, with at most
four per process in the kernel. The bridge carries one pending operation
and one receive record per socket, each at most 4 KiB. A credit message lets
the service deliver the next receive record after consumption. Duplicate
descriptors within a process share the socket. The grant and descriptors
stay with their owner across child launch and handle transfer.

When netd retires its private bridge or exits, the kernel revokes that
generation and wakes poll and wait-set. Old descriptors report HANGUP/ERROR
and operations fail with EPIPE. Netd retires its bridge when its active IPv4
configuration disappears or changes. A subsequent configuration uses a
fresh service generation with fresh descriptors.

An established TCP socket challenges an in-window RST with the wrong
sequence and only accepts an exact sequence. A valid reset wakes socket
waiters with HANGUP/ERROR and reports EPIPE through the current native
errno contract. Netd releases the failed TCP transport and discards
its frames still waiting for egress. The application descriptor retains
its error until closed. [Reset verification](../verification/tcp-reset-2026-11-20.json)

The QEMU suite connects an explicitly granted boot client to a strict local
E1000 peer. It checks 5,000 TCP bytes, partial writes and reads, FIN and EOF,
UDP truncation, mapped read-only copyout rejection, and child isolation.
The peer then creates an IPv4 conflict to verify bridge withdrawal while
the client waits on an idle socket. The kernel host tests separately exercise
registration replacement, stale tokens and quotas.

```sh
python3 tools/qemu_test.py --build --markers-file tools/net_socket_markers.json \
  --cpus 8 --accel tcg --timeout 260 --boot-timeout 120
```

The [opt-in C adapter](../sdk/posix-sockets.md) translates standard nonblocking
and blocking IPv4 calls, `bind`/`listen`/`accept`/`accept4`, and `getaddrinfo`
DNS queries for statically linked musl applications. Native `dunit::net` also
provides `TcpListener`, `UdpSocket::bind`, `resolve_ipv4` and
`TcpStream::connect_host`. IPv6, wider launcher policy and Internet qualification
remain open.
See [verification](../verification/native-sockets-2026-11-19.json).
