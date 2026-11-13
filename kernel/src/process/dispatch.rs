//! Dispatch owns hardware backing allocations; no Process borrow crosses IRET.
use super::*;

pub(super) struct KernelStack {
    bytes: Box<[UnsafeCell<u8>]>,
}
// Safe access exposes only an address. Actual stack use is the unsafe HAL's
// responsibility; the bytes may change while immutable ownership is shared.
unsafe impl Sync for KernelStack {}
impl KernelStack {
    pub(super) fn new() -> Self {
        let bytes = (0..DEFAULT_KERNEL_STACK_SIZE)
            .map(|_| UnsafeCell::new(0))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self { bytes }
    }
    pub(super) fn top(&self) -> usize {
        (self.bytes.as_ptr() as usize + self.bytes.len()) & !0xf
    }
}

/// Own the active root between dispatches without an extra kernel CR3 roundtrip
/// on each yield. Drop restores the caller on every return/error path.
pub(super) struct DispatchScope {
    previous_root: usize,
    previous_pid: u64,
    previous_stack: Option<Arc<KernelStack>>,
    _previous_space: Option<Arc<IrqSafeSpinLock<AddressSpace>>>,
    active: Option<Arc<IrqSafeSpinLock<AddressSpace>>>,
}
impl DispatchScope {
    pub(super) fn new() -> Self {
        let _irq = InterruptGuard::new();
        Self {
            previous_root: unsafe { crate::memory::vmm::active_root_frame() },
            previous_pid: crate::cpu::runtime().current_pid.load(Ordering::SeqCst),
            previous_stack: with_current_thread(|p| p.kernel_stack.clone()).flatten(),
            _previous_space: with_current_process(|p| p.address_space.clone()).flatten(),
            active: None,
        }
    }
    unsafe fn enter(&mut self, space: &Arc<IrqSafeSpinLock<AddressSpace>>) {
        let root = space.lock().root_frame().as_usize();
        // This also refreshes the canonical kernel half, even for the same
        // CR3. Do not bypass VMM synchronization with a raw equality shortcut.
        crate::memory::vmm::switch_to_root_frame(root);
        if !self
            .active
            .as_ref()
            .map(|old| Arc::ptr_eq(old, space))
            .unwrap_or(false)
        {
            self.active = Some(space.clone()); // retire old root only after switching
        }
    }
    pub(super) fn restore_root(&mut self) {
        unsafe {
            crate::memory::vmm::switch_to_root_frame(self.previous_root);
        }
        self.active = None;
    }
}
impl Drop for DispatchScope {
    fn drop(&mut self) {
        self.restore_root();
        crate::cpu::runtime()
            .current_pid
            .store(self.previous_pid, Ordering::SeqCst);
        unsafe {
            if let Some(stack) = &self.previous_stack {
                crate::hal::syscall_set_kernel_stack_top(stack.top() as u64);
            } else {
                Process::reset_syscall_stack_policy();
            }
        }
    }
}

pub(super) struct DispatchFrame {
    context: CpuContext,
    fs_base: u64,
    stack: Arc<KernelStack>,
    fpu: Arc<FpuState>,
    address_space: Arc<IrqSafeSpinLock<AddressSpace>>,
}
impl DispatchFrame {
    fn new(
        process: &mut Process,
        address_space: Arc<IrqSafeSpinLock<AddressSpace>>,
    ) -> Result<Self, ProcessError> {
        if process.is_kernel || process.context.rip == 0 || process.context.rsp == 0 {
            return Err(ProcessError::InvalidUserContext);
        }
        let stack = process
            .kernel_stack
            .clone()
            .ok_or(ProcessError::NoKernelStack)?;
        process.state = ProcessState::Running;
        Ok(Self {
            context: process.context,
            fs_base: process.fs_base,
            stack,
            fpu: process.fpu_state.clone(),
            address_space,
        })
    }

