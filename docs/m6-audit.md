# M6 audit and M7 entry checklist

Initial source audit: `36e85ce`, 24 October 2026. Line references below refer
to that revision, not to later edits. This initial audit did not run QEMU.
“Present” means implementation and existing test source were found; it does
not substitute for a fresh execution. `partial` and `missing` must remain open
until the named behavior is exercised. Subsequent verification belongs in the
validation record below, preserving this baseline.

## Every M6 item at the audit baseline

Items follow their order in `DUNIT_OS_TECHNICAL_ROADMAP.md:483–685`.
M6.1 is the architectural contract, M6.7 the dependency table, and M6.8 the
profile contract; their acceptance requirements are included as rows.

| Item | Baseline status | M8 dependency | Evidence / remaining acceptance |
|---|---|---|---|
| M6.1 native POSIX facade, no Linux kernel ABI | partial | no | Native adapters and generated numbers exist; silent unsupported flags/actions violate error policy. |
| M6.2.1 full musl fork | present | no | `toolchains/dunit-musl`, `.gitmodules`. |
| M6.2.2 pinned upstream | present | no | Fork `UPSTREAM.md`. |
| M6.2.3 pinned integration | present | no | Git submodule pointer. |
| M6.2.4 thematic changes | present | no | Fork history and `DELTA.md`; signal/network/ldso remain separate gates. |
| M6.2.5 upstream delta and upgrade discipline | partial | no | `tools/dunit/gen-delta.sh`; no fresh upgrade rehearsal/checksum proof. |
| M6.2.6 retain unsupported source | present | no | Full tree, syscall sentinels; CI deletion check. |
| M6.2.7 licenses and version metadata | partial | no | COPYRIGHT and generated manifest exist; exact Dunit revision/ABI hash absent from manifest. |
| M6.2 reproducible clean/offline sysroot acceptance | unverified | no | Requires two clean builds, archive comparison and recorded compiler inputs. |
| M6.3.1 calling convention | present | no | `abi/README.md`, generated C/Rust definitions. |
| M6.3.2 independently generated syscall numbers | present | no | `abi/syscalls.abi`, `tools/gen_abi.py`. |
| M6.3.3 signed errors | present | no | `abi/errno.abi`, musl syscall-return adapter. |
| M6.3.4 fd lifetime/rights/inheritance/stdio | partial | no | Per-process fd table exists; spawn creates fresh stdio, no generic inherited-fd actions. |
| M6.3.5 scalar widths and alignment | present | no | ABI docs and x86_64 LP64 headers. |
| M6.3.6 versioned public wire structs | present | no | Stat/readdir adapters avoid exposing kernel structs directly. |
| M6.3.7 entry argc/argv/envp/auxv | present | no | `kernel/src/elf/mod.rs`, `tools/m6_c_abi_markers.json`. |
| M6.3.8 ELF contract | partial | no | Kernel parser/loader enforce static profile; release-side exhaustive ELF validation missing. |
| M6.3.9 feature query | present | no | ABI feature query, ABI conformance test. |
| M6.4.1 logical x86_64-dunit target | present | no | Fork configure/build wrapper. |
| M6.4.2 compiler-driver bootstrap | present | no | `tools/dunit/build-libc.sh`. |
| M6.4.3 linker/crt/includes/static defaults | present | no | Generated `dunit-cc`, native static hello gate. |
| M6.4.4 staged crt/libc/headers | present | no | Build script sysroot install. |
| M6.4.5 host-header/library isolation | partial | no | `-nostdinc/-nostdlib`; no test forbidding explicit host include paths. |
| M6.4.6 prefixed tools and manifest | partial | no | cc/ar/strip exist; manifest lacks exact fork revision and complete unsupported profile. |
| M6.4.7 CMake/Meson/Autoconf files | missing | no | Direct flow exists; no cross files at baseline. |
| M6.5.1 reuse generic upstream code | present | no | Most source files untouched. |
| M6.5.2 narrow native backend boundaries | present | no | `src/dunit`, native thread/process/time adapters. |
| M6.5.3 raw syscall/Linux/cancellation audit | partial | no | Barrier raw SYS_futex; open/pipe2/spawn silent fallbacks; cancellation API overclaims. |
| M6.5.4 public type layout | present | no | Upstream x86_64 LP64 `alltypes.h.in`; native structure conversion. |
| M6.5.5 reserved Dunit extension namespace | present | no | `__dunit_*` internal symbols, no Dunit public POSIX-name pollution. |
| M6-A.1 startup/init/termination | present | no | crt and hello/ABI tests. |
| M6-A.2 exit/write/errors/stdio fds | present | no | hello/stdio tests. |
| M6-A.3 ISO C and basic stdio | present | no | Existing static C suite; upstream libc-test still absent. |
| M6-A.4 constructors/BSS/alignment/argv/env | present | no | `m6_c_abi` and musl hello suite; fresh matrix required. |
| M6-B.1 anonymous VM and W^X | present | no | `m6_wx`, kernel VM self-test. |
| M6-B.2 allocator without brk | present | no | musl malloc backend and `m6_musl_malloc`. |
| M6-B.3 realtime clock | partial | no | `syscall/mod.rs:1900`: both clock IDs return uptime; real RTC/validity absent. |
| M6-B.4 secure entropy | partial | no | `elf/mod.rs:430`: TSC-seeded xorshift for AT_RANDOM, not a CSPRNG. |
| M6-C.1 file/dir/seek/stat/fsync APIs | partial | no | Tests exist; open adapter drops EXCL although kernel supports it. |
| M6-C.2 POSIX flag/metadata translation | partial | no | `src/dunit/open.c:22–35` drops unsupported semantic flags silently. |
| M6-C.3 buffered FILE and terminal query | present | no | stdio/read/seek tests and isatty adapter. |
| M6-C.4 platform path policy | partial | DNS only | `/system`, `/persist`, `/tmp` established; account database and DNS policy unchosen. |
| M6-C.5 installed-root artifacts | partial | no, M5 | Initrd artifacts are external to kernel, but root remains MemFS. |
| M6-C persistence acceptance | partial | no, M5 | C tests write `/persist`; libc write/fsync/power-cycle/hash case is not a distinct gate. |
| M6-D.1 spawn/wait/kill argv/env/cwd/fds | partial | no | SpawnArgs=79 argv/env, inherited cwd; no fd inheritance/redirection contract. |
| M6-D.2 direct native posix_spawn | partial | no | `posix_spawn.c` uses native syscall, but ignores fa/attr. |
| M6-D.3 pipe/dup/redirection | partial | no | Dup test works; pipe number unmapped; spawn actions absent. |
| M6-D.4 unsupported fork returns error | present | no | Unsupported syscall sentinel; fork not a M6 static-tier prerequisite. |
| M6-E.1 schedulable user threads | present | no | Native ThreadCreate and musl thread suite. |
| M6-E.2 FS.base/static TLS | present | no | Native SetThreadPointer/PT_TLS; dynamic DTV remains separate. |
| M6-E.3 create/exit/join/detach cleanup | present | no | Musl thread backend and existing thread tests; fresh long-run leak gate required. |
| M6-E.4 atomic compare/block/wake/timeouts | present | no | Native futex adapter; condvar tests. |
| M6-E.5 pthread mutex/cond/rwlock/once/errno | partial | no | Basic tests exist; barrier bypasses dispatcher; broader contention/timeout matrix needed. |
| M6-E.6 forbid async cancellation first | missing | no, signals later | `pthread_setcanceltype` accepts async; self `pthread_cancel` can terminate. “Inert” is false. |
| M6-F.1 signal model before signals | deferred | no | Requires explicit signal/interruption ABI design, not networking. |
| M6-F.2 no unsupported cancel/timers claims | partial | no | API behavior/documentation must reject unsupported functionality. |
| M6-F.3 sockets after network handles | deferred | yes | No packet/socket object model yet; do not implement Linux socket-number mappings. |
| M6-F.4 dynamic loader | deferred | no | Separate loader/DSO/TLS milestone, not required for static tier. |
| M6-F.5 official static-only binaries | present | no | Build wrappers and initrd use static executables. |
| M6.7 primitive dependency ordering | partial | sockets only | VM/thread base present; fd-actions, RTC and secure entropy unresolved. |
| M6.8 release capability manifest/profile | partial | sockets only | Text manifest exists, unsupported surface incomplete; machine-readable release contract needed. |
| M6.9.1 self-checking libc programs | present | no | `userspace/ctests`, `tools/m6_*_markers.json`; some tests print FAIL but exit 0. |
| M6.9.2 OS execution through qemu_test | present | no | Existing marker suites and harness. |
| M6.9.3 upstream libc-test subset | missing | no | No pinned source/subset/exclusion manifest. |
| M6.9.4 negative tests | partial | no | Pointer/fd/OOM/fault tests; interrupted waits need signals; path-byte coverage incomplete. |
| M6.9.5 automatic static ELF gate | partial | no | Fork CI tests ET_EXEC and TLS opcode only; lacks INTERP/NEEDED/WX/undef checks. |
| M6.9.6 rebuild/previous-binary compatibility | partial | no | `ensure_musl_sysroot.py` fingerprints source/compiler; previous-release fixture suite absent. |
| M6.9.7 separated release gates | partial | network only | Per-feature suites exist; no single static/persistence/thread release report; network/dynamic deferred. |

