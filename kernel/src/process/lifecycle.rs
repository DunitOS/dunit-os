//! Registry membership, task lifecycle and current selectors.
use super::*;

pub fn allocate_pid() -> ProcessId {
    ProcessId(NEXT_PID.fetch_add(1, Ordering::SeqCst))
}

/// Acquire the owning task independently of metadata; return only an AS owner.
pub(super) fn process_address_space(pid: ProcessId) -> Result<Arc<IrqSafeSpinLock<AddressSpace>>, ProcessError> {
    current_access::with_process(pid, |process| process.address_space.clone())?
        .ok_or(ProcessError::NoAddressSpace)
}

pub(super) fn process_record_index(table: &[ProcessRecord], pid: ProcessId) -> Option<usize> {
    table.iter().position(|record| record.pid == pid)
}

pub(super) fn log_process_transition(pid: ProcessId, from: ProcessState, to: ProcessState, reason: &str) {
    if reason == "yield" || reason == "enter-user" {
        return;
    }
    if !crate::serial::debug_trace_enabled() {
        return;
    }
    crate::memory::serial_write("[PROCESS] pid=");
    serial_write_u64(pid.0);
    crate::memory::serial_write(" state=");
    serial_write_state(from);
    crate::memory::serial_write("->");
    serial_write_state(to);
    if !reason.is_empty() {
        crate::memory::serial_write(" reason=");
        crate::memory::serial_write(reason);
    }
    crate::memory::serial_write("\r\n");
}

pub(super) fn current_entity_id() -> Option<ProcessId> {
    let pid = crate::cpu::runtime().current_pid.load(Ordering::SeqCst);
    if pid == 0 {
        None
    } else {
        Some(ProcessId(pid))
    }
}

pub(super) fn owner_of(entity: ProcessId) -> Option<ProcessId> {
    // Короткое чтение таблицы на горячем пути планировщика: маскируем IRQ, чтобы
    // таймер (wake_expired_waiters) не взял &mut на тот же Vec во время нашего
    // разделяемого заимствования.
    let _irq = InterruptGuard::new();
    let table = process_table();
    let record = table.get(process_record_index(&table, entity)?)?;
    Some(record.owner.unwrap_or(entity))
}

pub(super) fn current_pid() -> Option<ProcessId> {
    owner_of(current_entity_id()?)
}

pub fn current_tid() -> Option<ThreadId> {
    current_entity_id().map(|id| ThreadId(id.0))
}

pub fn current_thread_pointer() -> Option<u64> {
    with_current_thread(|thread| thread.fs_base)
}

pub fn set_current_thread_pointer(base: u64) -> Result<(), ProcessError> {
    with_current_thread_mut(|thread| thread.set_thread_pointer(base))
        .ok_or(ProcessError::NoCurrentProcess)?
}

pub fn set_terminal_foreground_process(pid: Option<ProcessId>) {
    set_foreground_process(pid, ProcessOutputSink::Terminal);
}

pub fn set_foreground_process(pid: Option<ProcessId>, sink: ProcessOutputSink) {
    let mut terminal = TERMINAL_STATE.lock();
    let sink = if pid.is_some() { sink } else { ProcessOutputSink::SerialOnly };
    if terminal.foreground != pid || terminal.sink != sink || pid.is_none() {
        terminal.stdin.clear();
    }
    terminal.foreground = pid;
    terminal.sink = sink;
}

fn is_foreground_descendant(mut pid: ProcessId, foreground: ProcessId) -> bool {
    loop {
        if pid == foreground { return true; }
        let parent = {
            let table = process_table();
            process_record_index(&table, pid).and_then(|index| table[index].parent)
        };
        let Some(parent) = parent else { return false; };
        pid = parent;
    }
}

pub fn current_process_output_sink() -> Option<ProcessOutputSink> {
    let pid = current_pid()?;
    let (foreground, sink) = {
        let terminal = TERMINAL_STATE.lock();
        (terminal.foreground?, terminal.sink)
    };
    is_foreground_descendant(pid, foreground).then_some(sink)
}

pub fn current_process_is_terminal_foreground() -> bool {
    current_process_output_sink() == Some(ProcessOutputSink::Terminal)
}

