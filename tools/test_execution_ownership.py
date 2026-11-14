#!/usr/bin/env python3
"""Compile the actual execution state: contended admission and HAL handoff."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT


class ExecutionOwnership(unittest.TestCase):
    def compile_leases(self, send=False):
        source=(ROOT/'tools/test_execution_host.rs.in').read_text()
        for name,path in [('SYNC','kernel/src/sync.rs'),('REGISTRY','kernel/src/process/registry.rs'),
                          ('EXECUTION','kernel/src/process/execution.rs')]:
            source=source.replace('@'+name+'@',json.dumps(str(ROOT/path)))
        source=source.replace('@SEND@','std::thread::spawn(move||drop(lease));' if send else 'drop(lease);')
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'leases.rs';path.write_text(source)
            result=subprocess.run([os.environ.get('RUSTC','rustc'),'--edition=2021',str(path),'-o',str(path.with_suffix(''))],capture_output=True,text=True)
            if not send and result.returncode==0:
                subprocess.run([str(path.with_suffix(''))],check=True,timeout=30)
            return result

    def test_actual_registry_locks_and_leases(self):
        result=self.compile_leases()
        self.assertEqual(result.returncode,0,result.stderr)

    def test_execution_lease_cannot_migrate(self):
        result=self.compile_leases(send=True)
        self.assertNotEqual(result.returncode,0)
        self.assertIn('cannot be sent between threads',result.stderr)

    def test_exclusive_admission_handoff_and_stale_tickets(self):
        source = r'''
#[path=@STATE@] mod state;
use state::State;
use std::sync::{Arc,Mutex,Barrier,atomic::{AtomicUsize,Ordering}};
fn main() {
    let mut state=State::default();
    let old=state.reserve(0).unwrap();
    assert!(state.busy());assert!(!state.entered());
    assert!(!state.start(old,1)); // a reservation cannot migrate
    assert!(!state.retire_returned(0));
    assert!(state.start(old,0));assert!(state.entered());
    assert!(!state.start(old,0)); // one ticket cannot enter twice
    assert!(!state.retire_returned(0)); // active HAL/FPU cannot be retired
    assert!(!state.release(old)); // even the owner cannot release active hardware
    assert!(state.reserve(1).is_none());
    // Ready/Blocked is separate metadata; wake-before-return cannot grant a CPU.
    assert!(state.reserve(0).is_none());
    state.returned(old);assert!(state.busy());assert!(state.entered());
    assert!(state.reserve(1).is_none()); // exit bookkeeping still owns admission
    assert!(!state.retire_returned(1));
    assert!(state.retire_returned(0));assert!(!state.busy());
    let new=state.reserve(0).unwrap();
    assert!(!state.start(old,0));assert!(!state.release(old));
    assert!(state.start(new,0));state.returned(new);assert!(state.release(new));
    let abandoned=state.reserve(3).unwrap();assert!(state.release(abandoned));
    assert!(!state.busy()); // an unused frame does not strand a Ready task

    let state=Arc::new(Mutex::new(State::default()));
    let in_hal=Arc::new(AtomicUsize::new(0));let completed=Arc::new(AtomicUsize::new(0));
    let barrier=Arc::new(Barrier::new(8));
    let workers:Vec<_>=(0..8).map(|cpu| {
        let state=state.clone();let in_hal=in_hal.clone();let completed=completed.clone();let barrier=barrier.clone();
        std::thread::spawn(move|| {
            barrier.wait();
            for _ in 0..1000 {
                let ticket=loop {
                    if let Some(ticket)=state.lock().unwrap().reserve(cpu) { break ticket; }
                    std::thread::yield_now();
                };
                assert!(state.lock().unwrap().start(ticket,cpu));
                assert_eq!(in_hal.fetch_add(1,Ordering::SeqCst),0);
                std::thread::yield_now(); // simulated hardware interval, no metadata lock
                assert_eq!(in_hal.fetch_sub(1,Ordering::SeqCst),1);
                state.lock().unwrap().returned(ticket);
                std::thread::yield_now(); // return bookkeeping still excludes all other CPUs
                assert!(state.lock().unwrap().release(ticket));
                completed.fetch_add(1,Ordering::SeqCst);
            }
        })
    }).collect();
    for worker in workers {worker.join().unwrap();}
    assert_eq!(completed.load(Ordering::SeqCst),8000);assert!(!state.lock().unwrap().busy());
    println!("execution: 8000 exclusive entries, CPU/generation identity, wake-before-return and deferred retirement OK");
}
'''.replace('@STATE@',json.dumps(str(ROOT/'kernel/src/process/execution_state.rs')))
        with tempfile.TemporaryDirectory() as folder:
            path=Path(folder)/'execution.rs';path.write_text(source)
            subprocess.run([os.environ.get('RUSTC','rustc'),'--edition=2021',str(path),'-o',str(path.with_suffix(''))],check=True)
            subprocess.run([str(path.with_suffix(''))],check=True,timeout=30)


if __name__=='__main__':
    unittest.main()
