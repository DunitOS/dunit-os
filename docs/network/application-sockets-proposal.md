# Application socket boundary for N4

The maintainer approved this outbound IPv4 boundary on 19 November 2026.
See [implementation and verification](application-sockets.md).

The current E1000 packet descriptor belongs to a trusted service and carries
raw Ethernet frames. Applications need a separate descriptor whose operations
address one logical UDP endpoint or TCP stream. This proposal keeps the TCP,
UDP, DNS and route policy in netd and keeps ordinary descriptor ownership,
wait and copy validation in the kernel.

## First implementation

Use owner-bound native descriptors backed by bounded requests to one registered
netd generation. Support explicit outbound IPv4 grants before adding listen,
low-port binding, global observability, or interface configuration.

The first implementation assigns syscalls 97–100 and rights bits 9–10:

- `NetServiceRegister(device_handle, options)` publishes a live service
  generation and returns a private service FD. Its reads deliver kernel-tagged
  requests and its writes submit bounded responses. Only the designated boot
  service receives `NET_SERVICE` on its device grant.
- `NetConnectGrant(root_handle, child_pid, scope)` issues a nontransferable
  `NET_CONNECT` capability to an explicitly authorized live direct child. The
  policy comes from the trusted boot catalog. Launching an executable by path
  carries no such authority.
- `NetSocketOpen(connect_handle, protocol, options)` creates a nonblocking
  owner-bound UDP or TCP descriptor. Kernel and service both enforce limits
  before allocating a slot.
- `NetSocketOp(fd, operation, input, output)` covers IPv4 connect, UDP
  send-to/receive-from, TCP write shutdown, and endpoint/error queries. Use
  fixed-width versioned records with explicit lengths. Native read, write,
  poll, wait-set, dup and close use the same owner-bound descriptor.

A TCP read returns partial bytes, EOF after buffered data, EAGAIN when it
would block, or a terminal reset error. One UDP receive consumes one datagram
and explicitly reports truncation. Readiness is level-triggered. A closed
or replaced netd generation wakes waiters with a terminal error. It cannot
resume old sockets. Same-owner dup shares the open description; spawn and
handle transfer leave the initial socket authority within its owner.

Bound per-process and global descriptors, bytes, pending requests and
connections in both layers. Netd validates the descriptor generation and
kernel-attested owner for each request. All internal request and response
buffers are copied or kernel-owned. A failed copyout leaves response data
available for retry. The application never receives device handles, packet
FDs, DMA frames or other process traffic.

## Acceptance and alternative

Start with a trusted test client along with netd. Verify UDP request/response,
TCP connect, partial I/O and EOF, wait-set readiness, descriptor exhaustion,
ungranted caller denial, spawn/transfer isolation, close while waiting and
service restart in QEMU. Then adapt Dunit musl using the same descriptors and
test that a native FD and a POSIX socket have matching readiness. Listener
rights, wider launcher policy, app DNS and public Internet qualification are
separate decisions.

A pure userspace IPC library would require fewer kernel changes and can offer a
Rust-specific API. Native FD ownership, mixed readiness and musl descriptors
would still need a bridge. The owner-bound proxy covers those roadmap
requirements in the first application-facing slice.