pub fn request_terminal_stdin_for_current() -> Result<(), ProcessError> {
    let pid = current_pid().ok_or(ProcessError::NoCurrentProcess)?;
    let foreground = {
        let terminal = TERMINAL_STATE.lock();
        if terminal.sink != ProcessOutputSink::Terminal {
            return Err(ProcessError::NoCurrentProcess);
        }
        terminal.foreground.ok_or(ProcessError::NoCurrentProcess)?
    };
    if !is_foreground_descendant(pid, foreground) {
        return Err(ProcessError::NoCurrentProcess);
    }
    let mut terminal = TERMINAL_STATE.lock();
    // Registry ancestry is checked without the terminal lock. Revalidate the
    // selection before publishing a request so a foreground switch wins.
    if terminal.foreground != Some(foreground) || terminal.sink != ProcessOutputSink::Terminal {
        return Err(ProcessError::NoCurrentProcess);
    }
    terminal.stdin.request(pid.0);
    Ok(())
}

pub fn terminal_stdin_waiting_pid() -> Option<ProcessId> {
    TERMINAL_STATE.lock().stdin.waiting_pid().map(ProcessId)
}

pub fn provide_terminal_stdin(pid: ProcessId, data: &[u8]) -> Result<(), ProcessError> {
    let supplied = TERMINAL_STATE.lock().stdin.provide(pid.0, data);
    if !supplied { return Err(ProcessError::NoSuchProcess); }
    wake_poll_waiters();
    Ok(())
}

pub fn take_terminal_stdin_for_current(out: &mut [u8]) -> Result<Option<usize>, ProcessError> {
    let pid = current_pid().ok_or(ProcessError::NoCurrentProcess)?;
    Ok(TERMINAL_STATE.lock().stdin.take(pid.0, out))
}

pub fn get_process_snapshots() -> Vec<ProcessSnapshot> {
    let mut snapshots = Vec::new();
    let table = process_table();
    for record in table.iter() {
        if record.owner.is_some() {
            continue;
        }
        snapshots.push(ProcessSnapshot {
            pid: record.pid,
            parent: record.parent,
            state: effective_process_state(&table, record),
            status: record.status,
            has_run: record.has_run,
            waitable: record.waitable,
            path: record.path.clone(),
        });
    }
    snapshots
}

pub(super) fn effective_process_state(table: &[ProcessRecord], process: &ProcessRecord) -> ProcessState {
    if process.state == ProcessState::Ready
        && table.iter().any(|record| {
            record.owner == Some(process.pid) && record.state == ProcessState::Running
        })
    {
        ProcessState::Running
    } else {
        process.state
    }
}

pub fn process_stats() -> ProcessStats {
    let table = process_table();
    let mut stats = ProcessStats::default();
    for record in table.iter() {
        if record.owner.is_some() {
            continue;
        }
        stats.total += 1;
        match effective_process_state(&table, record) {
            ProcessState::Prepared => stats.prepared += 1,
            ProcessState::Ready => stats.ready += 1,
            ProcessState::Running => stats.running += 1,
            ProcessState::Blocked => stats.blocked += 1,
            ProcessState::Dead => stats.dead += 1,
            ProcessState::Reaped => stats.reaped += 1,
        }
    }
    stats
}

pub fn insert_process_record(
    pid: ProcessId,
    parent: Option<ProcessId>,
    path: String,
    state: ProcessState,
    waitable: bool,
    has_run: bool,
    process: Option<Process>,
) {
    // Object lifetime is independent of registry membership and Vec growth.
    // The retired Arc is always released after the metadata guard.
    let process = process.map(|process| Arc::new(IrqSafeSpinLock::new(process)));
    let retired = {
        let _irq = InterruptGuard::new();
        let mut table = process_table();
        if let Some(index) = process_record_index(&table, pid) {
            let record = &mut table[index];
            assert!(!record.execution.retirement_blocked(), "replace entered/finalizing task");
            record.execution = execution::State::default();
            record.wait_generation = None;
            record.parent = parent;
            record.owner = None;
            record.path = path;
            record.state = state;
            record.waitable = waitable;
            record.has_run = has_run;
            record.status = None;
            core::mem::replace(&mut record.process, process)
        } else {
            table.push(ProcessRecord { pid, owner: None, thread_auto_reap: false,
                parent, state, status: None, has_run, waitable, path, process,
                execution: execution::State::default(),
                wait_generation: None });
            None
        }
    };
    drop(retired);
}