## Root causes and implementable corrections

* `toolchains/dunit-musl/tools/dunit/gen_syscall_h.py`: missing `pipe: 83`
  mapping. Existing `pipe(fd[2])` has the same native argument shape.
* `src/unistd/pipe2.c`: upstream fallback ignores failed fcntl and returns
  success for unsupported CLOEXEC/NONBLOCK. Reject these until fd flags exist;
  invalid bits must still return EINVAL, without allocating descriptors.
* `src/dunit/open.c:22–35`: map existing EXCL (`vfs.rs:32`, bit 5), validate
  ACCMODE and unknown flags, reject unsupported semantic requests. LARGEFILE
  is neutral with 64-bit off_t; NOCTTY is neutral with no controlling-tty model.
  CLOEXEC/NONBLOCK/DIRECTORY/NOFOLLOW/SYNC/DSYNC/ASYNC/DIRECT/NOATIME/PATH/TMPFILE
  must not disappear silently. Avoid userspace check-then-create for EXCL.
* `src/process/posix_spawn.c:20–23`: reject nonempty file actions and nonzero
  unsupported attribute flags before creating a child. Never temporarily
  mutate the parent's cwd/fds to simulate child actions: other threads race.
  Real atomic child fd-actions need an approved native spawn contract.
* `src/process/posix_spawnp.c`: upstream stores `__execvpe` in attr.__fn, which
  native spawn ignores. PATH lookup belongs in userspace and needs a test.
