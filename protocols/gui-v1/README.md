# Dunit GUI protocol v1.0

This crate defines the DGUI surface protocol, its deterministic server model and
conformance tests. The live desktop also uses the sideband codecs in
[src/control.rs](src/control.rs) and [src/transfer.rs](src/transfer.rs).
“Must” states a requirement; “may” permits an alternative.

## Scope and native binding

The base protocol covers client connections, toplevel surfaces, CPU shared
buffers, atomic commit, configure/ack, frame callbacks, focus and input. It has
one seat, integer scaling and software composition. GPU resources, subsurfaces,
cross-surface popup roles, IME composition, output enumeration and privileged
shell policy are outside v1.0. TEXT_INPUT carries completed text. Negotiated XFR1
adds clipboard and drag/drop; its rights and lifetime contract is in
[the SDK transfer guide](../../docs/sdk/desktop-transfers.md#clipboard-and-dragdrop).

DGUI framing does not detect or fall back to historical GUI1 drawing messages.
The live desktop uses an independent userspace compositor and applications.
The abstract transport must provide authenticated peers, FIFO messages in each
direction, preserved message boundaries, disconnect notification and atomic
capability delivery. Packets are 32–256 bytes with no GUI fragmentation.

The native binding launches clients from the userspace application registry. Its
8-byte bootstrap identifies the compositor PID, checked against the
kernel-authenticated sender. Subsequent IPC is filtered to that PID. This is the
implemented launch trust contract; `dunit.gui.v1` remains a logical service name,
not an implemented authenticated service-discovery API.

Shared-buffer handles provide independent references, fixed backing size and
read-only/non-executable compositor mappings. No physical address or user pointer
is a wire capability. The native CAP1 announcement accompanies the FIFO import
sequence; the compositor attests its actual transferred handle, rights, backing
length and identity through the kernel. Advertised numbers alone authorize
nothing. Client writes finish before publication; BUFFER_RELEASE and completed
buffer destruction delimit SDK backing reuse.

The server model implements normalized DGUI input described below. The live
binding routes established INP1 seat input, including negotiated lifecycle and
pointer extensions. The model's SET_TITLE/SET_APP_ID metadata is not currently
applied to native desktop captions/identity: the SDK exposes manifest identity.
Panels, dock and launcher remain inside `gui_server` under the
[accepted retained-shell decision](../../docs/sdk/desktop-transfers.md#retained-shell-decision).
Reserved external policy opcodes are unavailable.

## Encoding, identifiers and ordering

All integers are little-endian fixed-width uN/iN, packed without implicit padding.
Native structs and usize are not wire encodings. Reserved fields are zero.
`str` is a u16 byte length followed by exactly that many UTF-8 bytes, with no NUL
or padding. Invalid UTF-8 or NUL returns BAD_VALUE.
`Rect` is `x:i32, y:i32, w:u32, h:u32`.

| Offset | Field | Value |
| --- | --- | --- |
| 0 | magic:u32 | 0x49554744; bytes 44 47 55 49, DGUI |
| 4 | major:u16, minor:u16 | 1, 0, including HELLO |
| 8 | opcode:u16, flags:u16 | flags=0 |
| 12 | size:u32 | Complete packet length including header |
| 16 | object:u64 | 0 for connection, otherwise surface/buffer ID |
| 24 | serial:u64 | Request ID or independent server event sequence |

Client serials start at 1 and strictly increase without repetition or wrap.
Server serials form an independent sequence with the same rules across every
reply and event. Replies correlate through payload `request:u64`, not the header
serial. A configure token is the CONFIGURE header serial; a commit token is the
COMMIT request serial. Exhaustion closes the connection; it cannot wrap. Client
threads serialize sends through one queue.

Every well-framed request receives exactly one RESULT, WELCOME or ERROR. Replies
follow request order; events may interleave. RESULT acknowledges acceptance,
not presentation. Causal events follow their reply: CREATE → RESULT → CONFIGURE;
COMMIT → RESULT → callback. DESTROY_SURFACE is the cleanup exception described
below. Fatal errors/disconnect may prevent outstanding replies; a sent serial
cannot be retried. Unknown opcodes, flags, enums/bits and trailing bytes fail.
Validate size before allocating.

Object IDs are client-selected nonzero u64 values in one connection namespace
shared by surfaces/buffers. Each successful creation uses an ID greater than any
previously successful creation. Deleted IDs stay stale for that connection;
IDs cannot address another connection. Server client_id values increase from 1
without wrap and are unique for that server lifetime; they are not capabilities.
A wrong zero/nonzero connection object returns BAD_VALUE; an absent object for
an object operation returns STALE_OBJECT.

## Negotiation and connection lifetime

```text
NEW -- HELLO / WELCOME --> ACTIVE -- DISCONNECT / RESULT --> CLOSED
NEW or ACTIVE -- fatal error / EOF / peer death --> CLOSED
```

Only HELLO is allowed in NEW; another request is fatal BAD_STATE. Repeated HELLO
in ACTIVE is recoverable BAD_STATE. HELLO offers a minor range; the server selects
the highest intersection. v1.0 supports [0,0]. Major must be 1 and later headers
use the negotiated version. Required features must be a subset of offered.

| Feature bit | Meaning |
| --- | --- |
| 0 | ARGB8888 |
| 1 | INPUT_LIFECYCLE for INP1 |
| 2 | DATA_TRANSFER for XFR1 |
| 3 | POINTER_MODE |

The server accepts a supported subset of offered features. Unknown offered bits
are ignored only in HELLO; an unsupported required bit returns UNSUPPORTED.
XRGB8888 is mandatory without a feature bit. Format bit 0 means XRGB8888 and
bit 1 means ARGB8888; input/transfer features do not change this format mask.

A server closes NEW after five monotonic seconds without complete HELLO; a client
closes after five seconds awaiting WELCOME. ACTIVE has no idle timeout. DISCONNECT
destroys its objects and no event follows its RESULT. A server restart invalidates
the connection, tokens and IDs: reconnect, negotiate, recreate and redraw. The
current SDK does not implement that reconnect lifecycle.

## Requests

Only IMPORT_BUFFER accepts exactly one attached capability; other requests accept
none. Payload lengths derive exactly from this table, including str/Rect tails.

| Opcode | Request | object | Payload in order |
| --- | --- | --- | --- |
| 0x0001 | HELLO | 0 | min_minor:u16, max_minor:u16, reserved:u32, offered_features:u64, required_features:u64 |
| 0x0002 | DISCONNECT | 0 | Empty |
| 0x0010 | CREATE_SURFACE | New surface ID | role:u32, width:u32, height:u32, format:u32 |
| 0x0011 | DESTROY_SURFACE | Surface | Empty |
| 0x0012 | SET_TITLE | Surface | title:str |
| 0x0013 | SET_APP_ID | Surface | app_id:str |
| 0x0014 | ACK_CONFIGURE | Surface | configure:u64 |
| 0x0020 | IMPORT_BUFFER | New buffer ID | width:u32, height:u32, stride:u32, format:u32, offset:u64 |
| 0x0021 | DESTROY_BUFFER | Buffer | Empty |
| 0x0022 | ATTACH_BUFFER | Surface | buffer:u64, damage_count:u32, reserved:u32, damage:Rect[damage_count] |
| 0x0023 | COMMIT | Surface | configure:u64, frame_callback:u32, reserved:u32 |

role=1 is TOPLEVEL; other roles return UNSUPPORTED. CREATE dimensions are desired
logical content size. Surface format/type remains fixed for its lifetime.
Title permits 0–160 bytes; app_id permits 1–128 ASCII bytes from `[A-Za-z0-9._-]`
and is initially empty. These are metadata, never authorization, applied to the
server model after successful RESULT. frame_callback is 0 or 1.
ATTACH buffer=0 explicitly unmaps and requires damage_count=0.

## Replies and events

| Opcode | Reply/event | object | Payload |
| --- | --- | --- | --- |
| 0x8001 | WELCOME | 0 | request:u64, client_id:u64, features:u64, formats:u32, reserved:u32 |
| 0x8002 | RESULT | Request object | request:u64 |
| 0x8003 | ERROR | Request object, or 0 for unreadable header | request:u64, code:u32, fatal:u32 |
| 0x8010 | CONFIGURE | Surface | width:u32, height:u32, scale:u32, state:u32 |
| 0x8011 | REQUEST_CLOSE | Surface | Empty |
| 0x8020 | BUFFER_RELEASE | Buffer | commit:u64 |
| 0x8021 | FRAME_DONE | Surface | commit:u64, time_ns:u64, status:u32, reserved:u32 |
| 0x8030 | POINTER_ENTER | Surface | x:i32, y:i32 |
| 0x8031 | POINTER_LEAVE | Surface | Empty |
| 0x8032 | POINTER_MOTION | Surface | time_ns:u64, x:i32, y:i32 |
| 0x8033 | POINTER_BUTTON | Surface | time_ns:u64, button:u32, pressed:u32 |
| 0x8034 | POINTER_AXIS | Surface | time_ns:u64, dx:i32, dy:i32 |
| 0x8040 | KEY_ENTER | Surface | modifiers:u32, reserved:u32 |
| 0x8041 | KEY_LEAVE | Surface | Empty |
| 0x8042 | KEY | Surface | time_ns:u64, usage:u32, pressed:u32, modifiers:u32, repeat:u32 |
| 0x8043 | TEXT_INPUT | Surface | time_ns:u64, text:str |

CONFIGURE state bits are activated=0, maximized=1, fullscreen=2; other bits zero.
FRAME_DONE status 0 is PRESENTED, meaning included in composition rather than a
physical scanout promise; 1 is DISCARDED, for example unmapped/occluded. Time is
monotonic nanoseconds since server start. TEXT_INPUT contains 1–160 bytes of
complete UTF-8 code points. ERROR fatal is 0/1 and request is 0 when the header
cannot be safely read. HELLO gets WELCOME instead of RESULT; a failed request
gets only ERROR.

## Surface, configure and commit

A surface goes ABSENT → UNMAPPED → MAPPED ↔ UNMAPPED → DESTROYED. CREATE begins
UNMAPPED and emits initial CONFIGURE after RESULT. It becomes visible only after
a successful buffer COMMIT. Any live state may be destroyed; later operations
return STALE_OBJECT.

The server retains the latest CONFIGURE and last ACK. A new configure replaces
the old token. ACK accepts the latest token exactly once; stale, foreign or repeated
tokens return BAD_SERIAL. ACK does not alter committed scene. COMMIT requires ACK
of latest and that token in its payload; multiple commits may use it until another
configure. A configure overtaking COMMIT returns BAD_SERIAL and retains pending
state. ACK latest and send a new COMMIT serial; old content remains until success.
The server keeps serving other clients while waiting for ACK.

ATTACH reserves an AVAILABLE buffer and pending geometry/damage. Repeated ATTACH
is BAD_STATE until successful COMMIT or surface destruction clears pending.
ACK before ATTACH/COMMIT is recommended, but ATTACH before ACK is valid. COMMIT
without pending is BAD_STATE; current-buffer reuse is never implicit. Unmap also
requires latest ACK.

COMMIT atomically checks token, geometry, format, quotas and callback capacity
before replacing committed state. Failure changes neither pending nor committed
and does not make a buffer BUSY. Success clears pending and binds the buffer to
the commit token. Pixels are immutable for the client from accepted ATTACH to
RELEASE. Buffer dimensions must equal configured logical dimensions × scale,
with matching surface format and checked multiplication. Content excludes server
decorations. Initial configure may adjust requested size; scales are 1,2,3,4.

Damage uses half-open pixel rectangles in the new backing: nonnegative x/y,
nonzero width/height and bounds inside the buffer. At most eight rectangles;
overlap is valid and zero rectangles means full damage. First map, geometry/scale
change and remap force full damage. Every backing must contain a complete valid
frame regardless of damage. Metadata changes require no COMMIT.

A successful callback=1 commit yields exactly one FRAME_DONE except on disconnect.
A second pending callback for that surface returns BUSY; callback=0 commits are
allowed. DISCARDED permits pacing hidden surfaces. Completion occurs on a
composition tick without waiting for visibility; it is not permission to write
pixels before RELEASE.

REQUEST_CLOSE asks the application to close; it may be ignored or repeated.
DESTROY removes input and pending/committed state, discards any callback and
releases buffers. Its RESULT follows those cleanup events and is a barrier:
no later event references the destroyed surface.

## Buffer ownership

```text
IMPORT → AVAILABLE -- ATTACH --> PENDING -- COMMIT --> BUSY
                   PENDING -- destroy surface --> AVAILABLE + RELEASE(commit=0)
                   BUSY -- last server reader done --> AVAILABLE + RELEASE(commit)
AVAILABLE -- DESTROY_BUFFER --> DESTROYED
```

IMPORT checks memory capability/type/READ|MAP, pins a mapping and stores immutable
metadata. Import failure closes the received server capability without creating
an object; the original client reference remains. A payload integer cannot
substitute for a capability. Reimporting one memory object within a connection
returns BAD_VALUE; bindings must compare actual identity. One memory object maps
to one buffer with no overlapping aliases. Shared pixels across connections do
not expose another connection's GUI objects.

A buffer belongs to one connection and at most one surface/pending/commit.
ATTACH of BUSY/PENDING returns BUSY. Destroy in either state returns BUSY and is
not deferred. Until AVAILABLE, the client must not write through any alias.
Memory safety must not depend on honest pixels: saved metadata and fixed backing
bounds prevent arbitrary reads, even if a client violates pixel immutability.
Such a client can corrupt its own image, not compositor memory.

Each successful ATTACH produces exactly one RELEASE when pending/committed readers
finish. A failed COMMIT produces no release. The current committed backing may
remain held until replacement/unmap/destroy, or be released earlier after a server
snapshot. commit=0 denotes canceled pending; otherwise it identifies successful
COMMIT. FRAME_DONE and RELEASE have no prescribed mutual order. Clients keep two
or three buffers and continue dispatching while awaiting release.

Disconnect/EOF/crash removes only that connection's objects, focus and policy
bindings. Exclude surfaces from scene/input, finish readers, then close mappings
and capabilities. Event delivery after disconnect is optional; cleanup is mandatory.
Client loss of server closes endpoint, mappings and handles. A RELEASE timeout
cannot prove that the old server stopped reading: kernel references survive until
real closure.

## Pixel formats and limits

XRGB8888 format=1 uses B,G,R,X bytes; X is ignored and alpha is 255. ARGB8888
format=2 uses B,G,R,A, premultiplied in sRGB byte space, source-over rounding
`(a*b+127)/255`. Clamp untrusted R/G/B to A. ARGB requires feature bit 0.
Stride and offset are multiples of four; stride >= width*4. Maps are read-only,
non-executable. Checked `end=offset+stride*(height-1)+width*4` must not exceed the
attested backing size. Row padding is not read as pixels.

| Resource | Maximum v1.0 |
| --- | --- |
| Packet; inbound/outbound queue per connection | 256 bytes; 128 packets each, 32 KiB |
| Server connections | 64; excess closes before HELLO |
| Surfaces/buffers per connection | 64/192 |
| Logical and pixel width/height | 1–4096; logical×scale <=4096 |
| Stride/backing object | 16384 bytes/64 MiB |
| Pinned bytes per connection/server | 256 MiB/512 MiB |
| Damage/pending attach/pending callback | 8 rectangles/1 per surface/1 per surface |
| Title/app ID/text event | 160/128/160 bytes |
| Held key usages/pointer buttons | 256/5 |

Account for complete backings, including offset/padding. Global budgets count each
import conservatively even when pages are shared across connections; internal
copies also consume budgets. Reserve resources before mutating scene and return
budget on release. Allocation shortage returns NO_MEMORY; quota excess returns
LIMIT_EXCEEDED. A server need not preallocate these maxima.

Full inbound transport returns WOULD_BLOCK without accepting a packet or losing
previous requests. That packet/capability stays with its sender and serial remains
unsent; wait for writable while dispatching incoming events. The server processes
at most 32 requests per connection per round-robin pass. Full outbound disconnects
only the slow client with fatal SLOW_CLIENT and best-effort ERROR. Key/button
input-count overflow resets focus and closes with fatal LIMIT_EXCEEDED; important
state transitions are never silently dropped or stored without bounds.

## Focus, normalized input and policy boundary

The model has at most one keyboard focus and one pointer focus. Only live mapped
surfaces receive input. Hit testing uses committed geometry, clips and stacking;
pixel transparency does not alter hit region. POINTER_ENTER/MOTION coordinates
are signed 24.8 logical content coordinates. AXIS deltas are signed 24.8 logical
pixels, positive right/down. Buttons are left=1, right=2, middle=3, back=4,
forward=5; pressed is 0/1.

Pointer order is LEAVE(old) → ENTER(new) → MOTION/BUTTON/AXIS(new). Press creates
an implicit surface grab until every button is released; coordinates may leave
content bounds. Destroy/unmap/disconnect cancels it. POINTER_LEAVE clears local
held buttons; synthetic release packets are unnecessary.

Keyboard order is KEY_LEAVE(old) → KEY_ENTER(new) → KEY/TEXT_INPUT(new).
KEY_LEAVE clears held keys/modifiers/repeat. KEY_ENTER reports current modifiers
without reintroducing physically held keys; their release is suppressed until a
new delivered press. KEY usage is USB HID page 0x07 in the low 16 bits, with zero
upper bits, rather than a PS/2 scancode. Modifiers are Shift=0, Ctrl=1, Alt=2,
Super=3, CapsLock=4, NumLock=5; left/right merge and values reflect post-key state.
repeat is 0/1, with 1 allowed only for an already-held key and pressed=1. The
server supplies repeats. Completed text follows its causative KEY and only goes
to keyboard focus. Focus changes discard pending text/repeat of the old surface.

Destroy/unmap sends LEAVE before deleting its target. COMMIT-unmap sends RESULT,
then LEAVE/callback/release; scene/input changes atomically before further input.
DESTROY uses its cleanup barrier. Unavailable peers receive local server cleanup.

An external DWM would require its own authenticated policy endpoint/capability.
HELLO features and SET_APP_ID cannot authorize it. Ordinary clients cannot set
global position, focus or stacking, or read foreign buffers. Reserved policy
opcodes 0x1000–0x1fff return ACCESS_DENIED. The model accepts validated external
policy decisions for replay; it retains surfaces and releases policy bindings
on policy-client loss. Its fallback places new toplevels above old, focuses by
click and uses server close gestures. The production shell remains in-process;
no external privileged shell protocol is implemented. Ordinary clients receive
no raw display/input access.

## Errors and validation order

Protocol error codes are positive u32 values, separate from negative kernel errno.
Recoverable ERROR preserves the failed operation's state except consuming its
request serial and closing delivered capabilities. Predelivery transport failure
does not consume serial; retry after ERROR uses a new one. Fatal cleanup destroys
the connection.

| Code | Name | Typical failure | Fatal |
| --- | --- | --- | --- |
| 1 | MALFORMED | Header/length/count/reserved/flags/attachment mismatch | Yes |
| 2 | VERSION_MISMATCH | Major/minor or no version intersection | Yes |
| 3 | BAD_OPCODE | Unknown opcode or wrong direction | Yes |
| 4 | BAD_STATE | Operation invalid in current state | NEW only |
| 5 | STALE_OBJECT | Absent/deleted object | No |
| 6 | WRONG_OBJECT_TYPE | Buffer used as surface | No |
| 7 | BAD_SERIAL | Old/unacknowledged configure | Header serial only |
| 8 | BAD_VALUE | Enum, geometry, UTF-8, duplicate ID/backing | No |
| 9 | UNSUPPORTED | Role/format/required feature | HELLO only |
| 10 | ACCESS_DENIED | Capability type/rights or policy opcode | No |
| 11 | OUT_OF_BOUNDS | Checked pixel/rectangle range overflow | No |
| 12 | BUSY | Held buffer or pending callback | No |
| 13 | LIMIT_EXCEEDED | Resource quota | Input overflow only |
| 14 | NO_MEMORY | Allocation/mapping shortage | No |
| 15 | SLOW_CLIENT | Outbound overflow | Yes |
| 16 | INTERNAL | Backend/invariant failure | Yes |

Priority is framing/attachments → version → header serial → opcode/direction/policy
range → connection state → object existence/type/fresh ID → payload enums/strings
→ configure/object state → checked bounds → quota/allocation → atomic mutation.
Return the earliest failure, validating fields within a stage in wire order.
Unknown enum gives BAD_VALUE; recognized disabled format/unsupported role gives
UNSUPPORTED. Malformed input never panics, including before HELLO; close if ERROR
cannot be sent. Failures do not reveal another client's IDs or data.

## Conformance and native verification

Tests under `tests/` cover framing/golden bytes, negotiation, object namespaces,
configure and buffer lifetime, bounds/resource limits, deterministic replay,
normalized input, sideband lifecycle, transfer and pointer modes. Replay receives
requests, transport failures, monotonic ticks, input and policy choices; identical
input must produce identical output, scene and counters. Host addresses, wall
clock and random IDs cannot affect it.

Required cases include stale/reused/cross-connection IDs; repeated serial/ACK;
configure overtaking COMMIT without losing pending; destroy cleanup barriers;
failed import/send/allocation rollback; exact backing end and one byte beyond;
width/stride/scale/offset overflow; malformed damage count versus quota overflow;
callback/release independence; focus/grab/repeat isolation; slow peers and restart;
and each hard limit at min/max/max+1. Native smoke alone does not establish these
model contracts.

```sh
cargo test --manifest-path protocols/gui-v1/Cargo.toml
python3 tools/check_gui_protocol.py
python3 tools/test_gui_protocol_source.py
python3 tools/gen_abi.py --check
python3 tools/test_sdk_gui_fixture.py component --accel tcg
```

The source gate derives canonical control tags and rejects duplicate production
byte/integer definitions while permitting independent test goldens. Native SDK
fixtures and OS acceptance always run through `tools/qemu_test.py`; serialize
builds/boots. The component fixture asserts two 15,360-pixel texture/composition/
presentation repairs and no unchanged-view publication. Pixel work is distinct
from frame time. See [window lifetime and damage](../../docs/sdk/layout-window.md)
and [SDK verification references](../../docs/sdk/widgets.md#verification).

## Live IPC control messages

[src/control.rs](src/control.rs) owns the existing control codecs. Ordinary SDK
clients and compositor use these definitions alongside DGUI.

| Family | Bytes | Layout |
| --- | --- | --- |
| CAP1 | 16 | Magic, transferred handle, advertised backing length, buffer object; four u32 words |
| INP1 | 20 | Magic, kind at 4, reserved 5–7, signed x/scancode and y/modifiers at 8/12, button/cooked value at 16 |
| RLD1 | 4 or 8 | Reload magic; configuration comes from the existing file |
| NOT1 | 8 + text | Magic, claimed UTF-8 length, at most 64 text bytes |

Capability decoding does not authorize a handle; native attestation supplies
actual length/rights/identity. Legacy input decoding rejects unknown kinds and
truncated key/pointer records while retaining historical eight-byte quit/leave.
Notification encoding ends at UTF-8 boundaries; decoding clamps claimed length to
received bytes and limit. Malformed notifications are recognized as that family
and cannot fall through into DGUI parsing. `tests/control.rs` pins bytes, negative
coordinates, lengths, UTF-8 and family separation.

The native compositor selects pixels from `Server::frame`, an accepted committed
snapshot including stride/offset/damage. Pending imports/attachments never replace
the displayed image. Legacy empty-damage clients retain conservative updates.

### Negotiated INP1 lifecycle

INPUT_LIFECYCLE is accepted through WELCOME. The 20-byte envelope, existing kinds
and legacy pointer values remain unchanged without the feature.

| Kind | Value | Fields |
| --- | --- | --- |
| KEY_UP | 7 | x=scancode, y=modifiers, value=0 |
| FOCUS_IN | 8 | x=y=value=0 |
| FOCUS_OUT | 10 | x=y=value=0 |

For negotiated MOVE/DOWN/UP/LEAVE/SCROLL, value low eight bits hold the button and
bits 8–11 hold Shift/Ctrl/Alt/Super; higher bits are zero. Key modifiers use only
those four bits. FOCUS_OUT cancels SDK captures, presses, pending activation,
menus/tooltips and text-selection timers. Legacy connections receive no new
focus or release kinds.

`Input::decode_for_features` validates accepted features, exact length, reserved
fields, kinds/modifiers and zero key-up text/focus payload. Historical
`Input::decode` retains old acceptance and short quit/leave. `pointer_value`,
`pointer_button` and `pointer_modifiers` own packing/access. `for_features` and
`encode_for_features` suppress new kinds and strip modifiers for legacy peers;
invalid canonical events fail. `Server::negotiated_features` exposes features only
for ACTIVE connections. `tests/input_lifecycle.rs` covers golden bytes, projection,
feature combinations, malformed input, fallback and bounded arbitrary decoding.
Pointer modifiers come from approved enqueue-time device snapshots, not later
keyboard queue state; see [SDK input](../../docs/sdk/input.md#pointer-modifier-snapshots).

### Optional pointer modes

POINTER_MODE feature bit 3 gates confinement, relative lock, acknowledgement,
revocation and signed device-relative input. PTR1 requests and INP1 event fields,
rights checks, cancellation and the compatible 32/40/56-byte device records are
specified in [pointer modes](../../docs/sdk/input.md#pointer-modes). Existing
records and unnegotiated clients retain their wire semantics.
