//! Child ownership and wait-set-based completion. Drop does not kill or reap.
use crate::{
    error::checked,
    time::{timeout_ns, Duration},
    Error, Result,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    kind: i32,
    code: i32,
}
impl ExitStatus {
    pub fn success(self) -> bool {
        self.kind == libdunit::WAIT_KIND_EXITED && self.code == 0
    }
    pub fn code(self) -> Option<i32> {
        (self.kind == libdunit::WAIT_KIND_EXITED).then_some(self.code)
    }
    pub fn kind(self) -> i32 {
        self.kind
    }
    pub fn raw_code(self) -> i32 {
        self.code
    }
    pub fn fault_kind(self) -> Option<i32> {
        (self.kind > 0).then_some(self.kind)
    }
}
#[derive(Debug)]
pub struct Child {
    pid: u32,
    status: Option<ExitStatus>,
}
impl Child {
    pub fn spawn(path: &str) -> Result<Self> {
        Self::from_spawned(libdunit::spawn(path))
    }
    pub(crate) fn from_spawned(result: isize) -> Result<Self> {
        let pid = checked(result)?;
        if pid == 0 {
            return Err(Error(libdunit::EIO));
        }
        Ok(Self {
            pid: pid as u32,
            status: None,
        })
    }
    pub fn id(&self) -> u32 {
        self.pid
    }
    pub fn kill(&mut self) -> Result<()> {
        if self.status.is_some() {
            return Err(Error(libdunit::ECHILD));
        }
        checked(libdunit::kill(self.pid)).map(|_| ())
    }
    /// Reap natural completion or terminate a live child, including the race
    /// where the child exits between the nonblocking wait and kill.
    pub fn terminate_and_wait(&mut self) -> Result<ExitStatus> {
        if let Some(status) = self.try_wait()? {
            return Ok(status);
        }
        match self.kill() {
            Ok(()) => self.wait(),
            Err(error) => self.try_wait()?.ok_or(error),
        }
    }
    fn wait_ns(&mut self, timeout: u64) -> Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        use libdunit::wait_set::*;
        let mut result = [Event::default()];
        if wait(&[Interest::process(self.pid, 0)], &mut result, timeout).map_err(Error)? == 0 {
            return Ok(None);
        }
        let mut status = libdunit::WaitStatus::empty();
        checked(libdunit::wait(self.pid, &mut status))?;
        if !status.exited() && !status.faulted() {
            return Err(Error(libdunit::EIO));
        }
        let status = ExitStatus {
            kind: status.kind,
            code: status.code,
        };
        self.status = Some(status);
        Ok(Some(status))
    }
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.wait_ns(0)
    }
    pub fn wait_timeout(&mut self, duration: Duration) -> Result<Option<ExitStatus>> {
        self.wait_ns(timeout_ns(duration)?)
    }
    pub fn wait(&mut self) -> Result<ExitStatus> {
        self.wait_ns(libdunit::wait_set::INFINITE)?
            .ok_or(Error(libdunit::EIO))
    }
}
pub fn id() -> u32 {
    libdunit::get_pid()
}

/// Terminate the current process through the native runtime.
pub fn exit(code: i32) -> ! {
    libdunit::exit(code)
}