* `src/thread/pthread_barrier_wait.c:87–88`: replace direct Linux-shape
  SYS_futex wait with the native dispatcher and exercise repeated generations.
* `src/thread/pthread_cancel.c`, `pthread_setcanceltype.c`: explicit ENOTSUP
  is required until signal/interruption semantics exist. Test that rejection
  leaves thread state and output arguments unchanged.

## Deferred to M8 and independently deferred work

M8 dependencies: sockets, bind/connect/listen/accept/send/receive/shutdown,
nonblocking socket readiness, DNS/resolver and network identity/configuration.
They need native network handles, netd and its rights/event model first.
All must remain documented unsupported; socket-looking success is prohibited.

RTC/time and secure entropy enable TLS in M8 but **do not depend on networking**.
Their mechanisms may be developed independently after the required decisions.
Signals/interruption/cancellation, fork/COW and dynamic loader/TLS are separate
later contracts. Installed-root execution depends on M5 filesystem/root work,
not M8. A static M6 completion claim must state these limits explicitly.

## Documentation contradictions at baseline

* Roadmap:1287 says “M0–M6 выполнены”, contradicting M5/M6 open items.
* Roadmap:1190 calls static musl complete without profile/acceptance limits.
* Roadmap:609 claims O_EXCL/mkstemp done despite missing libc flag mapping.
* Roadmap:632 describes async cancellation as inert; self-cancel disproves it.
* Roadmap:681 overstates CI ELF checks: fork CI checks ET_EXEC and the TLS
  instruction, not the complete static safety contract.
