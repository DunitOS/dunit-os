mod fd;
mod packet;
pub(crate) use packet::{bootstrap_net_root, grant_net_connect, grant_net_device};
mod runner;
pub use runner::{exit_current_process, request_current_user_exit, request_current_user_fault, user_fault_escape_requested, preempt_switch_requested, clear_preempt_switch, set_preemption_enabled, preemption_enabled, preemption_count, reset_preemption_count, enter_user_process};
mod lifecycle;
mod finalization;
pub(crate) use lifecycle::{wake_wait_set_waiters, child_exit_snapshot, is_pid_waiting_for_dispatch};
pub use lifecycle::{allocate_pid, current_tid, current_thread_pointer, set_current_thread_pointer, set_terminal_foreground_process, set_foreground_process, current_process_output_sink, current_process_is_terminal_foreground, request_terminal_stdin_for_current, terminal_stdin_waiting_pid, provide_terminal_stdin, take_terminal_stdin_for_current, get_process_snapshots, process_stats, insert_process_record, create_user_process_record, current_is_privileged, current_may_kill, create_user_thread, request_thread_auto_reap, wait_thread, wake_ipc_waiters, wake_poll_waiters, terminal_input_ready, wake_pipe_waiters, wake_expired_waiters, is_pid_blocked, reap_process, process_exists, thread_count_for_pid, kill_process, is_pid_runnable, wait_for_child, cleanup_prepared_children, autoreap_process, snapshot_processes, init_current_kernel_process};
use lifecycle::{process_address_space, process_record_index, log_process_transition, current_entity_id, owner_of, current_pid, release_finished_thread, cleanup_owned_threads, remove_waiter};
pub mod scheduler;
mod transitions;
mod syscall_boundary;
pub(crate) use syscall_boundary::reschedule_user_interrupt;
mod parking;
#[cfg(feature = "boot-smoke-tests")]
mod parking_smoke;
#[cfg(feature = "boot-smoke-tests")]
pub use parking_smoke::run_parking_smoke;
mod execution;
#[cfg(feature = "boot-smoke-tests")]
mod execution_smoke;
#[cfg(feature = "boot-smoke-tests")]
mod finalization_smoke;
#[cfg(feature = "boot-smoke-tests")]
pub use execution_smoke::run_execution_ownership_smoke;
pub use transitions::{mark_process_prepared_as_ready, mark_process_ready,
    save_current_user_context_for_yield,
    timer_preempt_save_and_schedule};
pub(crate) use parking::{block_current, pipe_block_current, poll_block_current, wait_set_block_current};
use parking::wake_entry;
mod terminal_stdin;
mod wait_queue;
mod futex;
mod user_word;
pub use futex::{futex_wait, futex_wake, run_clear_child_tid_on_exit, set_current_clear_child_tid, FutexWaitOutcome};
use wait_queue::{wake_matching, WaitEntry, WaitKey, WaitQueue};
pub(crate) use wait_queue::Checkpoint as WaitCheckpoint;
pub(crate) fn wait_checkpoint() -> WaitCheckpoint { WAIT_QUEUE.lock().checkpoint() }
mod current_access;
pub use current_access::{with_current_process, with_current_process_mut, with_current_thread_mut, with_process_mut};
pub(crate) use current_access::signal_current_handle;
use current_access::with_current_thread;
mod resources;
#[cfg(feature = "boot-smoke-tests")]
mod fd_retirement_smoke;
mod dispatch;
use dispatch::KernelStack;
pub(crate) use dispatch::active_address_space_pin;
#[cfg(feature = "boot-smoke-tests")]
pub use dispatch::run_dispatch_pin_smoke;
pub use resources::inherit_process_fds;
#[cfg(feature = "boot-smoke-tests")]
pub use resources::run_resource_cleanup_smoke;
#[cfg(feature = "boot-smoke-tests")]
mod fault_irq_smoke;

use terminal_stdin::TerminalStdin;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use core::cell::UnsafeCell;

use crate::handle::{
    Handle, HandleError, HandleObject, RIGHT_DISPLAY_MASTER, RIGHT_INPUT_MASTER, RIGHT_MAP,
    RIGHT_READ, RIGHT_SIGNAL, RIGHT_TRANSFER, RIGHT_WRITE,
};
use crate::memory::pmm::{get_pmm, PhysicalAddress};
use crate::memory::vmm::{AddressSpace, PageFlags, VirtualAddress};
#[cfg(feature = "boot-smoke-tests")]
use crate::memory::vmm::ActiveAddressSpace;
use crate::sync::{InterruptGuard, IrqSafeSpinLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadId(pub u64);

pub type ProcessFd = u32;

pub const FIRST_PROCESS_FD: ProcessFd = 3;
pub const MAX_PROCESS_FD: ProcessFd = 1024;
pub const DEFAULT_KERNEL_STACK_SIZE: usize = 0x40000;

static NEXT_FD_IDENTITY: AtomicU64 = AtomicU64::new(1);
static NEXT_PID: AtomicU64 = AtomicU64::new(1);
static NEXT_SHARED_VM_ID: AtomicU64 = AtomicU64::new(1);
static SHARED_VM_OBJECTS: IrqSafeSpinLock<BTreeMap<u64, SharedVmObject>> =
    IrqSafeSpinLock::new(BTreeMap::new());

struct SharedVmObject {
    frames: Vec<PhysicalAddress>,
    refs: usize,
}

fn shared_vm_release(id: u64, count: usize) {
    let mut objects = SHARED_VM_OBJECTS.lock();
    let Some(object) = objects.get_mut(&id) else { return; };
    object.refs -= count;
    if object.refs == 0 {
        let object = objects.remove(&id).unwrap();
        if let Some(pmm) = get_pmm() {
            for frame in object.frames { pmm.free_frame(frame); }
        }
    }
}

/// Аллоцирует набор обнулённых физических фреймов под разделяемый объект и
/// регистрирует его в глобальном реестре с `refs = 1`. Возвращает id объекта.
/// В отличие от [`Process::create_shared_vm`] НЕ добавляет id в `shared_owned` —
/// эту единственную ссылку держит вызывающий (например, capability-хэндл).
fn alloc_shared_vm_object(length: usize) -> Result<u64, ProcessError> {
    let length = length.checked_add(USER_PAGE_SIZE - 1)
        .map(|value| value & !(USER_PAGE_SIZE - 1))
        .ok_or(ProcessError::InvalidMemoryRange)?;
    if length == 0 || length > 64 * 1024 * 1024 {
        return Err(ProcessError::InvalidMemoryRange);
    }
    let pmm = get_pmm().ok_or(ProcessError::OutOfMemory)?;
    let mut frames = Vec::new();
    for _ in 0..length / USER_PAGE_SIZE {
        let Some(frame) = pmm.alloc_frame() else {
            for frame in frames { pmm.free_frame(frame); }
            return Err(ProcessError::OutOfMemory);
        };
        unsafe {
            core::ptr::write_bytes(
                crate::memory::vmm::phys_to_virt(frame.as_usize()) as *mut u8,
                0, USER_PAGE_SIZE,
            );
        }
        frames.push(frame);
    }
    let id = NEXT_SHARED_VM_ID.fetch_add(1, Ordering::SeqCst);
    SHARED_VM_OBJECTS.lock().insert(id, SharedVmObject { frames, refs: 1 });
    Ok(id)
}
mod registry;
#[cfg(feature = "boot-smoke-tests")]
mod registry_smoke;
#[cfg(feature = "boot-smoke-tests")]
pub use registry_smoke::run_registry_ownership_smoke;
use registry::{table as process_table, with_record as with_task_record};
/// Count of committed timer-driven context switches. Only advanced when
/// `timer_preempt_save_and_schedule` actually hands the CPU to another process,
/// so a non-zero value is proof that preemption fired (see M1 [PREEMPT-TEST]).
static PREEMPTION_COUNT: AtomicU64 = AtomicU64::new(0);
// UP round-robin is the default. A child may finish before its parent's first wait.
static PREEMPTION_ENABLED: AtomicBool = AtomicBool::new(true);
/// Registration and notification checkpoint share this lock; task -> queue.
static WAIT_QUEUE: IrqSafeSpinLock<WaitQueue> = IrqSafeSpinLock::new(WaitQueue::new());

/// Foreground selection and pending line ownership are one transaction. No
/// registry, wait-queue, console or user-copy work runs while this lock is held.
struct TerminalState {
    foreground: Option<ProcessId>,
    sink: ProcessOutputSink,
    stdin: TerminalStdin,
}
static TERMINAL_STATE: IrqSafeSpinLock<TerminalState> = IrqSafeSpinLock::new(TerminalState {
    foreground: None,
    sink: ProcessOutputSink::SerialOnly,
    stdin: TerminalStdin::new(),
});

pub const WAIT_KIND_EMPTY: i32 = -1;
pub const WAIT_KIND_SPAWN_PREPARED: i32 = -2;

/// Process lifecycle for the non-preemptive foundation:
///
/// Prepared: process object exists and owns its address space, kernel stack,
/// fd table, cwd and pid, but no executable image has run yet.
/// Ready: executable context has been prepared and a future scheduler may run it.
/// Running: crate::cpu::runtime().current_pid points at the process table record while the CPU is in
/// user mode. The process object still lives inside PROCESS_TABLE.
/// Dead: execution finished or faulted; wait observes the real status only when
/// has_run is true. Prepared/not-started children keep an explicit wait status.
/// Reaped: terminal exec or a future wait path consumed the heavyweight
/// ownership. Metadata stays behind briefly for diagnostics, but it is not
/// runnable or waitable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Prepared,
    Ready,
    Running,
    Blocked,
    Dead,
    Reaped,
}