pub fn create_user_process_record(
    path: String,
    waitable: bool,
    privileged: bool,
) -> Result<ProcessId, ProcessError> {
    let pid = allocate_pid();
    let parent = current_pid();
    let mut process = Process::new_user(pid)?;
    process.state = ProcessState::Prepared;
    process.privileged = privileged;
    if let Some(cwd) = with_current_process(|current| current.cwd.clone()) {
        process.cwd = cwd;
    }

    insert_process_record(
        pid,
        parent,
        path.clone(),
        ProcessState::Prepared,
        waitable,
        false,
        Some(process),
    );

    if crate::serial::debug_trace_enabled() {
        crate::memory::serial_write("[PROCESS] pid=");
        serial_write_u64(pid.0);
        crate::memory::serial_write(" parent=");
        match parent {
            Some(parent_pid) => serial_write_u64(parent_pid.0),
            None => crate::memory::serial_write("none"),
        }
        crate::memory::serial_write(" state=prepared path=");
        crate::memory::serial_write(&path);
        crate::memory::serial_write("\r\n");
    }

    Ok(pid)
}

/// Whether the current process holds the privileged (system-control) capability.
pub fn current_is_privileged() -> bool {
    with_current_process(|process| process.privileged).unwrap_or(false)
}

/// Authorization for killing `target`: a privileged process (init and the
/// system service tree, the kernel console) may signal any process; an
/// unprivileged one may signal only its own direct children. Closes the hole
/// where any process could terminate gui_server, the shell or a sibling.
pub fn current_may_kill(target: ProcessId) -> bool {
    if current_is_privileged() {
        return true;
    }
    let Some(caller) = current_pid() else {
        return false;
    };
    let table = process_table();
    process_record_index(&table, target)
        .map(|index| table[index].parent == Some(caller))
        .unwrap_or(false)
}
pub fn create_user_thread(entry: u64, stack_top: u64, arg: u64) -> Result<ThreadId, ProcessError> {
    let owner = current_pid().ok_or(ProcessError::NoCurrentProcess)?;
    if with_current_process(|p| p.is_kernel).unwrap_or(true) {
        return Err(ProcessError::InvalidUserContext);
    }
    let tid = ThreadId(allocate_pid().0);
    let (tls_base, tls_length, thread_pointer) = with_current_process_mut(|process| process.allocate_tls_instance())
        .ok_or(ProcessError::NoCurrentProcess)??;
    let mut thread = Process::new_user_thread(tid, entry, stack_top, arg);
    thread.fs_base = thread_pointer as u64;
    if tls_length != 0 {
        thread.tls_allocation = Some((tls_base, tls_length));
    }
    {
        let _irq = InterruptGuard::new();
        let mut table = process_table();
        let owner_index = process_record_index(&table, owner).ok_or(ProcessError::NoSuchProcess)?;
        if matches!(table[owner_index].state, ProcessState::Dead | ProcessState::Reaped) {
            return Err(ProcessError::NotRunnable);
        }
        table.push(ProcessRecord {
            pid: ProcessId(tid.0),
            owner: Some(owner),
            thread_auto_reap: false,
            parent: None,
            state: ProcessState::Ready,
            status: None,
            has_run: false,
            waitable: false,
            path: String::from("[thread]"),
            process: Some(Arc::new(IrqSafeSpinLock::new(thread))),
            execution: execution::State::default(),
            wait_generation: None,
        });
    }
    if let Err(error) = crate::process::scheduler::enqueue_ready(ProcessId(tid.0)) {
        let retired = {
            let _irq = InterruptGuard::new();
            let mut table = process_table();
            process_record_index(&table, ProcessId(tid.0)).map(|index| table.remove(index))
        };
        drop(retired);
        if tls_length != 0 {
            let _ = with_current_process_mut(|owner| owner.release_tls_allocation(tls_base, tls_length));
        }
        return Err(error);
    }
    crate::memory::serial_write("[THREAD] ready tid=");
    serial_write_u64(tid.0);
    crate::memory::serial_write(" pid=");
    serial_write_u64(owner.0);
    crate::memory::serial_write("\r\n");
    Ok(tid)
}