* Roadmap:547 promises precise versions; generated sysroot manifest omits the
  exact Dunit fork revision and ABI hash.
* README:95 describes MemFS only for `/app`/`/assets`; root `/` is also MemFS.
  README:96 installer description must not imply persistent system root.
* README:101 musl claim needs a named/versioned supported static profile.
  Limitations:119–123 omit fork/image-exec/signals, cancellation, real wall
  time and secure randomness. README contains no milestone numbering itself.
* Legacy test filenames `m8_*profile*` and `m7_installer*` are historical names,
  not evidence that networking/SDK are complete.
* CLAUDE musl cache paragraph requires manual rm; automatic source/compiler
  fingerprint invalidation already exists in `ensure_musl_sysroot.py`.

## M7 decisions reserved for the user

| Decision | Options and trade-off | Recommendation to discuss |
|---|---|---|
| Wait-set | Native mixed-object wait vs individual waits; first changes ABI, second cannot provide one fair event loop. | Design native bounded wait-set before SDK event loop. |
| SDK runtime | no_std+alloc gives narrow native API; std-over-musl requires substantially broader POSIX backend. | no_std+alloc first, without declaring std support. |
| App manifest | Versioned schema shared by init/launcher/packager vs independent formats. | One schema, agree fields/capability semantics before code. |
| Drop/clipboard | Paths are simple but ambient/racy; capabilities scope authority but need lifetime/transfer semantics. | Scoped handles after agreeing rights and revocation. |
| tcc -run | File output + spawn already fits static flow; in-memory execution needs relocation and safe RW→RX lifecycle. | First test existing mprotect RW→RX; do not invent a new syscall or permit W+X. |
| dufetch source | Config describes distribution policy; native query reports kernel/hardware facts. | Reuse available stats for memory; agree uname/sysinfo ABI for missing facts. |

## New host tooling and use

`python3 tools/dunit_sdk.py --sysroot build/dunit-sysroot --output build/sdk`
generates `toolchain.cmake`, `meson.cross`, `config.site`, and `sdk.json`.
Regenerate after moving the sysroot. The compiler wrapper owns crt/static
flags and include isolation. The cross files restrict target library/header
discovery and pkg-config to that sysroot; host build tools remain host tools.
They do not sanitize intentionally supplied project compiler flags: an explicit
`-I/usr/include` is a project violation, not silently rewritten.

Examples: CMake `-DCMAKE_TOOLCHAIN_FILE=/absolute/build/sdk/toolchain.cmake`;
Meson `--cross-file /absolute/build/sdk/meson.cross`; Autoconf
`CONFIG_SITE=/absolute/build/sdk/config.site ./configure --build=<host-triplet>
--host=x86_64-dunit`. Update an old project config.sub if it rejects Dunit.
No runtime probe is cached as true without evidence.

`python3 tools/check_userspace_elf.py <binary> ...` checks ELF64/LE/x86_64,
ET_EXEC, OSABI System V/version 0, segment bounds, entry, user range, no
PT_INTERP/PT_DYNAMIC/dynamic sections, no W+X, explicit non-executable stack,
and no named undefined global/weak symbols when symbol tables are present.
Stripped files can be structurally checked but cannot prove removed symbol
information; release checks should run **before stripping**. ELF OSABI is an
identification field, not proof that a binary uses native syscall numbers;
native QEMU execution and provenance remain mandatory.

Host tests: `/usr/bin/python3 tools/test_m6_tools.py` exercises rejected ELF
mutations and generated CMake/Meson/shell files, including paths with spaces,
quotes and dollar signs. Meson/CMake syntax tests require those host tools.

## Validation record

Initial tool implementation: 11 host tests passed, including actual CMake
script parsing and Meson machine-file parsing. Existing `build/userspace/setup`
and `musl_hello` passed the static checker; these preexisting artifacts are not
evidence of a fresh native build. No native build or QEMU was run by this audit
subtask. Full integrated release results must be recorded by the coordinating
task after rebuilding the changed musl/native image.

Integrated implementation and fresh native verification supersede this baseline
where explicitly listed in [m6-readiness.md](m6-readiness.md). The baseline
statuses above are retained for traceability, not presented as current results.
