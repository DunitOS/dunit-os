# Packet authority v1 proposal — human decision required

Status: proposed, not implemented or authorized by this document. No syscall,
rights manifest, bootstrap privilege or public packet contract changes accompany
this proposal. The current userspace UDP/DNS core still has no NIC transport.

## Recommended first implementation

Use bounded copied packet batches backed by the kernel's private E1000 DMA rings.
Keep Ethernet/IP/UDP/DNS policy in a userspace `netd` service. Ordinary applications
use a later service socket protocol and never receive device or raw queue access.
This first path avoids exposing DMA memory, physical addresses or writable device
descriptors. Shared CPU rings are a later measured optimization, not a second
concurrent packet API.

The initial device is QEMU 82540EM with legacy INTx. Other discovery-only devices
return ENOTSUP; this does not claim MSI, hardware recovery or general NIC support.

## Authority and bootstrap

The kernel grants one nontransferable NET_ROOT handle to the initial userspace
supervisor selected by the existing boot mechanism. Pass its numeric value in a
new optional loader environment variable, using the existing environment format.
The kernel grants by bootstrap role, never by application name, path or PID literal.
Forging the environment variable grants no authority: every operation checks the
caller's actual handle table and object identity.

`init` may grant a device session to a selected live child service. Extend its
service manifest with an explicit network-device grant field; absent means none.
The supplied default configuration grants it only to `netd`. A grant is tied to
the recipient process generation, with rollback if insertion or delivery fails.
The recipient gets no TRANSFER right. NET_ROOT itself cannot be delegated.
Grant policy must come from the trusted boot service catalog. Writable user
desktop profiles and ordinary application manifests cannot request this grant.

Add NET_CONTROL and NET_RAW rights to the authoritative rights manifest, with
numbers assigned only after approval and a fresh ABI fetch. Existing READ/WRITE
attenuate device queries and RX/TX access. NET_CONTROL permits link/reset control;
NET_RAW permits opening the raw Ethernet queue. Kernel-private object kinds are
NET_ROOT and NET_DEVICE; a queue is an owner-bound native file description.

## Native operations

Two new syscall contracts, numbers assigned only after approval:

- `NetDeviceGrant(root_handle, interface_index, recipient_pid, rights)` returns
  the device-handle number inserted into that recipient. Validate recipient
  generation and rights before publication; no DMA starts merely by granting.
- `NetQueueOpen(device_handle, options_ptr, options_len)` returns a packet FD
  for the calling authorized process. Options are fixed-width, versioned, bounded,
  reserved-zero and copied/validated before device state changes.

NET_DEVICE HandleRead returns a versioned zero-initialized MAC/MTU/link/counter
record. HandleWrite accepts versioned bounded controls only with NET_CONTROL.
Unsupported controls return ENOTSUP; unknown/reserved fields return EINVAL.
There is no generic MMIO/PCI access or Linux ioctl encoding.

The packet FD supports native read/write/poll/wait-set/close. It is process-owner
bound (threads of that process share it). Dup within the owner is permitted;
inheritance into another process and file-capability export/transfer are refused.
CLOEXEC alone is insufficient to enforce this boundary. Existing FD/handle/IPC
operations must cover the new object variants exhaustively.

## Packet records and progress

Read/write buffers contain whole frame records: a little-endian u16 frame length,
u16 flags, u32 reserved-zero, then frame bytes padded to eight bytes with zeroes.
Frames exclude FCS and are 60..1514 bytes for the first interface. Limit a call to
32 records and 48 KiB total, below the existing source-copy ceiling. Check every
length, alignment, reserved field and total before mutation. Version the queue
options so future metadata cannot silently change this record interpretation.

Writes accept only complete records. Return the byte length of a complete accepted
prefix if TX capacity runs out; EAGAIN means no record accepted. Malformed records
cause EINVAL before publication. RX leaves frames queued on EFAULT; a too-small
buffer for the first record returns EFBIG without consuming it. Empty means
EAGAIN, not EOF. Query the required size through a bounded device/queue metadata
record. Readiness is level-triggered and never consumes data.

A reservation/generation protects an RX snapshot while user copying occurs with
no NIC/task lock held. Copy destination pages in bounded scopes; an EFAULT may
leave partial bytes in the caller buffer, which the failed read does not claim
as valid records. Commit consumption only after successful copy and generation
revalidation. Revocation cancels publication and wakes waits with EPIPE. Device
failure reports EIO. These errors use existing errno values; socket-specific
errors remain a later, separately reviewed API.

IRQ work drains bounded descriptors into preallocated CPU queues, updates counters
and schedules wake publication without allocation or userspace policy under the
NIC lock. TX/RX queues retain bounded ownership and do not trust caller indices.

## Stop, failure and service restart

Closing/revoking a session masks device causes and stops submission, invalidates
queue generations, wakes blocked operations and excludes in-flight kernel readers.
Only observed device quiescence/reset, bus-master disable and readback permit DMA
backing retirement. Uncertain completion quarantines backing and prevents reopening
that controller until recovery is proven; it is never fake-success reclamation.

A `netd` crash releases its raw descriptors and triggers the same revocation path.
A supervised replacement receives a fresh session, never the previous generation.
NIC authority does not grant process-management, disk, display or input authority.

## Required acceptance before declaring transport usable

Keep existing discovery, PHY loopback and external INTx budgets. Add denial for
ungranted processes, rights attenuation, forbidden inheritance/export, source and
destination faults, malformed/oversize/batch-prefix handling, descriptor wrap,
RX/TX exhaustion, lost wakeups, close during wait, process exit, revocation and
`netd` restart. No DMA page may return while device or CPU readers retain it.

Then run real userspace Ethernet/ARP/ICMP exchange with the isolated QEMU host
peer, followed by UDP/DNS through that transport. Logical-peer diagnostics remain
separate evidence. Hardware recovery, flood/storm/link-toggle and broader NIC
acceptance stay open until directly tested.

Alternative: shared CPU packet rings in v1 require approving a ring ABI, index/
generation validation and mapping/revocation lifetime now. They can reduce copies,
but enlarge the first authority change. The copied-batch design is recommended
for the first concrete end-to-end transport, then optimize measured bottlenecks.