/// Opt-in is only valid for an additional thread; called before ThreadExit.
pub fn request_thread_auto_reap() -> Result<(), ProcessError> {
    let entity = current_entity_id().ok_or(ProcessError::NoCurrentProcess)?;
    let _irq = InterruptGuard::new();
    let mut table = process_table();
    let index = process_record_index(&table, entity).ok_or(ProcessError::NoSuchProcess)?;
    if table[index].owner.is_none() { return Err(ProcessError::InvalidUserContext); }
    table[index].thread_auto_reap = true;
    Ok(())
}

/// HAL has returned and the syscall stack is reset: the dead thread's stack
/// can now be freed. Keep only the join status unless exit explicitly detached.
pub(super) fn release_finished_thread(entity: ProcessId) -> Result<(), ProcessError> {
    let _irq = InterruptGuard::new();
    let owner = {
        let table = process_table();
        let index = process_record_index(&table, entity).ok_or(ProcessError::NoSuchProcess)?;
        table[index].owner.ok_or(ProcessError::InvalidUserContext)?
    };
    let tls = with_process_mut(entity, |thread| Ok(thread.tls_allocation))?;
    if let Some((base, length)) = tls {
        with_process_mut(owner, |process| process.release_tls_allocation(base, length))?;
        with_process_mut(entity, |thread| { thread.tls_allocation = None; Ok(()) })?;
    }
    let finished = {
        let mut table = process_table();
        let index = process_record_index(&table, entity).ok_or(ProcessError::NoSuchProcess)?;
        if !table[index].execution.finalizing() || !table[index].execution.retire_returned(crate::cpu::index()) {
            return Err(ProcessError::NotRunnable);
        }
        table[index].process.take()
    };
    drop(finished);
    Ok(())
}

pub fn wait_thread(tid: ThreadId) -> Result<ProcessExitStatus, ProcessError> {
    let _irq = InterruptGuard::new();
    let owner = current_pid().ok_or(ProcessError::NoCurrentProcess)?;
    if current_tid() == Some(tid) { return Err(ProcessError::NotChild); }
    let (retired, status) = {
        let mut table = process_table();
        let index = process_record_index(&table, ProcessId(tid.0)).ok_or(ProcessError::NoSuchProcess)?;
        let record = &table[index];
        if record.owner != Some(owner) { return Err(ProcessError::NotChild); }
        if record.state != ProcessState::Dead || record.execution.busy() { return Err(ProcessError::NotRunnable); }
        let status = record.status.ok_or(ProcessError::ProcessNotPrepared)?;
        (table.remove(index), status)
    };
    let tls = retired.process.as_ref().and_then(|task| task.lock().tls_allocation);
    scheduler::remove(ProcessId(tid.0));
    drop(retired);
    if let Some((base, length)) = tls {
        if let Some(result) = with_current_process_mut(|owner| owner.release_tls_allocation(base, length)) { result?; }
    }
    Ok(status)
}

pub(super) fn cleanup_owned_threads(owner: ProcessId) -> Result<(), ProcessError> {
    let retired = {
        let _irq = InterruptGuard::new();
        let mut table = process_table();
        if table.iter().any(|record| record.owner == Some(owner) && record.execution.retirement_blocked()) {
            return Err(ProcessError::NotRunnable);
        }
        let mut retired = Vec::new();
        let mut index = 0;
        while index < table.len() {
            if table[index].owner == Some(owner) { retired.push(table.remove(index)); }
            else { index += 1; }
        }
        retired
    };
    for record in retired {
        remove_waiter(record.pid);
        crate::process::scheduler::remove(record.pid);
        drop(record);
    }
    Ok(())
}

pub fn wake_ipc_waiters(pid: ProcessId) {
    wake_wait_set_waiters();
    let _irq = InterruptGuard::new();
    wake_matching(&WAIT_QUEUE, |entry| entry.key == WaitKey::Ipc(pid), 0, wake_entry);
}

pub fn wake_poll_waiters() {
    let _irq = InterruptGuard::new();
    wake_matching(&WAIT_QUEUE, |entry| matches!(entry.key, WaitKey::Poll | WaitKey::WaitSet), 0, wake_entry);
}
/// Event-driven rescan of call-scoped sets. Unrelated producers may cause a
/// spurious rescan; the original snapshot/deadline is preserved across restart.
pub(crate) fn wake_wait_set_waiters() {
    let _irq = InterruptGuard::new();
    wake_matching(&WAIT_QUEUE, |entry| entry.key == WaitKey::WaitSet, 0, wake_entry);
}
pub fn terminal_input_ready() -> bool {
    let Some(pid) = current_pid() else { return false; };
    TERMINAL_STATE.lock().stdin.is_ready(pid.0)
}

