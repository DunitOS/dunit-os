//! Monotonic time and blocking sleep through the native wait-set.
use crate::{error::checked, Error, Result};
pub use core::time::Duration;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Instant(u64);
impl Instant {
    pub fn now() -> Result<Self> {
        let mut time = [0u64; 2];
        checked(libdunit::syscall2(
            libdunit::SYSCALL_CLOCK_GET_TIME,
            1,
            time.as_mut_ptr() as usize,
        ))?;
        let ns = time[0]
            .checked_mul(1_000_000_000)
            .and_then(|sec| sec.checked_add(time[1]))
            .ok_or(Error(libdunit::EINVAL))?;
        Ok(Self(ns))
    }
    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        self.0
            .checked_add(u64::try_from(duration.as_nanos()).ok()?)
            .map(Self)
    }
    pub fn elapsed(self) -> Result<Duration> {
        Ok(Duration::from_nanos(Self::now()?.0.saturating_sub(self.0)))
    }
    pub fn deadline(self, token: u64) -> crate::event::Interest {
        crate::event::Interest::timer(self.0, token)
    }
}
pub(crate) fn timeout_ns(duration: Duration) -> Result<u64> {
    let ns = u64::try_from(duration.as_nanos()).map_err(|_| Error(libdunit::EINVAL))?;
    if ns == libdunit::wait_set::INFINITE {
        Err(Error(libdunit::EINVAL))
    } else {
        Ok(ns)
    }
}
pub fn sleep(duration: Duration) -> Result<()> {
    libdunit::wait_set::wait(&[], &mut [], timeout_ns(duration)?)
        .map(|_| ())
        .map_err(Error)
}
