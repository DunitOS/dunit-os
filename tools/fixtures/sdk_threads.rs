extern crate alloc;
use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicUsize, Ordering};
use dunit::{
    sync::{Condvar, Mutex},
    thread,
};

struct CountDrop(Arc<AtomicUsize>);
impl Drop for CountDrop {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
fn require(value: bool) -> dunit::Result<()> {
    if value {
        Ok(())
    } else {
        dunit::println!("[SDK-THREADS] FAIL")?;
        Err(dunit::Error(dunit::errno::EIO))
    }
}
#[dunit::main]
fn main() -> dunit::Result<()> {
    let owner = thread::current_id();
    let count = Arc::new(Mutex::new((0usize, 0usize)));
    let mut workers = Vec::new();
    for index in 0..8 {
        let count = count.clone();
        workers.push(thread::spawn(move || {
            for _ in 0..500 {
                let mut value = count.lock();
                assert_eq!(value.1, value.0 * 3);
                value.0 += 1;
                value.1 += 3;
                drop(value);
                thread::yield_now().unwrap();
            }
            (index, thread::current_id(), String::from("owned result"))
        })?);
    }
    let mut ids = Vec::new();
    for (expected, worker) in workers.into_iter().enumerate() {
        let id = worker.thread_id();
        let (index, current, text) = worker.join().map_err(|_| dunit::Error(dunit::errno::EIO))?;
        require(
            index == expected
                && current == id
                && id != owner
                && !ids.contains(&id)
                && text == "owned result",
        )?;
        ids.push(id);
    }
    require(*count.lock() == (4000, 12000))?;
    dunit::println!("[SDK-THREADS] 8 workers, 4000 checked mutex updates and owned results OK")?;

    let pair = Arc::new((Mutex::new((0usize, false)), Condvar::new()));
    let remote = pair.clone();
    let worker = thread::spawn(move || {
        for step in 1..=128 {
            let mut state = remote.1.wait_while(remote.0.lock(), |state| !state.1);
            assert_eq!(state.0, step);
            state.1 = false;
            remote.1.notify_one();
        }
    })?;
    for step in 1..=128 {
        let mut state = pair.1.wait_while(pair.0.lock(), |state| state.1);
        state.0 = step;
        state.1 = true;
        pair.1.notify_one();
    }
    worker.join().map_err(|_| dunit::Error(dunit::errno::EIO))?;
    dunit::println!("[SDK-THREADS] 128 condition handoffs without lost notifications OK")?;

    let drops = Arc::new(AtomicUsize::new(0));
    let result = CountDrop(drops.clone());
    drop(thread::spawn(move || result)?);
    require(drops.load(Ordering::SeqCst) == 1)?;
    for expected in 0..24 {
        require(
            thread::Builder::new()
                .stack_size(32 * 1024)
                .spawn(move || expected)?
                .join()
                .map_err(|_| dunit::Error(dunit::errno::EIO))?
                == expected,
        )?;
    }
    require(matches!(
        thread::Builder::new().stack_size(1).spawn(|| ()),
        Err(dunit::Error(dunit::errno::EINVAL))
    ))?;
    dunit::println!(
        "[SDK-THREADS] joining Drop, 24 stack lifecycles and invalid stack rejection OK"
    )?;
    dunit::println!("[SDK-THREADS] all checks passed")
}