/// Wake every process blocked on the pipe end `id`.
pub fn wake_pipe_waiters(id: u64) {
    wake_poll_waiters();
    let _irq = InterruptGuard::new();
    wake_matching(&WAIT_QUEUE, |entry| entry.key == WaitKey::Pipe(id), 0, wake_entry);
}

pub fn wake_expired_waiters() {
    let _irq = InterruptGuard::new();
    let now = crate::clock::monotonic_ticks();
    wake_matching(&WAIT_QUEUE, |entry| {
        entry.deadline.map(|deadline| now >= deadline).unwrap_or(false)
    }, 0, wake_entry);
}

pub fn is_pid_blocked(pid: ProcessId) -> bool {
    // Короткое чтение таблицы: маскируем IRQ, чтобы таймер не мутировал Vec под нами.
    let _irq = InterruptGuard::new();
    process_table()
        .iter()
        .any(|record| record.pid == pid && record.state == ProcessState::Blocked)
}

/// Foreground retry predicate, not execution admission. Observe both states
/// together so an IRQ wake from Blocked to Ready cannot fall between queries.
pub(crate) fn is_pid_waiting_for_dispatch(pid: ProcessId) -> bool {
    let table = process_table();
    process_record_index(&table, pid)
        .map(|index| {
            let record = &table[index];
            record.process.is_some()
                && matches!(record.state, ProcessState::Ready | ProcessState::Blocked)
        })
        .unwrap_or(false)
}

pub(super) fn remove_waiter(entity: ProcessId) {
    let _irq = InterruptGuard::new();
    let pending = with_task_record(entity, |process, record| {
        WAIT_QUEUE.lock().remove_entity(entity);
        record.wait_generation = None;
        Ok(process.wait_set.take())
    }).ok().flatten();
    // Detached records can leave registrations, but no live replacement shares
    // an allocated PID. The normal path above serializes cancel with admission.
    if registry::task(entity).is_err() { WAIT_QUEUE.lock().remove_entity(entity); }
    drop(pending); // readiness/file leases may reenter after the task lock ends
}

pub fn reap_process(pid: ProcessId) -> Result<(), ProcessError> {
    remove_waiter(pid);
    let (from, retired) = {
        let _irq = InterruptGuard::new();
        let mut table = process_table();
        let index = process_record_index(&table, pid).ok_or(ProcessError::NoSuchProcess)?;
        let record = &mut table[index];
        if record.state != ProcessState::Dead || record.execution.busy() { return Err(ProcessError::NotRunnable); }
        if execution::group_entered(&table, pid) { return Err(ProcessError::NotRunnable); }
        let record = &mut table[index];
        let from = record.state;
        let retired = record.process.take();
        record.state = ProcessState::Reaped;
        record.waitable = false;
        (from, retired)
    };
    crate::process::scheduler::remove(pid);
    if let Some(mut ipc) = crate::ipc::get_ipc_manager() { ipc.clear_messages(pid); }
    drop(retired);
    log_process_transition(pid, from, ProcessState::Reaped, "reap");
    Ok(())
}

pub fn process_exists(pid: ProcessId) -> bool {
    let table = process_table();
    process_record_index(&table, pid)
        .map(|index| table[index].owner.is_none())
        .unwrap_or(false)
}

pub fn thread_count_for_pid(owner: ProcessId) -> usize {
    process_table().iter().filter(|record| record.owner == Some(owner)).count()
}

