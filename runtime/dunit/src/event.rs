//! Blocking event-loop core. Readiness never consumes application data.
use alloc::vec::Vec;
pub use libdunit::wait_set::{
    Event, Interest, ELAPSED, ERROR, EXITED, HANGUP, INFINITE, READ, WRITE,
};

pub use crate::Error;

/// Own the interest/result storage so application code needs no raw pointers.
/// Drain level-ready sources before waiting again. An IPC interest can also
/// carry cancellation messages; the application owns that message's meaning.
#[derive(Default)]
pub struct EventLoop {
    interests: Vec<Interest>,
    events: Vec<Event>,
}
impl EventLoop {
    pub fn new(interests: &[Interest]) -> Result<Self, Error> {
        let mut event_loop = Self::default();
        event_loop.set_interests(interests)?;
        Ok(event_loop)
    }
    pub fn set_interests(&mut self, interests: &[Interest]) -> Result<(), Error> {
        if interests.len() > libdunit::wait_set::MAX_INTERESTS
            || interests.iter().any(|interest| !interest.valid())
        {
            return Err(Error(libdunit::EINVAL));
        }
        self.interests.clear();
        self.interests.extend_from_slice(interests);
        self.events.resize(interests.len(), Event::default());
        Ok(())
    }
    pub fn wait(&mut self, timeout_ns: u64) -> Result<&[Event], Error> {
        let count = libdunit::wait_set::wait(&self.interests, &mut self.events, timeout_ns)
            .map_err(Error)?;
        Ok(&self.events[..count])
    }
}