    /// Caller selects a single task on this CPU; parallel Running claims remain
    /// disabled until the process/scheduler ownership and TLB gates are complete.
    pub(super) unsafe fn run(&self, scope: &mut DispatchScope) {
        assert!(
            crate::sync::interrupts_enabled(),
            "IRQ masking held across userspace dispatch"
        );
        scope.enter(&self.address_space);
        crate::hal::syscall_set_kernel_stack_top(self.stack.top() as u64);
        crate::hal::set_user_fpu_state(self.fpu.0.get().cast::<u8>());
        crate::hal::set_fs_base(self.fs_base);
        crate::hal::run_user_context(&self.context as *const CpuContext);
        crate::hal::set_fs_base(0);
        crate::hal::set_user_fpu_state(core::ptr::null_mut());
        Process::reset_syscall_stack_policy();
        // Scope retains the active root after this frame is released. It
        // switches away before dropping the last root owner, including errors.
    }
}

pub(super) fn prepare(pid: ProcessId) -> Result<DispatchFrame, ProcessError> {
    let _irq = InterruptGuard::new();
    let owner = owner_of(pid).ok_or(ProcessError::NoSuchProcess)?;
    let address_space = process_address_space(owner)?;
    with_process_mut(pid, |process| DispatchFrame::new(process, address_space))
}

#[cfg(feature = "boot-smoke-tests")]
pub fn run_dispatch_pin_smoke() -> bool {
    fn check() -> Result<(), ProcessError> {
        let mut process = Process::new_user(allocate_pid())?;
        let page = process.map_anonymous(0, USER_PAGE_SIZE, true, false, false, true)?;
        process.context.rip = 0x200000;
        process.context.rsp = 0x7fffffffe000;
        let space = process
            .address_space
            .clone()
            .ok_or(ProcessError::NoAddressSpace)?;
        let root = space.lock().root_frame();
        let weak_space = Arc::downgrade(&space);
        let weak_stack = Arc::downgrade(
            process
                .kernel_stack
                .as_ref()
                .ok_or(ProcessError::NoKernelStack)?,
        );
        let weak_fpu = Arc::downgrade(&process.fpu_state);
        let frame = DispatchFrame::new(&mut process, space)?;
        drop(process);
        if weak_stack.upgrade().is_none()
            || weak_fpu.upgrade().is_none()
            || weak_space.upgrade().is_none()
        {
            return Err(ProcessError::NoSuchProcess);
        }
        {
            let space = frame.address_space.lock();
            if space.root_frame() != root
                || space
                    .translate_user_page(VirtualAddress(page))
                    .ok()
                    .flatten()
                    .is_none()
            {
                return Err(ProcessError::NoAddressSpace);
            }
        }
        // Exercise the real root-switch/drop path on an error return, without
        // entering synthetic user code. IRQ masking hides the temporary PID.
        let _irq = InterruptGuard::new();
        let previous_root = unsafe { crate::memory::vmm::active_root_frame() };
        let previous_pid = crate::cpu::runtime().current_pid.load(Ordering::SeqCst);
        fn abort_with_active_root(
            frame: DispatchFrame,
            weak_stack: &alloc::sync::Weak<KernelStack>,
            weak_fpu: &alloc::sync::Weak<FpuState>,
            weak_space: &alloc::sync::Weak<IrqSafeSpinLock<AddressSpace>>,
        ) -> Result<(), ProcessError> {
            let mut scope = DispatchScope::new();
            unsafe {
                scope.enter(&frame.address_space);
            }
            crate::cpu::runtime().current_pid.store(0, Ordering::SeqCst);
            drop(frame);
            if weak_stack.upgrade().is_some()
                || weak_fpu.upgrade().is_some()
                || weak_space.upgrade().is_none()
            {
                return Err(ProcessError::NoSuchProcess);
            }
            Err(ProcessError::NotRunnable) // Scope must restore before retiring the root.
        }
        if abort_with_active_root(frame, &weak_stack, &weak_fpu, &weak_space)
            != Err(ProcessError::NotRunnable)
            || unsafe { crate::memory::vmm::active_root_frame() } != previous_root
            || crate::cpu::runtime().current_pid.load(Ordering::SeqCst) != previous_pid
            || weak_space.upgrade().is_some()
        {
            return Err(ProcessError::NoSuchProcess);
        }
        Ok(())
    }
    if check().is_ok() {
        crate::memory::serial_write("[DISPATCH-PIN] stack/FPU/address-space survive Process drop and reclaim after frame OK\r\n");
        true
    } else {
        false
    }
}
