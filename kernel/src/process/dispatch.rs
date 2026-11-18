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
    _cpu_bound: core::marker::PhantomData<*mut ()>,
}
impl DispatchScope {
    pub(super) fn new() -> Self {
        let _irq = InterruptGuard::new();
        Self {
            previous_root: unsafe { crate::memory::vmm::active_root_frame() },
            previous_pid: crate::cpu::runtime().current_pid.load(Ordering::SeqCst),
            previous_stack: with_current_thread(|p| p.kernel_stack.clone()).flatten(),
            _previous_space: active_address_space_pin().or_else(||with_current_process(|p| p.address_space.clone()).flatten()),
            active: None,
            _cpu_bound: core::marker::PhantomData,
        }
    }
    unsafe fn enter(&mut self, space: &Arc<IrqSafeSpinLock<AddressSpace>>) {
        let _irq=InterruptGuard::new();
        let mapped=space.lock();
        let root = mapped.root_frame().as_usize();
        // This also refreshes the canonical kernel half, even for the same
        // CR3. Do not bypass VMM synchronization with a raw equality shortcut.
        crate::memory::vmm::switch_to_root_frame(root);
        drop(mapped);
        if !self
            .active
            .as_ref()
            .map(|old| Arc::ptr_eq(old, space))
            .unwrap_or(false)
        {
            self.active = Some(space.clone()); // retire old root only after switching
        }
        crate::cpu::runtime().active_space.store(Arc::as_ptr(space) as usize,Ordering::Release);
    }
    pub(super) fn restore_root(&mut self) {
        let _irq=InterruptGuard::new();
        let _previous_guard=self._previous_space.as_ref().map(|space|space.lock());
        unsafe {
            crate::memory::vmm::switch_to_root_frame(self.previous_root);
        }
        crate::cpu::runtime().active_space.store(self._previous_space.as_ref().map(|space|Arc::as_ptr(space) as usize).unwrap_or(0),Ordering::Release);
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
    lease: Option<execution::Lease>,
    fs_base: u64,
    stack: Arc<KernelStack>,
    fpu: Arc<FpuState>,
    address_space: Arc<IrqSafeSpinLock<AddressSpace>>,
}
impl DispatchFrame {
    fn new(
        process: &mut Process,
        address_space: &Arc<IrqSafeSpinLock<AddressSpace>>,
    ) -> Result<Self, ProcessError> {
        if process.is_kernel || process.context.rip == 0 || process.context.rsp == 0 {
            return Err(ProcessError::InvalidUserContext);
        }
        let stack = process
            .kernel_stack
            .clone()
            .ok_or(ProcessError::NoKernelStack)?;
        Ok(Self {
            context: process.context,
            lease: None,
            fs_base: process.fs_base,
            stack,
            fpu: process.fpu_state.clone(),
            address_space: address_space.clone(),
        })
    }

