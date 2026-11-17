//! Owned native threads. Dropping a handle joins it before reclaiming its stack.
//!
//! Closures and results must be `Send + 'static`. There is no detach operation:
//! the existing ABI cannot reclaim a running thread's own userspace stack.
use crate::{Error, Result};
use alloc::boxed::Box;
use core::{
    cell::UnsafeCell,
    fmt,
    mem::ManuallyDrop,
    sync::atomic::{AtomicU32, Ordering},
};

#[cfg(not(test))]
#[path = "thread/platform.rs"]
pub(crate) mod platform;
#[cfg(test)]
#[path = "thread/test_platform.rs"]
pub(crate) mod platform;

const PAGE_SIZE: usize = 4096;
const DEFAULT_STACK_SIZE: usize = 256 * 1024;
const MIN_STACK_SIZE: usize = 16 * 1024;

/// A native thread identity, distinct from the owning process identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadId(usize);
pub fn current_id() -> ThreadId {
    ThreadId(platform::current_id())
}
pub fn yield_now() -> Result<()> {
    crate::error::checked(platform::yield_now()).map(|_| ())
}

/// An abnormal native exit, or an exit without returning the closure's result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JoinError {
    pub kind: i32,
    pub code: i32,
}
impl fmt::Display for JoinError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            out,
            "thread did not return a value (kind={}, code={})",
            self.kind, self.code
        )
    }
}
impl core::error::Error for JoinError {}

/// Stack size is the usable range; native guard pages surround it.
#[derive(Clone, Copy, Debug)]
pub struct Builder {
    stack_size: usize,
}
impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}
impl Builder {
    pub const fn new() -> Self {
        Self {
            stack_size: DEFAULT_STACK_SIZE,
        }
    }
    pub fn stack_size(mut self, bytes: usize) -> Self {
        self.stack_size = bytes;
        self
    }
    pub fn spawn<F, T>(self, run: F) -> Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        if self.stack_size < MIN_STACK_SIZE {
            return Err(Error(crate::errno::EINVAL));
        }
        let bytes = self
            .stack_size
            .checked_add(PAGE_SIZE - 1)
            .map(|n| n & !(PAGE_SIZE - 1))
            .filter(|n| *n >= MIN_STACK_SIZE)
            .and_then(|n| n.checked_add(2 * PAGE_SIZE).map(|_| n))
            .ok_or(Error(crate::errno::EINVAL))?;
        let address = crate::error::checked(platform::map_stack(bytes))?;
        let stack = Stack { address, bytes };
        let packet = Box::new(Packet {
            job: UnsafeCell::new(Some(Box::new(run))),
            result: UnsafeCell::new(None),
            done: AtomicU32::new(0),
        });
        let started = platform::create(&*packet, address + bytes);
        if started <= 0 {
            stack.release();
            return Err(Error(if started == 0 {
                crate::errno::EIO
            } else {
                started
            }));
        }
        Ok(JoinHandle {
            packet: ManuallyDrop::new(packet),
            stack: Some(stack),
            tid: started as usize,
            joined: false,
        })
    }
}
pub fn spawn<F, T>(run: F) -> Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Builder::new().spawn(run)
}

struct Stack {
    address: usize,
    bytes: usize,
}
impl Stack {
    fn release(self) {
        let result = platform::unmap_stack(self.address, self.bytes);
        assert_eq!(result, 0, "native thread stack unmap failed: {result}");
    }
}
struct Packet<T> {
    job: UnsafeCell<Option<Box<dyn FnOnce() -> T + Send + 'static>>>,
    result: UnsafeCell<Option<T>>,
    done: AtomicU32,
}
// Only the admitted worker accesses job/result before native join acknowledges
// that execution has stopped. The owner accesses them after that handoff.
unsafe impl<T: Send> Send for Packet<T> {}
unsafe impl<T: Send> Sync for Packet<T> {}

/// An owning handle. `join` and Drop wait for native execution to stop before
/// releasing any backing. Do not drop it while holding a lock its worker needs.
/// Self-join and broken native ownership contracts panic rather than free a
/// possibly live stack. SDK panics do not unwind; worker exits can skip destructors.
pub struct JoinHandle<T> {
    // Suppress automatic field destruction if a failed native join panics:
    // without a stop acknowledgement, freeing this packet would be unsound.
    packet: ManuallyDrop<Box<Packet<T>>>,
    stack: Option<Stack>,
    tid: usize,
    joined: bool,
}
impl<T> JoinHandle<T> {
    pub fn thread_id(&self) -> ThreadId {
        ThreadId(self.tid)
    }
    /// The closure has returned; `join` still performs the native stop handoff.
    pub fn is_finished(&self) -> bool {
        self.packet.done.load(Ordering::Acquire) != 0
    }
    fn wait(&mut self) -> (i32, i32) {
        assert_ne!(
            platform::current_id(),
            self.tid,
            "a thread cannot join itself"
        );
        loop {
            let (result, kind, code) = platform::join(self.tid);
            if result == self.tid as isize {
                self.joined = true;
                return (kind, code);
            }
            assert_eq!(
                result,
                crate::errno::EAGAIN,
                "native thread join failed: {result}"
            );
            if self.is_finished() {
                assert!(platform::yield_now() >= 0, "native thread yield failed");
            } else {
                // A faulted worker cannot publish done. A finite futex wait
                // periodically checks native exit without busy-polling its job.
                let result = platform::futex_wait(&self.packet.done, 0, 10);
                assert!(
                    platform::retry_wait(result),
                    "native thread completion wait failed: {result}"
                );
            }
        }
    }
    pub fn join(mut self) -> core::result::Result<T, JoinError> {
        let (kind, code) = self.wait();
        if kind != 0 || code != 0 {
            return Err(JoinError { kind, code });
        }
        unsafe { (*self.packet.result.get()).take() }.ok_or(JoinError { kind, code })
    }
}
impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        if !self.joined {
            self.wait();
        }
        if let Some(stack) = self.stack.take() {
            stack.release();
        }
        unsafe {
            ManuallyDrop::drop(&mut self.packet);
        }
    }
}

unsafe fn run_job<T>(address: usize) {
    let packet = &*(address as *const Packet<T>);
    let job = (*packet.job.get())
        .take()
        .expect("native thread entered twice");
    let result = job();
    *packet.result.get() = Some(result);
    packet.done.store(1, Ordering::Release);
    let wake = platform::futex_wake(&packet.done, 0);
    assert!(wake >= 0, "native thread completion wake failed: {wake}");
}
#[cfg(not(test))]
extern "C" fn entry<T>(address: usize) -> ! {
    unsafe {
        run_job::<T>(address);
    }
    libdunit::thread_exit(0)
}