/// Terminates a non-kernel process and leaves a waitable exit status for its
/// parent. The current process uses the normal syscall escape path; a prepared
/// or ready child can be marked dead without ever entering userspace.
pub fn kill_process(pid: ProcessId, code: i32) -> Result<bool, ProcessError> {
    if current_pid() == Some(pid) {
        // A secondary thread cannot synchronously destroy the owner address
        // space while still executing on it. Process-wide self-exit is added
        // with the later lifecycle/signal contract.
        if current_entity_id() != Some(pid) {
            return Err(ProcessError::InvalidUserContext);
        }
        let is_kernel = with_current_process(|process| process.is_kernel)
            .ok_or(ProcessError::NoCurrentProcess)?;
        if is_kernel {
            return Err(ProcessError::InvalidUserContext);
        }
        crate::cpu::runtime().exit_code.store(code, Ordering::SeqCst);
        crate::cpu::runtime().exit_kind.store(0, Ordering::SeqCst);
        crate::cpu::runtime().exit_requested.store(true, Ordering::SeqCst);
        return Ok(true);
    }

    let exit = ProcessExit { pid, status: ProcessExitStatus::Exited(code) };
    if let Err(err) = finalization::begin(exit, false) {
        if err == ProcessError::NotRunnable {
            let cpu_mask = {
                let _irq = InterruptGuard::new();
                let mut table = process_table();
                execution::arm_group_cancel(&mut table, pid, code)
            };
            for cpu in 0..64 {
                if (cpu_mask & (1u64 << cpu)) != 0 {
                    let _ = crate::cpu::reschedule::request(cpu);
                }
            }
        }
        return Err(err);
    }
    let (fds, vm) = with_process_mut(pid, |process| {
        Ok((process.take_fd_resources(), process.take_vm_resources()))
    })?;
    if let Some(mut ipc) = crate::ipc::get_ipc_manager() { ipc.clear_messages(pid); }
    fds.release();
    cleanup_owned_threads(pid)?;
    drop(vm);
    finalization::publish(exit)?;
    Ok(false)
}

/// An owned observation; execution ownership is a separate scheduler gate.
pub fn is_pid_runnable(pid: ProcessId) -> bool {
    let _irq = InterruptGuard::new();
    let (task, owner) = {
        let table = process_table();
        let Some(index) = process_record_index(&table, pid) else { return false; };
        let record = &table[index];
        if record.state != ProcessState::Ready || record.execution.busy() { return false; }
        let Some(task) = record.process.clone() else { return false; };
        let owner = if let Some(owner) = record.owner {
            process_record_index(&table, owner).and_then(|index| table[index].process.clone())
        } else { None };
        (task, owner)
    };
    let (context_ready, own_space) = {
        let process = task.lock();
        (!process.is_kernel && process.context.rip != 0 && process.context.rsp != 0
            && process.kernel_stack_top().is_some(), process.address_space.is_some())
    };
    if !context_ready { return false; }
    // Never hold two task locks (thread -> owner would permit ABBA).
    let has_space = owner.map(|owner| owner.lock().address_space.is_some()).unwrap_or(own_space);
    if !has_space { return false; }
    let table = process_table();
    process_record_index(&table, pid).map(|index| {
        table[index].state == ProcessState::Ready && !table[index].execution.busy() && registry::verify(&table[index], &task).is_ok()
    }).unwrap_or(false)
}

/// Non-consuming observation, with exactly the existing child-wait authority.
pub(crate) fn child_exit_snapshot(pid: ProcessId) -> Result<Option<WaitRecord>, ProcessError> {
    let parent = with_current_process(|process| process.pid).ok_or(ProcessError::NoCurrentProcess)?;
    let table = process_table();
    let record = table.iter().find(|record| record.pid == pid && record.parent == Some(parent)
        && record.waitable && record.owner.is_none()).ok_or(ProcessError::NotChild)?;
    Ok(record.status.filter(|_| record.has_run && record.state == ProcessState::Dead
        && !record.execution.busy() && !execution::group_entered(&table, pid)).map(|status| WaitRecord {
        pid, kind: status.kind_code(), code: status.exit_code(),
    }))
}

