//! Execution-ticket state, always protected by the registry lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    generation: u64,
    cpu: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Reserved,
    Active,
    Returned,
}

/// Metadata protected by the registry lock, independent of lifecycle state.
/// Ready/Blocked can be published while the previous CPU still owns HAL/FPU.
#[derive(Default)]
pub(crate) struct State {
    generation: u64,
    held: Option<(Ticket, Phase)>,
    finalizing: bool,
}
impl State {
    pub(crate) fn reserve(&mut self, cpu: usize) -> Option<Ticket> {
        if self.busy() {
            return None;
        }
        self.generation = self
            .generation
            .checked_add(1)
            .expect("execution generation exhausted");
        let ticket = Ticket {
            generation: self.generation,
            cpu,
        };
        self.held = Some((ticket, Phase::Reserved));
        Some(ticket)
    }
    pub(crate) fn start(&mut self, ticket: Ticket, cpu: usize) -> bool {
        if self.finalizing || ticket.cpu != cpu || self.held != Some((ticket, Phase::Reserved)) {
            return false;
        }
        self.held = Some((ticket, Phase::Active));
        true
    }
    pub(crate) fn returned(&mut self, ticket: Ticket) {
        assert_eq!(
            self.held,
            Some((ticket, Phase::Active)),
            "invalid HAL return owner"
        );
        self.held = Some((ticket, Phase::Returned));
    }
    /// Local thread teardown has finished after HAL returned. Permit detaching
    /// its object/status atomically; a still-active CPU must never pass here.
    pub(crate) fn retire_returned(&mut self, cpu: usize) -> bool {
        if !matches!(self.held, Some((ticket, Phase::Returned)) if ticket.cpu == cpu) {
            return false;
        }
        self.held = None;
        true
    }
    pub(crate) fn release(&mut self, ticket: Ticket) -> bool {
        if self.held.map(|held| held.0) != Some(ticket)
            || self.held == Some((ticket, Phase::Active))
        {
            return false;
        }
        self.held = None;
        true
    }
    pub(crate) fn busy(&self) -> bool {
        self.held.is_some() || self.finalizing
    }
    pub(crate) fn finalizing(&self) -> bool {
        self.finalizing
    }
    /// Cleanup is a separate ownership interval after hardware return. An
    /// external finalizer may revoke Reserved admission, never Active/Returned.
    pub(crate) fn begin_finalization(&mut self, returned_cpu: Option<usize>) -> bool {
        if self.finalizing {
            return false;
        }
        match (returned_cpu, self.held) {
            (Some(cpu), Some((ticket, Phase::Returned))) if ticket.cpu == cpu => {}
            (None, None | Some((_, Phase::Reserved))) => {
                self.held = None;
            }
            _ => return false,
        }
        self.finalizing = true;
        true
    }
    /// Publishing a waitable result requires all dispatch backing to have been
    /// released. Lease drop clears held, but cannot clear the cleanup fence.
    pub(crate) fn complete_finalization(&mut self) -> bool {
        if !self.finalizing || self.held.is_some() {
            return false;
        }
        self.finalizing = false;
        true
    }
    pub(crate) fn retirement_blocked(&self) -> bool {
        self.entered() || self.finalizing
    }
    /// Reserved frames may be revoked by retirement; entered frames must finish
    /// their HAL return and dispatcher bookkeeping before external retirement.
    pub(crate) fn entered(&self) -> bool {
        matches!(self.held, Some((_, Phase::Active | Phase::Returned)))
    }
}