pub struct ProcessRecord {
    pub pid: ProcessId,
    /// None for a process/main thread; Some(pid) for an additional thread.
    pub owner: Option<ProcessId>,
    thread_auto_reap: bool,
    pub parent: Option<ProcessId>,
    pub state: ProcessState,
    pub status: Option<ProcessExitStatus>,
    pub has_run: bool,
    pub waitable: bool,
    pub path: String,
    /// Independently owned task object; registry guards never span its mutex.
    process: Option<registry::Task>,
    execution: execution::State,
    wait_generation: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    pub pid: ProcessId,
    pub parent: Option<ProcessId>,
    pub state: ProcessState,
    pub status: Option<ProcessExitStatus>,
    pub has_run: bool,
    pub waitable: bool,
    pub path: String,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessStats {
    pub total: u64,
    pub prepared: u64,
    pub ready: u64,
    pub running: u64,
    pub blocked: u64,
    pub dead: u64,
    pub reaped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitRecord {
    pub pid: ProcessId,
    pub kind: i32,
    pub code: i32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CpuContext {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

impl CpuContext {
    pub const fn new() -> Self {
        Self {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            rsp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rip: 0,
            rflags: 0,
        }
    }
}

#[repr(C, align(16))]
pub struct FpuState(UnsafeCell<[u8; 512]>);

// On this UP kernel, the selected process alone owns its FPU buffer while
// user mode runs. IRQs disable nesting before saving to the same buffer.
unsafe impl Sync for FpuState {}

impl FpuState {
    pub const fn new() -> Self {
        let mut bytes = [0; 512];
        // FINIT defaults: x87 control word and MXCSR. The remaining registers
        // start cleared, so each new process receives an independent state.
        bytes[0] = 0x7f;
        bytes[1] = 0x03;
        bytes[24] = 0x80;
        bytes[25] = 0x1f;
        Self(UnsafeCell::new(bytes))
    }
}

pub struct Process {
    pub pid: ProcessId,
    pub state: ProcessState,
    pub context: CpuContext,
    /// 16-byte aligned legacy x87/MMX/SSE state (FXSAVE format).
    pub fpu_state: Arc<FpuState>,
    /// x86_64 userspace thread pointer installed in IA32_FS_BASE.
    pub fs_base: u64,
    /// User word to zero + futex-wake when this thread exits (0 = none).
    /// Native CLONE_CHILD_CLEARTID / set_tid_address; musl uses it to release
    /// the thread-list lock on thread death.
    pub clear_child_tid: u64,
    /// Privileged (system-control) capability. Granted by the kernel to the
    /// processes it launches directly (init and `exec` from the kernel console)
    /// and inherited across the libdunit SpawnProcess path, but NEVER granted
    /// via the userspace posix_spawn (SpawnArgs) path. Gates destructive admin
    /// operations: killing a process that is not the caller's child, and the
    /// mutating TerminalDiag verbs (blkwrite/mkfs/mount/install).
    pub privileged: bool,
    pub is_kernel: bool,
    pub cwd: String,
    pub status: Option<ProcessExitStatus>,
    address_space: Option<Arc<IrqSafeSpinLock<AddressSpace>>>,
    kernel_stack: Option<Arc<KernelStack>>,
    pub entry_argc: usize,
    pub entry_argv: usize,
    pub entry_envp: usize,
    fd_table: BTreeMap<ProcessFd, FdEntry>,
    fd_wakeup_pending: bool,
    retired_resources: resources::RetiredResources,
    /// How many process fds reference each underlying VFS descriptor. dup/dup2
    /// make several process fds share one VFS open file (and its offset); the
    /// VFS close only fires when the last of them is closed.
    vfs_fd_refs: BTreeMap<crate::fs::vfs::FileDescriptor, u32>,
    next_mmap_addr: usize,
    shared_owned: Vec<u64>,
    shared_pages: BTreeMap<usize, u64>,
    guard_regions: BTreeMap<usize, usize>,
    tls_image: Vec<u8>,
    tls_mem_size: usize,
    tls_align: usize,
    tls_allocation: Option<(usize, usize)>,
    /// Таблица хэндлов процесса (capabilities с правами).
    handle_table: crate::handle::HandleTable,
    /// Счётчик доставленных, но ещё не считанных сигналов (право SIGNAL).
    pending_signals: u64,
    pub(crate) wait_set: Option<Box<crate::syscall::wait_set::Pending>>,
    /// Explicit opt-in to atomic IPC handle delivery; legacy inboxes cannot drain it.
    pub(crate) ipc_handles_enabled: bool,
}

const USER_MMAP_BASE: usize = 0x0000_0010_0000_0000;
const USER_MMAP_END: usize = 0x0000_7000_0000_0000;
// Exclusive top of the user half: first address past the canonical lower half.
// Checks use `base >= USER_ADDRESS_END`, equivalent to `base > 0x7FFF_FFFF_FFFF`
// (see `syscall::USER_SPACE_END`, the inclusive form of the same boundary).
const USER_ADDRESS_END: u64 = 0x0000_8000_0000_0000;
const USER_PAGE_SIZE: usize = 4096;

fn vm_range_end(start: usize, length: usize) -> Result<usize, ProcessError> {
    if start < USER_MMAP_BASE || start & (USER_PAGE_SIZE - 1) != 0 || length == 0 {
        return Err(ProcessError::InvalidMemoryRange);
    }
    let length = length.checked_add(USER_PAGE_SIZE - 1)
        .map(|value| value & !(USER_PAGE_SIZE - 1))
        .ok_or(ProcessError::InvalidMemoryRange)?;
    let end = start.checked_add(length).ok_or(ProcessError::InvalidMemoryRange)?;
    if end > USER_MMAP_END { return Err(ProcessError::InvalidMemoryRange); }
    Ok(end)
}

impl Process {
    /// Creates an isolated userspace process.
    ///
    /// Address-space allocation failure is part of the API contract: callers
    /// must handle it instead of receiving a process that only looks usable.
    pub fn new(pid: ProcessId) -> Result<Self, ProcessError> {
        Self::new_user(pid)
    }

    fn new_without_address_space(pid: ProcessId, is_kernel: bool) -> Self {
        let mut process = Self {
            pid,
            state: ProcessState::Ready,
            context: CpuContext::new(),
            fpu_state: Arc::new(FpuState::new()),
            fs_base: 0,
            clear_child_tid: 0,
            privileged: false,
            is_kernel,
            cwd: String::from("/"),
            status: None,
            address_space: None,
            kernel_stack: None,
            entry_argc: 0,
            entry_argv: 0,
            entry_envp: 0,
            fd_table: BTreeMap::new(),
            fd_wakeup_pending: false,
            retired_resources: resources::RetiredResources::default(),
            vfs_fd_refs: BTreeMap::new(),
            next_mmap_addr: USER_MMAP_BASE,
            shared_owned: Vec::new(),
            shared_pages: BTreeMap::new(),
            guard_regions: BTreeMap::new(),
            tls_image: Vec::new(),
            tls_mem_size: 0,
            tls_align: 1,
            tls_allocation: None,
            handle_table: crate::handle::HandleTable::new(),
            pending_signals: 0,
            wait_set: None,
            ipc_handles_enabled: false,
        };
        process.reserve_stdio();
        process
    }

    pub fn new_kernel(pid: ProcessId) -> Self {
        Self::new_without_address_space(pid, true)
    }

    pub fn new_user(pid: ProcessId) -> Result<Self, ProcessError> {
        let kernel_stack = Arc::new(KernelStack::new());
        let mut process = Self {
            pid,
            state: ProcessState::Ready,
            context: CpuContext::new(),
            fpu_state: Arc::new(FpuState::new()),
            fs_base: 0,
            clear_child_tid: 0,
            privileged: false,
            is_kernel: false,
            cwd: String::from("/"),
            status: None,
            address_space: Some(
                Arc::new(IrqSafeSpinLock::new(AddressSpace::new().map_err(|_| ProcessError::AddressSpaceCreateFailed)?)),
            ),
            kernel_stack: Some(kernel_stack),
            entry_argc: 0,
            entry_argv: 0,
            entry_envp: 0,
            fd_table: BTreeMap::new(),
            fd_wakeup_pending: false,
            retired_resources: resources::RetiredResources::default(),
            vfs_fd_refs: BTreeMap::new(),
            next_mmap_addr: USER_MMAP_BASE,
            shared_owned: Vec::new(),
            shared_pages: BTreeMap::new(),
            guard_regions: BTreeMap::new(),
            tls_image: Vec::new(),
            tls_mem_size: 0,
            tls_align: 1,
            tls_allocation: None,
            handle_table: crate::handle::HandleTable::new(),
            pending_signals: 0,
            wait_set: None,
            ipc_handles_enabled: false,
        };
        process.reserve_stdio();
        Ok(process)
    }

    fn new_user_thread(tid: ThreadId, entry: u64, stack_top: u64, arg: u64) -> Self {
        let mut thread = Self::new_without_address_space(ProcessId(tid.0), false);
        thread.kernel_stack = Some(Arc::new(KernelStack::new()));
        thread.fd_table.clear(); // Owned by the process, shared by its threads.
        thread.context.rip = entry;
        thread.context.rsp = stack_top;
        thread.context.rdi = arg;
        thread.context.rflags = 0x202;
        thread
    }

    pub fn address_space(&self) -> Option<crate::sync::IrqSafeSpinLockGuard<'_, AddressSpace>> {
        self.address_space.as_ref().map(|space| space.lock())
    }

    pub fn address_space_mut(&mut self) -> Option<crate::sync::IrqSafeSpinLockGuard<'_, AddressSpace>> {
        self.address_space()
    }

    pub fn kernel_stack_top(&self) -> Option<usize> {
        self.kernel_stack.as_ref().map(|stack| stack.top())
    }

    #[cfg(feature = "boot-smoke-tests")]
    pub unsafe fn install_syscall_stack(&self) -> Result<(), ProcessError> {
        let stack_top = self.kernel_stack_top().ok_or(ProcessError::NoKernelStack)?;
        crate::hal::syscall_set_kernel_stack_top(stack_top as u64);
        Ok(())
    }

    pub unsafe fn reset_syscall_stack_policy() {
        crate::hal::syscall_reset_kernel_stack();
    }

    #[cfg(feature = "boot-smoke-tests")]
    pub unsafe fn activate_address_space(&self) -> Result<ActiveAddressSpace, ProcessError> {
        self.address_space()
            .ok_or(ProcessError::NoAddressSpace)
            .map(|address_space| address_space.activate())
    }

    pub fn terminate(&mut self) {
        self.state = ProcessState::Dead;
    }

    pub fn exit(&mut self, code: i32) {
        self.status = Some(ProcessExitStatus::Exited(code));
        self.terminate();
    }

    pub fn fault(&mut self, fault: ProcessFault) {
        self.status = Some(ProcessExitStatus::Fault(fault));
        self.terminate();
    }

    /// Maps zero-filled private anonymous memory into this process.
    /// A zero address chooses a monotonically increasing, page-aligned region;
    /// a non-zero address is treated as an exact request and must be free.
    pub fn map_anonymous(
        &mut self,
        requested_addr: usize,
        length: usize,
        writable: bool,
        executable: bool,
        guarded: bool,
        accessible: bool,
    ) -> Result<usize, ProcessError> {
        if self.is_kernel || length == 0 {
            return Err(ProcessError::InvalidMemoryRange);
        }
        let map_length = length
            .checked_add(USER_PAGE_SIZE - 1)
            .map(|value| value & !(USER_PAGE_SIZE - 1))
            .ok_or(ProcessError::InvalidMemoryRange)?;
        let total_length = if guarded {
            map_length.checked_add(2 * USER_PAGE_SIZE)
        } else {
            Some(map_length)
        }.ok_or(ProcessError::InvalidMemoryRange)?;
        let start = if requested_addr == 0 {
            self.next_mmap_addr
        } else {
            if requested_addr & (USER_PAGE_SIZE - 1) != 0 {
                return Err(ProcessError::InvalidMemoryRange);
            }
            requested_addr
        };
        let end = start
            .checked_add(total_length)
            .ok_or(ProcessError::InvalidMemoryRange)?;
        if start < USER_MMAP_BASE || end > USER_MMAP_END {
            return Err(ProcessError::InvalidMemoryRange);
        }

        let mut address_space = self
            .address_space_mut()
            .ok_or(ProcessError::NoAddressSpace)?;
        let mut page = start;
        while page < end {
            match address_space
                .translate_user_page(crate::memory::vmm::VirtualAddress::from_usize(page))
            {
                Ok(None) => {}
                Ok(Some(_)) => return Err(ProcessError::AddressInUse),
                Err(_) => return Err(ProcessError::InvalidMemoryRange),
            }
            page += USER_PAGE_SIZE;
        }

        let mut page_flags = crate::memory::vmm::PageFlags::empty();
        if writable {
            page_flags |= crate::memory::vmm::PageFlags::WRITABLE;
        }
        if !executable {
            page_flags |= crate::memory::vmm::PageFlags::NO_EXECUTE;
        }

        let mut mapped_end = start;
        while mapped_end < end {
            if address_space
                .map_user_page(
                    crate::memory::vmm::VirtualAddress::from_usize(mapped_end),
                    page_flags,
                )
                .is_err()
            {
                let mut rollback = start;
                while rollback < mapped_end {
                    let _ = address_space
                        .unmap_user_page(crate::memory::vmm::VirtualAddress::from_usize(rollback));
                    rollback += USER_PAGE_SIZE;
                }
                return Err(ProcessError::OutOfMemory);
            }
            mapped_end += USER_PAGE_SIZE;
        }

        drop(address_space);
        if requested_addr == 0 {
            self.next_mmap_addr = end;
        }
        if !accessible {
            let accessible_start = if guarded { start + USER_PAGE_SIZE } else { start };
            let accessible_end = if guarded { end - USER_PAGE_SIZE } else { end };
            let mut address_space = self.address_space_mut().unwrap();
            for page in (accessible_start..accessible_end).step_by(USER_PAGE_SIZE) {
                address_space.protect_user_page(
                    VirtualAddress(page), PageFlags::NO_EXECUTE, false,
                ).map_err(|_| ProcessError::InvalidMemoryRange)?;
            }
        }
        if guarded {
            let mut address_space = self.address_space_mut().unwrap();
            let guard_flags = PageFlags::NO_EXECUTE;
            address_space.protect_user_page(VirtualAddress(start), guard_flags, false)
                .map_err(|_| ProcessError::InvalidMemoryRange)?;
            address_space.protect_user_page(VirtualAddress(end - USER_PAGE_SIZE), guard_flags, false)
                .map_err(|_| ProcessError::InvalidMemoryRange)?;
            drop(address_space);
            self.guard_regions.insert(start + USER_PAGE_SIZE, map_length);
            Ok(start + USER_PAGE_SIZE)
        } else {
            Ok(start)
        }
    }

    pub fn create_shared_vm(&mut self, length: usize) -> Result<u64, ProcessError> {
        if self.is_kernel || length == 0 { return Err(ProcessError::InvalidMemoryRange); }
        let id = alloc_shared_vm_object(length)?;
        self.shared_owned.push(id);
        Ok(id)
    }

    pub fn close_shared_vm(&mut self, id: u64) -> Result<(), ProcessError> {
        let index = self.shared_owned.iter().position(|owned| *owned == id)
            .ok_or(ProcessError::InvalidMemoryRange)?;
        self.shared_owned.swap_remove(index);
        shared_vm_release(id, 1);
        Ok(())
    }

    pub fn map_shared_vm(
        &mut self, id: u64, requested_addr: usize, writable: bool,
    ) -> Result<usize, ProcessError> {
        if self.is_kernel { return Err(ProcessError::InvalidUserContext); }
        let frames = {
            let mut objects = SHARED_VM_OBJECTS.lock();
            let object = objects.get_mut(&id).ok_or(ProcessError::InvalidMemoryRange)?;
            object.refs += object.frames.len();
            object.frames.clone()
        };
        let length = frames.len() * USER_PAGE_SIZE;
        let start = if requested_addr == 0 { self.next_mmap_addr } else { requested_addr };
        let end = start.checked_add(length).ok_or(ProcessError::InvalidMemoryRange);
        let valid = start >= USER_MMAP_BASE && start & (USER_PAGE_SIZE - 1) == 0
            && end.as_ref().map(|end| *end <= USER_MMAP_END).unwrap_or(false);
        if !valid {
            shared_vm_release(id, frames.len());
            return Err(ProcessError::InvalidMemoryRange);
        }
        let end = end.unwrap();
        let Some(mut address_space) = self.address_space_mut() else {
            shared_vm_release(id, frames.len());
            return Err(ProcessError::NoAddressSpace);
        };
        let mut mapped = 0;
        for (index, frame) in frames.iter().copied().enumerate() {
            let page = start + index * USER_PAGE_SIZE;
            if address_space.translate_user_page(VirtualAddress(page)).ok().flatten().is_some()
                || address_space.map_user_frame(
                    VirtualAddress(page), frame,
                    PageFlags::NO_EXECUTE | if writable { PageFlags::WRITABLE } else { PageFlags::empty() },
                ).is_err()
            {
                for rollback in 0..mapped {
                    let _ = address_space.unmap_user_page(VirtualAddress(start + rollback * USER_PAGE_SIZE));
                }
                shared_vm_release(id, frames.len());
                return Err(ProcessError::AddressInUse);
            }
            mapped += 1;
        }
        drop(address_space);
        for index in 0..frames.len() {
            self.shared_pages.insert(start + index * USER_PAGE_SIZE, id);
        }
        if requested_addr == 0 { self.next_mmap_addr = end; }
        Ok(start)
    }

    pub fn unmap_range(&mut self, start: usize, length: usize) -> Result<(), ProcessError> {
        let end = vm_range_end(start, length)?;
        let mut guard = None;
        for (&usable, &len) in &self.guard_regions {
            let reservation_start = usable - USER_PAGE_SIZE;
            let reservation_end = usable + len + USER_PAGE_SIZE;
            if start < reservation_end && end > reservation_start {
                if start != usable || end != usable + len {
                    return Err(ProcessError::InvalidMemoryRange);
                }
                guard = Some((usable, len));
            }
        }
        {
            let address_space = self.address_space_mut().ok_or(ProcessError::NoAddressSpace)?;
            for page in (start..end).step_by(USER_PAGE_SIZE) {
                if address_space.translate_user_page(VirtualAddress(page)).ok().flatten().is_none() {
                    return Err(ProcessError::InvalidMemoryRange);
                }
            }
        }
        for page in (start..end).step_by(USER_PAGE_SIZE) {
            self.address_space_mut().unwrap().unmap_user_page(VirtualAddress(page))
                .map_err(|_| ProcessError::InvalidMemoryRange)?;
            if let Some(id) = self.shared_pages.remove(&page) { shared_vm_release(id, 1); }
        }
        if let Some((usable, len)) = guard {
            let mut address_space = self.address_space_mut().unwrap();
            address_space.unmap_user_page(VirtualAddress(usable - USER_PAGE_SIZE)).unwrap();
            address_space.unmap_user_page(VirtualAddress(usable + len)).unwrap();
            drop(address_space);
            self.guard_regions.remove(&usable);
        }
        Ok(())
    }

    pub fn protect_range(
        &mut self, start: usize, length: usize, writable: bool,
        executable: bool, present: bool,
    ) -> Result<(), ProcessError> {
        let end = vm_range_end(start, length)?;
        for (usable, len) in &self.guard_regions {
            if start <= *usable - USER_PAGE_SIZE && end > *usable - USER_PAGE_SIZE
                || start <= *usable + *len && end > *usable + *len {
                return Err(ProcessError::InvalidMemoryRange);
            }
        }
        let mut address_space = self.address_space_mut().ok_or(ProcessError::NoAddressSpace)?;
        for page in (start..end).step_by(USER_PAGE_SIZE) {
            if address_space.translate_user_page(VirtualAddress(page)).ok().flatten().is_none() {
                return Err(ProcessError::InvalidMemoryRange);
            }
        }
        let flags = (if writable { PageFlags::WRITABLE } else { PageFlags::empty() })
            | (if executable { PageFlags::empty() } else { PageFlags::NO_EXECUTE });
        for page in (start..end).step_by(USER_PAGE_SIZE) {
            address_space.protect_user_page(VirtualAddress(page), flags, present)
                .map_err(|_| ProcessError::InvalidMemoryRange)?;
        }
        Ok(())
    }

    pub fn install_tls_template(
        &mut self,
        image: &[u8],
        mem_size: usize,
        align: usize,
    ) -> Result<(), ProcessError> {
        if image.len() > mem_size || mem_size == 0 || !align.is_power_of_two() {
            return Err(ProcessError::InvalidMemoryRange);
        }
        self.tls_image.clear();
        self.tls_image.extend_from_slice(image);
        self.tls_mem_size = mem_size;
        self.tls_align = align.max(1);
        let (base, length, thread_pointer) = self.allocate_tls_instance()?;
        self.fs_base = thread_pointer as u64;
        self.tls_allocation = Some((base, length));
        Ok(())
    }

    fn allocate_tls_instance(&mut self) -> Result<(usize, usize, usize), ProcessError> {
        if self.tls_mem_size == 0 {
            return Ok((0, 0, 0));
        }
        let tls_size = self.tls_mem_size.checked_add(self.tls_align - 1)
            .map(|value| value & !(self.tls_align - 1))
            .ok_or(ProcessError::InvalidMemoryRange)?;
        // Dunit static TLS follows x86_64 variant II: TLS data is directly
        // below the thread pointer and FS:0 contains the thread pointer itself.
        let mapped_length = tls_size.checked_add(core::mem::size_of::<u64>())
            .and_then(|value| value.checked_add(USER_PAGE_SIZE - 1))
            .map(|value| value & !(USER_PAGE_SIZE - 1))
            .ok_or(ProcessError::InvalidMemoryRange)?;
        let base = self.map_anonymous(0, mapped_length, true, false, false, true)?;
        let address_space = self.address_space().ok_or(ProcessError::NoAddressSpace)?;
        let mut copied = 0;
        while copied < self.tls_image.len() {
            let addr = base + copied;
            let chunk = (USER_PAGE_SIZE - (addr & (USER_PAGE_SIZE - 1)))
                .min(self.tls_image.len() - copied);
            let phys = address_space.translate_user_page(VirtualAddress(addr))
                .map_err(|_| ProcessError::InvalidMemoryRange)?
                .ok_or(ProcessError::InvalidMemoryRange)?;
            unsafe {
                core::ptr::copy_nonoverlapping(
                    self.tls_image.as_ptr().add(copied),
                    crate::memory::vmm::phys_to_virt(phys.as_usize()) as *mut u8,
                    chunk,
                );
            }
            copied += chunk;
        }
        let thread_pointer = base + tls_size;
        let tp_phys = address_space.translate_user_page(VirtualAddress(thread_pointer))
            .map_err(|_| ProcessError::InvalidMemoryRange)?
            .ok_or(ProcessError::InvalidMemoryRange)?;
        unsafe {
            (crate::memory::vmm::phys_to_virt(tp_phys.as_usize()) as *mut u64)
                .write_unaligned(thread_pointer as u64);
        }
        Ok((base, mapped_length, thread_pointer))
    }

    pub fn set_thread_pointer(&mut self, base: u64) -> Result<(), ProcessError> {
        if base >= USER_ADDRESS_END {
            return Err(ProcessError::InvalidMemoryRange);
        }
        self.fs_base = base;
        unsafe { crate::hal::set_fs_base(base); }
        Ok(())
    }

    fn take_fd_wakeup(&mut self) -> bool {
        core::mem::take(&mut self.fd_wakeup_pending)
    }

    pub fn fd_count(&self) -> usize {
        self.fd_table.len()
    }

    fn reserve_stdio(&mut self) {
        self.fd_table.insert(0, FdEntry::stdin());
        self.fd_table.insert(1, FdEntry::stdout());
        self.fd_table.insert(2, FdEntry::stderr());
    }

    // --- Таблица хэндлов (capabilities с правами) ------------------------------

    /// Создаёт объект памяти нужного размера и возвращает хэндл с полным набором
    /// прав для памяти: READ|WRITE|MAP|TRANSFER.
    pub fn handle_create_memory(&mut self, len: usize) -> Result<Handle, HandleError> {
        if len == 0 || len > crate::handle::MAX_MEMORY_OBJECT {
            return Err(HandleError::TooLarge);
        }
        let data = alloc::vec![0u8; len];
        Ok(self.handle_table.insert(
            HandleObject::Memory(data),
            RIGHT_READ | RIGHT_WRITE | RIGHT_MAP | RIGHT_TRANSFER,
        ))
    }

    /// Создаёт конечную точку для сигналов процессу `target`. Право SIGNAL|TRANSFER.
    pub fn handle_create_endpoint(&mut self, target: ProcessId) -> Handle {
        self.handle_table
            .insert(HandleObject::Endpoint(target), RIGHT_SIGNAL | RIGHT_TRANSFER)
    }

    pub fn handle_rights(&self, handle: Handle) -> Result<u32, HandleError> {
        self.handle_table.rights(handle)
    }

    pub fn handle_create_file(&mut self, file: alloc::sync::Arc<crate::fs::file_cap::FileCapability>, rights: u32) -> Handle {
        self.handle_table.insert(HandleObject::File(file), rights)
    }

    pub fn handle_file(&self, handle: Handle, rights: u32) -> Result<alloc::sync::Arc<crate::fs::file_cap::FileCapability>, HandleError> {
        let entry=self.handle_table.require(handle,rights)?;
        match &entry.object {
            HandleObject::File(file)=>Ok(alloc::sync::Arc::clone(file)),
            _=>Err(HandleError::WrongType),
        }
    }

    pub fn handle_dup(&mut self, handle: Handle, new_rights: u32) -> Result<Handle, HandleError> {
        // Master handles remain non-duplicable under the existing capability
        // contract. Repeated acquisition retains independent device leases;
        // it does not grant transfer or duplication rights.
        if let Ok(entry) = self.handle_table.get(handle) {
            if matches!(entry.object, HandleObject::Display | HandleObject::Input) {
                return Err(HandleError::WrongType);
            }
        }
        let new = self.handle_table.duplicate(handle, new_rights)?;
        // Дубликат разделяемого буфера — ещё одна ссылка на тот же объект фреймов;
        // учитываем её, чтобы буфер не освободился раньше времени.
        if let Ok(entry) = self.handle_table.get(new) {
            if let HandleObject::SharedFrames { id } = entry.object {
                if let Some(object) = SHARED_VM_OBJECTS.lock().get_mut(&id) {
                    object.refs += 1;
                }
            }
        }
        Ok(new)
    }

    /// Создаёт разделяемый буфер (zero-copy физические фреймы) и возвращает
    /// capability-хэндл с правами READ|WRITE|MAP|TRANSFER. Хэндл держит одну
    /// ссылку на объект; close/teardown/dup/transfer корректно её учитывают.
    pub fn handle_create_shared(&mut self, len: usize) -> Result<Handle, HandleError> {
        if self.is_kernel {
            return Err(HandleError::WrongType);
        }
        let id = alloc_shared_vm_object(len).map_err(|error| match error {
            ProcessError::OutOfMemory => HandleError::MapFailed,
            _ => HandleError::TooLarge,
        })?;
        Ok(self.handle_table.insert(
            HandleObject::SharedFrames { id },
            RIGHT_READ | RIGHT_WRITE | RIGHT_MAP | RIGHT_TRANSFER,
        ))
    }

    /// Закрывает хэндл; если это было мастер-право на дисплей — возвращает его.
    pub fn handle_close(&mut self, handle: Handle) -> Result<(), HandleError> {
        let entry = self.handle_table.remove(handle)?;
        if matches!(entry.object, HandleObject::Display) {
            crate::handle::release_display(self.pid);
        }
        if matches!(entry.object, HandleObject::Input) {
            crate::handle::release_input(self.pid);
        }
        if let HandleObject::SharedFrames { id } = entry.object {
            shared_vm_release(id, 1);
        } else if let HandleObject::File(file) = entry.object {
            self.retired_resources.files.push(file);
        } else if let HandleObject::NetDevice(device) = entry.object {
            self.retired_resources.net_devices.push(device);
        }
        Ok(())
    }

    /// Читает из объекта памяти (нужно право READ). Возвращает число байт.
    pub fn handle_memory_read(&self, handle: Handle, out: &mut [u8]) -> Result<usize, HandleError> {
        let entry = self.handle_table.require(handle, RIGHT_READ)?;
        match &entry.object {
            HandleObject::Memory(data) => {
                let n = out.len().min(data.len());
                out[..n].copy_from_slice(&data[..n]);
                Ok(n)
            }
            _ => Err(HandleError::WrongType),
        }
    }

    /// Пишет в объект памяти (нужно право WRITE). Возвращает число байт.
    pub fn handle_memory_write(&mut self, handle: Handle, src: &[u8]) -> Result<usize, HandleError> {
        let entry = self.handle_table.require_mut(handle, RIGHT_WRITE)?;
        match &mut entry.object {
            HandleObject::Memory(data) => {
                let n = src.len().min(data.len());
                data[..n].copy_from_slice(&src[..n]);
                Ok(n)
            }
            _ => Err(HandleError::WrongType),
        }
    }

    /// Отображает содержимое объекта памяти в адресное пространство процесса
    /// (нужно право MAP). Возвращает виртуальный адрес отображения.
    pub fn handle_map(&mut self, handle: Handle, addr: usize, len: usize) -> Result<usize, HandleError> {
        // Проверяем право и достаём байты до вызова map_anonymous, чтобы не
        // держать заимствование таблицы хэндлов через &mut self.
        let shared = {
            let entry = self.handle_table.require(handle, RIGHT_MAP)?;
            match &entry.object {
                HandleObject::Memory(_) => None,
                HandleObject::SharedFrames { id } => {
                    Some((*id, entry.rights & RIGHT_WRITE != 0))
                }
                _ => return Err(HandleError::WrongType),
            }
        };
        if let Some((id, writable)) = shared {
            // Zero-copy: алиасим реальные физические фреймы объекта в адресное
            // пространство через уже проверенный путь map_shared_vm. Право WRITE
            // в маске хэндла определяет writable-маппинг — так capability
            // разграничивает доступ (напр. сервер RO, клиент RW).
            return self
                .map_shared_vm(id, addr, writable)
                .map_err(|_| HandleError::MapFailed);
        }
        let (map_len, writable) = {
            let entry = self.handle_table.get(handle)?;
            let writable = entry.rights & RIGHT_WRITE != 0;
            match &entry.object {
                HandleObject::Memory(data) => (len.min(data.len()), writable),
                _ => return Err(HandleError::WrongType),
            }
        };
        let req_len = len.max(1);
        let mapped = self
            .map_anonymous(addr, req_len, true, false, false, true)
            .map_err(|_| HandleError::MapFailed)?;
        // Адресное пространство вызывающего процесса активно во время системного
        // вызова, поэтому пишем прямо в только что отображённый пользовательский
        // регион.
        let entry = self.handle_table.get(handle)?;
        if let HandleObject::Memory(data) = &entry.object {
            unsafe {
                core::ptr::copy_nonoverlapping(data.as_ptr(), mapped as *mut u8, map_len);
            }
        }
        if !writable {
            let page_len = req_len
                .checked_add(USER_PAGE_SIZE - 1)
                .map(|v| v & !(USER_PAGE_SIZE - 1))
                .ok_or(HandleError::MapFailed)?;
            self.protect_range(mapped, page_len, false, false, true)
                .map_err(|_| HandleError::MapFailed)?;
        }
        Ok(mapped)
    }

    /// Возвращает истинный размер (в байтах) backing-объекта разделяемого буфера
    /// по хэндлу. Нужен доверенному получателю (напр. компоситору), чтобы
    /// валидировать геометрию присланного буфера против фактически выделенных
    /// фреймов, а не против размера, который называет недоверенный отправитель.
    pub fn handle_shared_len(&self, handle: Handle) -> Result<usize, HandleError> {
        let entry = self.handle_table.get(handle)?;
        match &entry.object {
            HandleObject::SharedFrames { id } => {
                let objects = SHARED_VM_OBJECTS.lock();
                let object = objects.get(id).ok_or(HandleError::BadHandle)?;
                Ok(object.frames.len() * USER_PAGE_SIZE)
            }
            _ => Err(HandleError::WrongType),
        }
    }

    /// Validate authority and copy the target; delivery happens after this borrow.
    fn signal_target(&self, handle: Handle) -> Result<ProcessId, HandleError> {
        let entry = self.handle_table.require(handle, RIGHT_SIGNAL)?;
        match &entry.object {
            HandleObject::Endpoint(pid) => Ok(*pid),
            _ => Err(HandleError::WrongType),
        }
    }

    /// Забирает и обнуляет счётчик доставленных сигналов процесса.
    pub fn handle_take_signals(&mut self) -> u64 {
        core::mem::take(&mut self.pending_signals)
    }

    pub fn add_pending_signal(&mut self, value: u64) {
        self.pending_signals = self.pending_signals.saturating_add(value);
    }

    /// Захватывает мастер-право на дисплей (эксклюзивно). Возвращает хэндл с
    /// правом DISPLAY_MASTER (без TRANSFER: singleton не переносится и не
    /// дублируется — см. handle_dup/handle_take_for_transfer).
    pub fn handle_display_acquire(&mut self) -> Result<Handle, HandleError> {
        if !crate::handle::try_acquire_display(self.pid) {
            return Err(HandleError::DisplayBusy);
        }
        Ok(self
            .handle_table
            .insert(HandleObject::Display, RIGHT_DISPLAY_MASTER))
    }

    /// Validate the handle and capture the exclusive seat generation together.
    pub(crate) fn require_input_master(&self, handle: Handle) -> Result<crate::input::ReadToken, HandleError> {
        let entry = self.handle_table.require(handle, RIGHT_INPUT_MASTER)?;
        if !matches!(entry.object, HandleObject::Input) {
            return Err(HandleError::AccessDenied);
        }
        crate::input::read_token(self.pid.0).ok_or(HandleError::AccessDenied)
    }

    /// Захватывает мастер-право на источник ввода (эксклюзивно). Возвращает хэндл
    /// с правом INPUT_MASTER (без TRANSFER) либо `DisplayBusy` (ресурс занят).
    pub fn handle_input_acquire(&mut self) -> Result<Handle, HandleError> {
        if !crate::handle::try_acquire_input(self.pid) {
            return Err(HandleError::DisplayBusy);
        }
        Ok(self
            .handle_table
            .insert(HandleObject::Input, RIGHT_INPUT_MASTER))
    }

    /// Забирает хэндл из таблицы для передачи; проверяет право TRANSFER.
    /// Мастер-право на дисплей не переносится (эксклюзивный системный ресурс с
    /// отдельным учётом владельца) — попытка её передать отклоняется.
    pub fn handle_take_for_transfer(
        &mut self,
        handle: Handle,
    ) -> Result<(HandleObject, u32), HandleError> {
        let entry = self.handle_table.require(handle, RIGHT_TRANSFER)?;
        if matches!(entry.object, HandleObject::Display | HandleObject::Input | HandleObject::NetRoot | HandleObject::NetDevice(_) | HandleObject::NetConnect { .. }) {
            return Err(HandleError::WrongType);
        }
        self.handle_table.take_for_transfer(handle)
    }

    /// Принимает переданный объект в свою таблицу, сохраняя (сужённые) права.
    pub fn handle_receive(&mut self, object: HandleObject, rights: u32) -> Handle {
        self.handle_table.insert(object, rights)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.take_fd_resources().release();
        drop(self.take_vm_resources());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    NoCurrentProcess,
    NoSuchProcess,
    NotChild,
    InvalidFd,
    FdTableFull,
    NoAddressSpace,
    AddressSpaceCreateFailed,
    NoKernelStack,
    InvalidUserContext,
    ProcessNotPrepared,
    ProcessAlreadyExists,
    NotRunnable,
    SchedulerUnavailable,
    InvalidMemoryRange,
    AddressInUse,
    OutOfMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessExit {
    pub pid: ProcessId,
    pub status: ProcessExitStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessExitStatus {
    Exited(i32),
    Fault(ProcessFault),
}

impl ProcessExitStatus {
    pub const fn exit_code(self) -> i32 {
        match self {
            ProcessExitStatus::Exited(code) => code,
            ProcessExitStatus::Fault(fault) => fault.exit_code(),
        }
    }

    pub const fn kind_code(self) -> i32 {
        match self {
            ProcessExitStatus::Exited(_) => 0,
            ProcessExitStatus::Fault(fault) => fault_kind_to_i32(fault),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessFault {
    PageFault,
    GeneralProtection,
    InvalidOpcode,
    DivideByZero,
    Unknown,
}

impl ProcessFault {
    pub const fn exit_code(self) -> i32 {
        match self {
            ProcessFault::PageFault => -14,
            ProcessFault::GeneralProtection => -11,
            ProcessFault::InvalidOpcode => -4,
            ProcessFault::DivideByZero => -8,
            ProcessFault::Unknown => -1,
        }
    }

    pub const fn reason(self) -> &'static str {
        match self {
            ProcessFault::PageFault => "page fault",
            ProcessFault::GeneralProtection => "general protection fault",
            ProcessFault::InvalidOpcode => "invalid opcode",
            ProcessFault::DivideByZero => "divide by zero",
            ProcessFault::Unknown => "user fault",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdTarget {
    Stdin,
    Stdout,
    Stderr,
    Vfs(crate::fs::vfs::FileDescriptor),
    /// One end of an anonymous pipe (see crate::pipe). `write` selects which end.
    Pipe { id: u64, write: bool },
    /// Process-local native Ethernet queue, never inherited or exported.
    Packet { generation: u64, access: u32 },
    /// Native application socket tied to the current process and service generation.
    NetSocket { token: u64 },
    /// Private request/response channel for the registered network service.
    NetService { token: u64 },
}

impl FdTarget {
    pub(crate) fn owner_bound(self) -> bool {
        matches!(self, Self::Packet { .. } | Self::NetSocket { .. } | Self::NetService { .. })
    }
}

/// What the fd layer asks the syscall layer to finish after a close/dup2 that
/// dropped the last reference to a shared object (the Process cannot reach the
/// VFS or the wait queue itself).
#[derive(Clone, Copy)]
pub enum FdCleanup {
    None,
    CloseVfs(crate::fs::vfs::FileDescriptor),
    WakePipe(u64),
    ClosePacket(u64),
    CloseSocket(FdTarget),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum ProcessOutputSink {
    SerialOnly = 0,
    Terminal = 1,
}

/// Descriptor flags are per-fd; status belongs to the shared open description.
#[derive(Debug, Clone)]
pub struct FdEntry {
    pub target: FdTarget,
    /// Unique descriptor-slot binding; dups and reused slots get a fresh id.
    pub(crate) identity: u64,
    pub cloexec: bool,
    pub terminal: Option<Arc<crate::pty::Endpoint>>,
    status: Arc<AtomicU32>,
}
impl FdEntry {
    pub fn new(target: FdTarget, status: u32) -> Self {
        Self { target, identity: NEXT_FD_IDENTITY.fetch_add(1, Ordering::Relaxed), cloexec: false, terminal: None, status: Arc::new(AtomicU32::new(status)) }
    }
    pub fn stdin() -> Self { Self::new(FdTarget::Stdin, 1) }
    pub fn stdout() -> Self { Self::new(FdTarget::Stdout, 2) }
    pub fn stderr() -> Self { Self::new(FdTarget::Stderr, 2) }
    pub fn vfs(fd: crate::fs::vfs::FileDescriptor) -> Self { Self::new(FdTarget::Vfs(fd), 3) }
    pub fn pipe(id: u64, write: bool) -> Self { Self::new(FdTarget::Pipe { id, write }, if write { 2 } else { 1 }) }
    pub fn status(&self) -> u32 { self.status.load(Ordering::SeqCst) }
    pub fn set_status(&self, status: u32) { self.status.store(status, Ordering::SeqCst); }
}

pub fn allocate_fd(entry: FdEntry) -> Result<ProcessFd, ProcessError> {
    with_current_process_mut(|process| process.allocate_fd(entry)).ok_or(ProcessError::NoCurrentProcess)?
}

pub fn open_vfs_fd(vfs_fd: crate::fs::vfs::FileDescriptor) -> Result<ProcessFd, ProcessError> {
    with_current_process_mut(|process| process.open_vfs_fd(vfs_fd)).ok_or(ProcessError::NoCurrentProcess)?
}

pub fn open_pipe_fd(id: u64, write: bool) -> Result<ProcessFd, ProcessError> {
    with_current_process_mut(|process| process.install_pipe_fd(id, write)).ok_or(ProcessError::NoCurrentProcess)?
}

pub fn drop_fd(fd: ProcessFd) -> Result<FdCleanup, ProcessError> {
    with_current_process_mut(|process| process.drop_fd(fd)).ok_or(ProcessError::NoCurrentProcess)?
}

pub fn dup_fd(
    oldfd: ProcessFd,
    newfd: Option<ProcessFd>,
) -> Result<(ProcessFd, FdCleanup), ProcessError> {
    with_current_process_mut(|process| process.dup_fd(oldfd, newfd)).ok_or(ProcessError::NoCurrentProcess)?
}

/// Owned snapshot; never expose an fd-table reference across a syscall stage.
pub fn get_fd(fd: ProcessFd) -> Option<FdEntry> {
    with_current_process(|process| process.get_fd(fd).cloned()).flatten()
}


#[cfg(feature = "boot-smoke-tests")]
pub fn run_process_address_space_smoke() -> bool {
    crate::memory::serial_write("[PROCESS-ADDRSPACE-TEST] START\r\n");

    let pid = allocate_pid();
    let mut process = match Process::new_user(pid) {
        Ok(process) => process,
        Err(_) => {
            crate::memory::serial_write("[PROCESS-ADDRSPACE-TEST] create failed\r\n");
            return false;
        }
    };

    let mut address_space = match process.address_space_mut() {
        Some(address_space) => address_space,
        None => {
            crate::memory::serial_write("[PROCESS-ADDRSPACE-TEST] no address space\r\n");
            return false;
        }
    };

    let user_page = VirtualAddress::from_usize(0x0050_0000);
    if address_space
        .map_user_page(user_page, PageFlags::WRITABLE)
        .is_err()
    {
        crate::memory::serial_write("[PROCESS-ADDRSPACE-TEST] map failed\r\n");
        return false;
    }

    drop(address_space);
    unsafe {
        let _active = process.activate_address_space().ok();
        if _active.is_none() {
            crate::memory::serial_write("[PROCESS-ADDRSPACE-TEST] activate failed\r\n");
            return false;
        }

        let ptr = user_page.as_usize() as *mut u64;
        core::ptr::write_volatile(ptr, 0x5052_4F43_4153_5043);
        if core::ptr::read_volatile(ptr) != 0x5052_4F43_4153_5043 {
            crate::memory::serial_write("[PROCESS-ADDRSPACE-TEST] user page check failed\r\n");
            return false;
        }
    }

    crate::memory::serial_write("[PROCESS-ADDRSPACE-TEST] OK\r\n");
    true
}

fn serial_write_u64(mut value: u64) {
    let mut buf = [0u8; 20];
    let mut index = buf.len();

    if value == 0 {
        crate::memory::serial_write("0");
        return;
    }

    while value > 0 {
        index -= 1;
        buf[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }

    for byte in &buf[index..] {
        let ch = [*byte];
        let s = unsafe { core::str::from_utf8_unchecked(&ch) };
        crate::memory::serial_write(s);
    }
}

fn serial_write_usize(value: usize) {
    serial_write_u64(value as u64);
}

fn serial_write_state(state: ProcessState) {
    let name = match state {
        ProcessState::Prepared => "Prepared",
        ProcessState::Ready => "Ready",
        ProcessState::Running => "Running",
        ProcessState::Blocked => "Blocked",
        ProcessState::Dead => "Dead",
        ProcessState::Reaped => "Reaped",
    };
    crate::memory::serial_write(name);
}

#[cfg(feature = "boot-smoke-tests")]
pub fn run_process_kernel_stack_smoke() -> bool {
    crate::memory::serial_write("[PROCESS-KSTACK-TEST] START\r\n");

    let pid = allocate_pid();
    let process = match Process::new_user(pid) {
        Ok(process) => process,
        Err(_) => {
            crate::memory::serial_write("[PROCESS-KSTACK-TEST] create failed\r\n");
            return false;
        }
    };

    match process.kernel_stack_top() {
        Some(stack_top) if stack_top & 0xF == 0 => {}
        Some(_) => {
            crate::memory::serial_write("[PROCESS-KSTACK-TEST] unaligned stack top\r\n");
            return false;
        }
        None => {
            crate::memory::serial_write("[PROCESS-KSTACK-TEST] missing kernel stack\r\n");
            return false;
        }
    }

    crate::memory::serial_write("[PROCESS-KSTACK-TEST] stack ready\r\n");

    let ok = unsafe {
        if process.install_syscall_stack().is_err() {
            crate::memory::serial_write("[PROCESS-KSTACK-TEST] install failed\r\n");
            return false;
        }

        let ok = crate::syscall::run_userspace_syscall_smoke();
        Process::reset_syscall_stack_policy();
        ok
    };

    if !ok {
        crate::memory::serial_write("[PROCESS-KSTACK-TEST] syscall smoke failed\r\n");
        return false;
    }

    crate::memory::serial_write("[PROCESS-KSTACK-TEST] OK\r\n");
    true
}

const fn fault_kind_to_i32(fault: ProcessFault) -> i32 {
    match fault {
        ProcessFault::PageFault => 1,
        ProcessFault::GeneralProtection => 2,
        ProcessFault::InvalidOpcode => 3,
        ProcessFault::DivideByZero => 4,
        ProcessFault::Unknown => 5,
    }
}