pub fn wait_for_child(requested_pid: ProcessId) -> Result<WaitRecord, ProcessError> {
    let parent_pid = with_current_process(|process| process.pid).ok_or(ProcessError::NoCurrentProcess)?;
    let (mut record, kind, code) = {
        let _irq = InterruptGuard::new();
        let mut table = process_table();
        // Select a child that has already finished so a terminated child is reaped
        // even when an earlier-listed sibling is still running (POSIX wait(-1)
        // "any terminated child"). Report EAGAIN only when a matching child exists
        // but none has finished; ECHILD (NotChild) when there is no matching child
        // at all — including a specific pid that is not our child, which POSIX
        // requires to be ECHILD rather than ENOENT.
        let mut child_index = None;
        let mut matched_any = false;
        for (index, record) in table.iter().enumerate() {
            if record.parent != Some(parent_pid) || !record.waitable {
                continue;
            }
            if requested_pid.0 != 0 && record.pid != requested_pid {
                continue;
            }
            matched_any = true;
            let finished = !record.execution.busy() && !execution::group_entered(&table, record.pid)
                && ((record.status.is_some() && record.has_run)
                || (record.state == ProcessState::Prepared && !record.has_run));
            if finished {
                child_index = Some(index);
                break;
            }
        }

        let index = match child_index {
            Some(index) => index,
            None if matched_any => return Err(ProcessError::NotRunnable),
            None => return Err(ProcessError::NotChild),
        };

        let record = &table[index];
        let (kind, code) = match record.status {
            Some(status) if record.has_run => (status.kind_code(), status.exit_code()),
            _ if record.state == ProcessState::Prepared && !record.has_run => {
                (WAIT_KIND_SPAWN_PREPARED, 0)
            }
            _ => return Err(ProcessError::NotRunnable),
        };

        (table.remove(index), kind, code)
    };
    let pid = record.pid;
    log_process_transition(pid, record.state, ProcessState::Reaped, "wait");
    crate::process::scheduler::remove(pid);
    if let Some(mut ipc) = crate::ipc::get_ipc_manager() { ipc.clear_messages(pid); }
    if let Some(task) = record.process.take() {
        let fds = { task.lock().take_fd_resources() };
        let closed = fds.release();
        if closed > 0 {
            crate::memory::serial_write("[PROCESS] reaped pid=");
            serial_write_u64(pid.0);
            crate::memory::serial_write(" cleaned-fds=");
            serial_write_usize(closed);
            crate::memory::serial_write("\r\n");
        }
    }

    wake_wait_set_waiters();
    Ok(WaitRecord { pid, kind, code })
}

pub fn cleanup_prepared_children(parent_pid: ProcessId) -> usize {
    let retired = {
        let _irq = InterruptGuard::new();
        let mut table = process_table();
        let mut retired = Vec::new();
        let mut index = 0;
        while index < table.len() {
            if table[index].parent == Some(parent_pid) && !table[index].has_run
                && table[index].process.is_some() { retired.push(table.remove(index)); }
            else { index += 1; }
        }
        retired
    };
    let removed = retired.len();
    for record in retired {
        log_process_transition(record.pid, record.state, ProcessState::Reaped, "parent-exit");
        crate::process::scheduler::remove(record.pid);
        if let Some(mut ipc) = crate::ipc::get_ipc_manager() { ipc.clear_messages(record.pid); }
        drop(record);
    }
    removed
}

pub fn autoreap_process(pid: ProcessId, reason: &str) -> Result<(), ProcessError> {
    let record = {
        let _irq = InterruptGuard::new();
        let mut table = process_table();
        let index = process_record_index(&table, pid).ok_or(ProcessError::NoSuchProcess)?;
        let owner = table[index].owner.unwrap_or(pid);
        if execution::group_entered(&table, owner) { return Err(ProcessError::NotRunnable); }
        table.remove(index)
    };
    log_process_transition(pid, record.state, ProcessState::Reaped, reason);
    crate::process::scheduler::remove(pid);
    if let Some(mut ipc) = crate::ipc::get_ipc_manager() { ipc.clear_messages(pid); }
    drop(record);
    Ok(())
}

pub fn snapshot_processes(out: &mut Vec<ProcessSnapshot>) {
    out.clear();
    let table = process_table();
    for record in table.iter() {
        if record.owner.is_some() {
            continue;
        }
        out.push(ProcessSnapshot {
            pid: record.pid,
            parent: record.parent,
            state: effective_process_state(&table, record),
            status: record.status,
            has_run: record.has_run,
            waitable: record.waitable,
            path: record.path.clone(),
        });
    }
}

pub fn init_current_kernel_process() {
    if crate::cpu::runtime().current_pid.load(Ordering::SeqCst) != 0 {
        return;
    }

    let pid = allocate_pid();
    let mut process = Process::new_kernel(pid);
    process.state = ProcessState::Running;
    insert_process_record(
        pid,
        None,
        String::from("kernel"),
        ProcessState::Running,
        false,
        true,
        Some(process),
    );
    crate::cpu::runtime().current_pid.store(pid.0, Ordering::SeqCst);
}