    /// Exclusive admission is revalidated immediately before touching hardware.
    pub(super) unsafe fn run(&mut self, scope: &mut DispatchScope) -> Result<(), ProcessError> {
        assert!(
            crate::sync::interrupts_enabled(),
            "IRQ masking held across userspace dispatch"
        );
        self.lease.as_mut().ok_or(ProcessError::NotRunnable)?.start()?;
        scope.enter(&self.address_space);
        crate::hal::syscall_set_kernel_stack_top(self.stack.top() as u64);
        crate::hal::set_user_fpu_state(self.fpu.0.get().cast::<u8>());
        crate::hal::set_fs_base(self.fs_base);
        crate::hal::run_user_context(&self.context as *const CpuContext);
        crate::hal::set_fs_base(0);
        crate::hal::set_user_fpu_state(core::ptr::null_mut());
        Process::reset_syscall_stack_policy();
        self.lease.as_mut().expect("dispatch lease").returned();
        crate::cpu::reschedule::returned();
        // Scope retains the active root after this frame is released. It
        // switches away before dropping the last root owner, including errors.
        Ok(())
    }
}

pub(super) fn prepare(pid: ProcessId) -> Result<DispatchFrame, ProcessError> {
    let _irq = InterruptGuard::new();
    let owner = owner_of(pid).ok_or(ProcessError::NoSuchProcess)?;
    let address_space = process_address_space(owner)?;
    with_task_record(pid, |process, record| {
        if record.state != ProcessState::Ready { return Err(ProcessError::NotRunnable); }
        let mut frame = DispatchFrame::new(process, &address_space)?;
        frame.lease = Some(execution::Lease::reserve(pid, record)?);
        Ok(frame)
    })
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
        let frame = DispatchFrame::new(&mut process, &space)?;
        drop(space);
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
        let previous_space_pointer=crate::cpu::runtime().active_space.load(Ordering::Acquire);
        fn abort_with_active_root(
            frame: DispatchFrame,
            page:usize,
            weak_stack: &alloc::sync::Weak<KernelStack>,
            weak_fpu: &alloc::sync::Weak<FpuState>,
            weak_space: &alloc::sync::Weak<IrqSafeSpinLock<AddressSpace>>,
        ) -> Result<(), ProcessError> {
            let mut scope = DispatchScope::new();
            unsafe {
                scope.enter(&frame.address_space);
            }
            let caller=ProcessId(crate::cpu::runtime().current_pid.load(Ordering::SeqCst));
            crate::cpu::runtime().current_pid.store(0, Ordering::SeqCst);
            drop(frame);
            if weak_stack.upgrade().is_some()
                || weak_fpu.upgrade().is_some()
                || weak_space.upgrade().is_none()
            {
                return Err(ProcessError::NoSuchProcess);
            }
            // Task lookup under this callback would deadlock on the held task
            // mutex. The CPU-owned AS lease must suffice even after Frame drop.
            with_process_mut(caller,|_| {
                crate::syscall::copy_buffer_to_user(page as *mut u8,b"mapping-pin").map_err(|_|ProcessError::InvalidMemoryRange)?;
                let data=crate::syscall::copy_buffer_from_user(page as *const u8,11).map_err(|_|ProcessError::InvalidMemoryRange)?;
                if data.as_slice()!=b"mapping-pin" {return Err(ProcessError::InvalidMemoryRange);}
                Ok(())
            })?;
            Err(ProcessError::NotRunnable) // Scope must restore before retiring the root.
        }
        if abort_with_active_root(frame, page, &weak_stack, &weak_fpu, &weak_space)
            != Err(ProcessError::NotRunnable)
            || unsafe { crate::memory::vmm::active_root_frame() } != previous_root
            || crate::cpu::runtime().current_pid.load(Ordering::SeqCst) != previous_pid
            || weak_space.upgrade().is_some()
            || crate::cpu::runtime().active_space.load(Ordering::Acquire)!=previous_space_pointer
        {
            return Err(ProcessError::NoSuchProcess);
        }
        Ok(())
    }
    if check().is_ok() {
        crate::memory::serial_write("[DISPATCH-PIN] stack/FPU/address-space survive Process drop and reclaim after frame OK\r\n");
        crate::memory::serial_write("[USER-MAPPING] source pin and task-scope copy survive object drop OK\r\n");
        true
    } else {
        false
    }
}

/// The CPU-bound dispatch scope owns this Arc until root/pointer restoration.
/// IRQ masking makes selection/refcount acquisition indivisible on this CPU;
/// no task lookup is needed, so user copy can run inside an owner task scope.
pub(crate) fn active_address_space_pin()->Option<Arc<IrqSafeSpinLock<AddressSpace>>> {
    let _irq=InterruptGuard::new();
    let ptr=crate::cpu::runtime().active_space.load(Ordering::Acquire) as *const IrqSafeSpinLock<AddressSpace>;
    if ptr.is_null() {return None;}
    unsafe {Arc::increment_strong_count(ptr);Some(Arc::from_raw(ptr))}
}
