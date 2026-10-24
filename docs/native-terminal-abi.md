# Native terminal Ioctl subset

`Ioctl(fd, operation, record, record_len)` uses Dunit operation numbers and bounded
little-endian records, not Linux requests/termios structures. Operations:

| op | record | Meaning |
|---|---|---|
| 0 | null, length 0 | Probe an attached PTY stdio descriptor |
| 1 | 8 bytes | Get rows, columns, pixel width, pixel height (four u16) |
| 2 | 8 bytes | Set those dimensions on the attached endpoint |
| 3 | 4 bytes | Get pending input byte count (u32) |

Unset dimensions are zero, meaning unknown; the kernel does not invent terminal
geometry. State is shared by the PTY endpoint's stdin/stdout/stderr and inherited/duplicated descriptors, independent of process identity. The process must
possess a valid descriptor referring to that terminal. Unknown descriptors return
EBADF, ordinary files/pipes return ENOTTY, bad record lengths return EINVAL,
invalid user memory returns EFAULT, unsupported terminal operations return
EOPNOTSUPP. Records are copied and validated before mutation.

musl translates TIOCGWINSZ/TIOCSWINSZ/FIONREAD and uses the public ioctl adapter
for isatty. Canonical line editing, terminal echo, baud rates, process groups,
TCGETS/TCSETS, drain/flush and signal generation are **not implemented**; requests
are rejected explicitly. Returning a fabricated termios record would claim a
line discipline the PTY does not have. Geometry change signals are likewise not
claimed. GUI master geometry publication awaits a descriptor mechanism for the
master endpoint; this subset does not overload an unrelated file descriptor
with a PTY table identifier.
