# Interruptible syscall and reschedule return boundary

Implementation date: 17 November 2026. This closes a narrow prerequisite;
applications still run on BSP and APs remain isolated hardware workers. It does
not authorize AP application dispatch, remote group kill, cancellation or backing
reclamation. The execution lease and finalization fence retain their contracts.

`syscall_entry` saves the user GPR scratch and FXSAVE image, selects its CPU-local
kernel stack, then enables interrupts only around `syscall_handler`. It disables
interrupts before calling a separate return boundary, after all handler scopes
have unwound, and before restoring user state. CPL0 IRQs always IRET. NMIs always
IRET, including when there is no TLB mailbox request, so an unsolicited NMI cannot
escape an interrupted handler through a pending user fault/preemption flag.

Ordinary APIC vector 246 publishes a CPU-local sticky hint. Each target admits
one outstanding monotonically increasing generation, serializing concurrent
senders without a lock or an acknowledgement wait. Send failure leaves that
same generation outstanding and retryable. APIC receipt is not completion.
A syscall return boundary captures the actual return value and user context,
marks the task Ready, and requests HAL handback. Exit, fault, blocking restart
and voluntary-yield paths retain their own context and status. A CPL3 IPI may
capture its interrupted context directly; a CPL0 IPI only publishes the hint.
No task/mapping mutex or IRQ guard spans the HAL handback. Existing admission
leases prevent another executor from entering a Ready task prematurely.

Only after `run_user_context` returns, FS/TLS and FPU pointers and syscall-stack
policy are reset, and the execution lease records Returned, is the captured
generation acknowledged with Release publication. Acquire readers can distinguish
request, delivery and actual handback. A returned unrelated dispatch cannot
acknowledge a request that has not been captured. The runner can redispatch the
same Ready root when no other runnable task exists.

## Checks

`python3 tools/test_irq_progress.py` compiles the actual mailbox and return-boundary
Rust modules with host task/HAL shims. It passes 8,000 contended generation
transactions, no premature ACK, normal negative syscall-result capture,
exit/block/magic priority, and CPL0 versus CPL3 handling. These checks do not
prove native IRQ entry or userspace restoration; the native image build compiles
the actual HAL MP source and the guest compiles the C probe with TinyCC.

The native suite is `tools/irq_progress_markers.json`. Build a smoke-test image
with at least two CPUs using the existing qemu harness. It compiles
`assets/tcc/irq_progress.c` with TinyCC inside the OS. Feature-only DebugLog code 5
holds a real IRQ-safe lock while AP 1's isolated vector-247 worker sends a
vector-246 request to BSP. It checks that the delivered hint and actual PIT IRQ
counter cannot advance under the lock, then checks PIT and IPI progress during
the interruptible handler. The pre-handback check rejects an early ACK. The
second call checks the post-HAL ACK and lock availability. Userspace checks the
syscall return value, four callee-saved GPRs, XMM7 payload and errno/TLS value
before making that second call. Both kernel and user markers are required.
The AP worker uses only atomics and APIC transport, never process/VFS/heap APIs.

Fresh serialized QEMU 10.0.13 TCG runs passed on 2, 4 and 8 vCPUs with the
original native marker budgets. They also passed the existing resource, GUI and
thread-fault regression suites; the external INTx peer independently checked
103 transformed frames in every run. See the [follow-up receipts](../verification/multitrack-progress-2026-11-17.json)
for image hashes and separate run results. The native probe exercises CPL0
delivery followed by the syscall return boundary; direct CPL3 IPI capture is
covered by the host boundary test, not a native remote-stop proof.

IRQ-off latency budgets, complete lock ranking, remote stop/cancel, remote
idle/runqueues and concurrent application/VM/fault stress remain open gates.
No KVM or application scheduling speedup was measured.
