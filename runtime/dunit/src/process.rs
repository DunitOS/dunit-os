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
    /// The child was reaped before its prepared image ever ran.
    pub fn spawn_prepared(self) -> bool {
        self.kind == libdunit::WAIT_KIND_SPAWN_PREPARED
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
        self.wait_ns_with(timeout, libdunit::wait, |pid, timeout| {
            use libdunit::wait_set::*;
            let mut result = [Event::default()];
            wait(&[Interest::process(pid, 0)], &mut result, timeout)
                .map(|count| count != 0)
                .map_err(Error)
        })
    }
    fn wait_ns_with(
        &mut self,
        timeout: u64,
        mut reap: impl FnMut(u32, &mut libdunit::WaitStatus) -> isize,
        wait: impl FnOnce(u32, u64) -> Result<bool>,
    ) -> Result<Option<ExitStatus>> {
        // A prepared child can be reaped by native wait but is not an exited
        // process wait-set event. Probe before subscribing so it cannot strand
        // a supervisor in an infinite wait.
        if let Some(status) = self.try_wait_with(&mut reap)? {
            return Ok(Some(status));
        }
        if !wait(self.pid, timeout)? {
            return Ok(None);
        }
        self.try_wait_with(reap)
    }
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.try_wait_with(libdunit::wait)
    }
    fn try_wait_with(
        &mut self,
        wait: impl FnOnce(u32, &mut libdunit::WaitStatus) -> isize,
    ) -> Result<Option<ExitStatus>> {
        if let Some(status) = self.status {
            return Ok(Some(status));
        }
        let mut status = libdunit::WaitStatus::empty();
        let result = wait(self.pid, &mut status);
        if result == libdunit::EAGAIN {
            return Ok(None);
        }
        let pid = checked(result)?;
        if pid != self.pid as usize
            || (!status.exited() && !status.faulted() && !status.spawn_prepared())
        {
            return Err(Error(libdunit::EIO));
        }
        let status = ExitStatus {
            kind: status.kind,
            code: status.code,
        };
        self.status = Some(status);
        Ok(Some(status))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn child() -> Child {
        Child::from_spawned(42).unwrap()
    }

    #[test]
    fn prepared_child_reaps_before_watch_and_caches_without_reporting_success() {
        let mut child = child();
        let status = child
            .wait_ns_with(
                u64::MAX,
                |pid, status| {
                    assert_eq!(pid, 42);
                    status.kind = libdunit::WAIT_KIND_SPAWN_PREPARED;
                    pid as isize
                },
                |_, _| panic!("prepared child must not wait for an exit event"),
            )
            .unwrap()
            .unwrap();
        assert!(status.spawn_prepared());
        assert!(!status.success());
        assert_eq!(status.code(), None);
        assert_eq!(status.fault_kind(), None);
        assert_eq!(
            child.try_wait_with(|_, _| panic!("cached child reaped twice")),
            Ok(Some(status))
        );
    }

    #[test]
    fn pending_timeout_and_errors_do_not_cache_a_status() {
        let mut child = child();
        assert_eq!(child.try_wait_with(|_, _| libdunit::EAGAIN), Ok(None));
        assert_eq!(
            child.wait_ns_with(
                77,
                |_, _| libdunit::EAGAIN,
                |pid, timeout| {
                    assert_eq!((pid, timeout), (42, 77));
                    Ok(false)
                }
            ),
            Ok(None)
        );
        assert_eq!(
            child.try_wait_with(|_, _| libdunit::ECHILD),
            Err(Error(libdunit::ECHILD))
        );
        assert_eq!(
            child.wait_ns_with(
                0,
                |_, _| libdunit::EAGAIN,
                |_, _| Err(Error(libdunit::EINVAL))
            ),
            Err(Error(libdunit::EINVAL))
        );
        assert!(child.status.is_none());
    }

    #[test]
    fn completion_between_probe_and_watch_is_reaped_after_level_wakeup() {
        for (kind, code) in [
            (libdunit::WAIT_KIND_EXITED, 0),
            (libdunit::WAIT_KIND_EXITED, 7),
            (3, -1),
        ] {
            let mut child = child();
            let mut probes = 0;
            let status = child
                .wait_ns_with(
                    100,
                    |pid, status| {
                        probes += 1;
                        if probes == 1 {
                            return libdunit::EAGAIN;
                        }
                        status.kind = kind;
                        status.code = code;
                        pid as isize
                    },
                    |_, _| Ok(true),
                )
                .unwrap()
                .unwrap();
            assert_eq!(probes, 2);
            assert_eq!(
                status.success(),
                kind == libdunit::WAIT_KIND_EXITED && code == 0
            );
            assert_eq!(status.raw_code(), code);
            assert_eq!(status.fault_kind(), (kind > 0).then_some(kind));
            assert_eq!(
                child.try_wait_with(|_, _| panic!("completed child reaped twice")),
                Ok(Some(status))
            );
        }
    }

    #[test]
    fn rejects_mismatched_pid_and_unknown_status_without_caching() {
        for (pid, kind) in [(43, 0), (0, 0), (42, libdunit::WAIT_KIND_EMPTY), (42, -3)] {
            let mut child = child();
            assert_eq!(
                child.try_wait_with(|_, status| {
                    status.kind = kind;
                    pid
                }),
                Err(Error(libdunit::EIO))
            );
            assert!(child.status.is_none());
        }
    }
}
