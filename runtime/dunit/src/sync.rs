//! Native futex-backed mutual exclusion and condition variables.
//!
//! There is no poisoning or unwinding. A worker that faults or exits while
//! holding a lock leaves it locked. Locking is not recursive.
use crate::thread::platform;
use core::{
    cell::UnsafeCell,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicU32, Ordering},
};

/// One owner at a time, with bounded state and blocking contended waits.
pub struct Mutex<T: ?Sized> {
    state: AtomicU32,
    value: UnsafeCell<T>,
}
unsafe impl<T: Send + ?Sized> Send for Mutex<T> {}
unsafe impl<T: Send + ?Sized> Sync for Mutex<T> {}
impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            state: AtomicU32::new(0),
            value: UnsafeCell::new(value),
        }
    }
    pub fn into_inner(self) -> T {
        self.value.into_inner()
    }
}
impl<T: ?Sized> Mutex<T> {
    pub fn get_mut(&mut self) -> &mut T {
        self.value.get_mut()
    }
    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {
        self.state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| MutexGuard {
                mutex: self,
                _not_send: PhantomData,
            })
    }
    pub fn lock(&self) -> MutexGuard<'_, T> {
        if let Some(guard) = self.try_lock() {
            return guard;
        }
        loop {
            if self.state.swap(2, Ordering::Acquire) == 0 {
                break;
            }
            let result = platform::futex_wait(&self.state, 2, 0);
            assert!(
                platform::retry_wait(result),
                "native mutex wait failed: {result}"
            );
        }
        MutexGuard {
            mutex: self,
            _not_send: PhantomData,
        }
    }
}
impl<T: Default> Default for Mutex<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// A guard stays on its acquiring thread. Shared access requires `T: Sync`.
pub struct MutexGuard<'a, T: ?Sized> {
    mutex: &'a Mutex<T>,
    _not_send: PhantomData<*mut ()>,
}
unsafe impl<T: Sync + ?Sized> Sync for MutexGuard<'_, T> {}
impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.value.get() }
    }
}
impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.value.get() }
    }
}
impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        if self.mutex.state.swap(0, Ordering::Release) == 2 {
            let result = platform::futex_wake(&self.mutex.state, 1);
            assert!(result >= 0, "native mutex wake failed: {result}");
        }
    }
}

/// Notifications are hints; always recheck the protected predicate in a loop.
pub struct Condvar {
    sequence: AtomicU32,
}
impl Default for Condvar {
    fn default() -> Self {
        Self::new()
    }
}
impl Condvar {
    pub const fn new() -> Self {
        Self {
            sequence: AtomicU32::new(0),
        }
    }
    pub fn wait<'a, T: ?Sized>(&self, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
        let sequence = self.sequence.load(Ordering::Acquire);
        let mutex = guard.mutex;
        drop(guard);
        let result = platform::futex_wait(&self.sequence, sequence, 0);
        assert!(
            platform::retry_wait(result),
            "native condition wait failed: {result}"
        );
        mutex.lock()
    }
    pub fn wait_while<'a, T: ?Sized>(
        &self,
        mut guard: MutexGuard<'a, T>,
        mut predicate: impl FnMut(&mut T) -> bool,
    ) -> MutexGuard<'a, T> {
        while predicate(&mut *guard) {
            guard = self.wait(guard);
        }
        guard
    }
    fn notify(&self, count: usize) {
        // Never wrap a futex generation back to an old expected value.
        let mut sequence = self.sequence.load(Ordering::Relaxed);
        loop {
            let next = sequence
                .checked_add(1)
                .expect("condition notification generation exhausted");
            match self.sequence.compare_exchange_weak(
                sequence,
                next,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => sequence = actual,
            }
        }
        let result = platform::futex_wake(&self.sequence, count);
        assert!(result >= 0, "native condition wake failed: {result}");
    }
    pub fn notify_one(&self) {
        self.notify(1);
    }
    pub fn notify_all(&self) {
        self.notify(0);
    }
}
