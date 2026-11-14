//! Actual task/queue wake identity; host tests force the remote admission gap.
use super::*;

pub fn run_parking_smoke() -> bool {
    fn check() -> Result<(), ProcessError> {
        let pid = allocate_pid();
        let mut process = Process::new_user(pid)?;
        process.context.rip = 0x200000;
        process.context.rsp = 0x7fffffffe000;
        insert_process_record(
            pid,
            None,
            String::from("[parking-proof]"),
            ProcessState::Ready,
            false,
            false,
            Some(process),
        );
        let entry = WaitEntry {
            entity: pid,
            key: WaitKey::Pipe(u64::MAX),
            deadline: None,
        };
        let install = || {
            with_task_record(pid, |process, record| {
                process.state = ProcessState::Blocked;
                record.state = process.state;
                let generation = WAIT_QUEUE.lock().push(entry);
                record.wait_generation = Some(generation);
                Ok(generation)
            })
        };
        install()?;
        let mut delayed = None;
        wake_matching(
            &WAIT_QUEUE,
            |entry| entry.entity == pid,
            1,
            |entry, generation| delayed = Some((entry, generation)),
        );
        let fresh = install()?;
        let (old, generation) = delayed.ok_or(ProcessError::NotRunnable)?;
        wake_entry(old, generation);
        let unchanged = with_task_record(pid, |_, record| {
            Ok(record.state == ProcessState::Blocked && record.wait_generation == Some(fresh))
        })?;
        if !unchanged {
            return Err(ProcessError::InvalidUserContext);
        }
        wake_pipe_waiters(u64::MAX);
        if !is_pid_runnable(pid) {
            return Err(ProcessError::NotRunnable);
        }
        let checkpoint = wait_checkpoint();
        wake_pipe_waiters(u64::MAX); // no waiter: the winning notification must persist
        if !WAIT_QUEUE.lock().changed(checkpoint) {
            return Err(ProcessError::InvalidUserContext);
        }
        autoreap_process(pid, "parking-proof-done")?;
        Ok(())
    }
    if check().is_ok() {
        crate::memory::serial_write(
            "[PARKING] checkpoint, stale wake and registration identity OK\r\n",
        );
        true
    } else {
        crate::memory::serial_write("[PARKING] FAIL\r\n");
        false
    }
}
