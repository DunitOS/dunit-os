//! Exclusive execution admission; IRQ/task guards never cross HAL.
use super::*;
use core::marker::PhantomData;

#[path = "execution_state.rs"]
mod state;
pub(super) use state::{State, Ticket};

pub(super) fn group_entered(table: &[ProcessRecord], owner: ProcessId) -> bool {
    table.iter().any(|record| {
        (record.pid == owner || record.owner == Some(owner)) && record.execution.entered()
    })
}

/// The Arc pins object identity, the ticket grants a single CPU admission.
/// A lease cannot migrate, even before it has entered HAL.
pub(super) struct Lease {
    pid: ProcessId,
    task: registry::Task,
    ticket: Ticket,
    _cpu_bound: PhantomData<*mut ()>,
}
impl Lease {
    pub(super) fn reserve(
        pid: ProcessId,
        record: &mut ProcessRecord,
    ) -> Result<Self, ProcessError> {
        if record.state != ProcessState::Ready {
            return Err(ProcessError::NotRunnable);
        }
        let ticket = record
            .execution
            .reserve(crate::cpu::index())
            .ok_or(ProcessError::NotRunnable)?;
        Ok(Self {
            pid,
            task: record.process.clone().expect("verified task"),
            ticket,
            _cpu_bound: PhantomData,
        })
    }
    pub(super) fn start(&mut self) -> Result<(), ProcessError> {
        let _irq = InterruptGuard::new();
        let mut process = self.task.lock();
        let from = {
            let mut table = process_table();
            let index =
                process_record_index(&table, self.pid).ok_or(ProcessError::NoSuchProcess)?;
            registry::verify(&table[index], &self.task)?;
            if let Some(owner) = table[index].owner {
                let owner_index =
                    process_record_index(&table, owner).ok_or(ProcessError::NoSuchProcess)?;
                if matches!(
                    table[owner_index].state,
                    ProcessState::Dead | ProcessState::Reaped
                ) {
                    return Err(ProcessError::NotRunnable);
                }
            }
            let record = &mut table[index];
            if record.state != ProcessState::Ready
                || !record.execution.start(self.ticket, crate::cpu::index())
            {
                return Err(ProcessError::NotRunnable);
            }
            let from = record.state;
            record.state = ProcessState::Running;
            record.has_run = true;
            process.state = ProcessState::Running;
            from
        };
        drop(process);
        scheduler::remove(self.pid);
        log_process_transition(self.pid, from, ProcessState::Running, "enter-user");
        Ok(())
    }
    pub(super) fn returned(&mut self) {
        let mut table = process_table();
        let index =
            process_record_index(&table, self.pid).expect("entered task retired before HAL return");
        assert!(registry::verify(&table[index], &self.task).is_ok());
        table[index].execution.returned(self.ticket);
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _irq = InterruptGuard::new();
        let ready = {
            let mut table = process_table();
            process_record_index(&table, self.pid)
                .map(|index| {
                    let record = &mut table[index];
                    registry::verify(record, &self.task).is_ok()
                        && record.execution.release(self.ticket)
                        && record.state == ProcessState::Ready
                })
                .unwrap_or(false)
        };
        if ready {
            let _ = scheduler::enqueue_ready(self.pid);
        }
    }
}
