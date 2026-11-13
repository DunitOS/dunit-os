#!/usr/bin/env python3
"""Compile the actual current-access module to enforce non-escaping borrows."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT


class Access(unittest.TestCase):
    def compile(self, body, run=False):
        source = r'''
#![allow(dead_code)]
extern crate alloc;
#[path=@SYNC@] mod actual_sync;
mod sync {
    use crate::actual_sync;
    pub use actual_sync::SpinLock as IrqSafeSpinLock;
    pub type IrqSafeSpinLockGuard<'a,T> = actual_sync::SpinLockGuard<'a,T>;
    pub struct InterruptGuard;
    impl InterruptGuard { pub fn new() -> Self { Self } }
}
mod process {
    use std::sync::atomic::{AtomicU64, Ordering};
    #[derive(Clone,Copy,Debug,PartialEq,Eq)] pub struct ProcessId(pub u64);
    pub type Handle=u32;
    #[derive(Debug,PartialEq,Eq)] pub enum ProcessError { NoSuchProcess,ProcessNotPrepared,NotRunnable }
    #[derive(Debug,PartialEq,Eq)] pub enum HandleError { NoSuchTarget,AccessDenied,WrongType }
    pub struct Process { pub pid:u64,pub cwd:String,pub cap:std::sync::Arc<u64>,pub pending:bool,pub signals:u64 }
    impl Process {
        fn take_fd_wakeup(&mut self)->bool { std::mem::take(&mut self.pending) }
        fn signal_target(&self,h:Handle)->Result<ProcessId,HandleError> {
            match h {0=>Err(HandleError::AccessDenied),3=>Err(HandleError::WrongType),n=>Ok(ProcessId(n as u64))}
        }
        fn add_pending_signal(&mut self,value:u64) { self.signals=self.signals.saturating_add(value); }
    }
    pub struct ProcessRecord {pid:ProcessId,process:Option<std::sync::Arc<crate::sync::IrqSafeSpinLock<Process>>>}
    #[path=@REGISTRY@] pub mod registry;
    use registry::table as process_table;
    pub static DROPS:AtomicU64=AtomicU64::new(0);
    impl Drop for Process {
        fn drop(&mut self) {
            if self.pid>=4 {
                let _metadata=registry::table(); // catches destructor under registry lock
                DROPS.fetch_add(1,Ordering::SeqCst);
            }
        }
    }
    pub fn probe_registry() {
        use std::sync::{Arc,atomic::{AtomicBool,Ordering}};
        fn object() -> registry::Task {
            Arc::new(crate::sync::IrqSafeSpinLock::new(Process {pid:4,cwd:String::new(),
                cap:Arc::new(4),pending:false,signals:0}))
        }
        registry::table().push(ProcessRecord {pid:ProcessId(4),process:Some(object())});
        let pin=registry::task(ProcessId(4)).unwrap();let weak=Arc::downgrade(&pin);
        let retired={registry::table().pop().unwrap()};
        assert_eq!(registry::validate(ProcessId(4),&pin),Err(ProcessError::NoSuchProcess));
        registry::table().push(ProcessRecord {pid:ProcessId(4),process:Some(object())});
        assert_eq!(registry::validate(ProcessId(4),&pin),Err(ProcessError::ProcessNotPrepared));
        drop(retired);assert!(weak.upgrade().is_some());drop(pin);assert!(weak.upgrade().is_none());
        assert_eq!(DROPS.load(Ordering::SeqCst),1);
        let workers:Vec<_>=(0..8).map(|_|std::thread::spawn(|| {
            for _ in 0..1000 {
                with_process_mut(ProcessId(4),|process| {
                    assert_eq!(registry::table().len(),4); // callback runs without registry guard
                    process.signals+=1;Ok(())
                }).unwrap();
            }
        })).collect();
        for worker in workers {worker.join().unwrap();}
        assert_eq!(current_access::with_process(ProcessId(4),|p|p.signals),Ok(8000));
        registry::with_record(ProcessId(4),|process,record| {
            assert_eq!(record.pid,ProcessId(4));process.signals+=1;Ok(())
        }).unwrap();
        let entered=Arc::new(AtomicBool::new(false));let finish=Arc::new(AtomicBool::new(false));
        let weak={Arc::downgrade(&registry::task(ProcessId(4)).unwrap())};
        let worker={let entered=entered.clone();let finish=finish.clone();std::thread::spawn(move|| {
            with_process_mut(ProcessId(4),|p| {
                entered.store(true,Ordering::SeqCst);
                while !finish.load(Ordering::SeqCst) {std::thread::yield_now();}
                p.signals+=1;Ok(p.signals)
            }).unwrap()
        })};
        while !entered.load(Ordering::SeqCst) {std::thread::yield_now();}
        let retired={registry::table().pop().unwrap()};drop(retired);
        assert!(weak.upgrade().is_some());assert_eq!(DROPS.load(Ordering::SeqCst),1);
        registry::table().push(ProcessRecord {pid:ProcessId(4),process:Some(object())});
        with_process_mut(ProcessId(4),|p| {p.signals=100;Ok(())}).unwrap();
        finish.store(true,Ordering::SeqCst);assert_eq!(worker.join().unwrap(),8002);
        assert!(weak.upgrade().is_none());assert_eq!(DROPS.load(Ordering::SeqCst),2);
        assert_eq!(current_access::with_process(ProcessId(4),|p|p.signals),Ok(100));
        // Hold the object lock while another lookup obtains the old Arc. The
        // registry must stay available; after replacement the blocked operation
        // must reject its old lease instead of touching the replacement.
        let pin=registry::task(ProcessId(4)).unwrap();let guard=pin.lock();
        let waiter=std::thread::spawn(||with_process_mut(ProcessId(4),|p| {p.signals=999;Ok(())}));
        while Arc::strong_count(&pin)<3 {std::thread::yield_now();}
        let retired={registry::table().pop().unwrap()};
        registry::table().push(ProcessRecord {pid:ProcessId(4),process:Some(object())});
        drop(retired);drop(guard);
        assert_eq!(waiter.join().unwrap(),Err(ProcessError::ProcessNotPrepared));
        assert_eq!(current_access::with_process(ProcessId(4),|p|p.signals),Ok(0));
        drop(pin);
        println!("registry: real locks, 8000 mutations, reentry, pinned retirement and blocked stale lease rejected OK");
    }
    pub static CURRENT:AtomicU64=AtomicU64::new(2);
    pub static WAKES:AtomicU64=AtomicU64::new(0);
    fn current_pid()->Option<ProcessId> { (CURRENT.load(Ordering::SeqCst)!=0).then_some(ProcessId(1)) }
    fn current_entity_id()->Option<ProcessId> { let n=CURRENT.load(Ordering::SeqCst);(n!=0).then_some(ProcessId(n)) }
    fn wake_wait_set_waiters() {
        WAKES.fetch_add(1,Ordering::SeqCst);
        assert_eq!(current_access::with_current_process(|p|p.pending),Some(false));
    }
    pub fn init() {
        for n in 1..=2 { process_table().push(ProcessRecord {pid:ProcessId(n),process:Some(std::sync::Arc::new(crate::sync::IrqSafeSpinLock::new(Process {
            pid:n,cwd:format!("task{n}"),cap:std::sync::Arc::new(n),pending:false,signals:0 })))}); }
        process_table().push(ProcessRecord {pid:ProcessId(3),process:None});
    }
    #[path=@ACCESS@] pub mod current_access;
    pub use current_access::*;
}
fn main() {
process::init();
@BODY@
}
'''.replace('@SYNC@', json.dumps(str(ROOT / 'kernel/src/sync.rs'))).replace('@REGISTRY@', json.dumps(str(ROOT / 'kernel/src/process/registry.rs'))).replace('@ACCESS@', json.dumps(str(ROOT / 'kernel/src/process/current_access.rs'))).replace('@BODY@', body)
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'access.rs'
            path.write_text(source)
            result = subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', str(path),
                                     '-o', str(Path(folder) / 'access')], capture_output=True, text=True)
            if run and result.returncode == 0:
                subprocess.run([str(Path(folder) / 'access')], check=True, timeout=30)
            return result

    def test_owned_results_compile(self):
        result = self.compile('''
let _ = process::with_current_process(|p| p.pid);
let _ = process::with_current_process(|p| p.cwd.clone());
let _ = process::with_current_process(|p| p.cap.clone());
let _ = process::with_current_process_mut(|p| { p.pid += 1; p.pid });
let _ = process::with_current_thread_mut(|p| std::mem::take(&mut p.cwd));
''')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_process_and_field_borrows_cannot_escape(self):
        for helper, expression in [('with_current_process', 'p'), ('with_current_process', 'p.cwd.as_str()'),
                                   ('with_current_process_mut', '&mut p.pid'),
                                   ('with_current_thread', 'p'),
                                   ('with_current_thread_mut', '&mut p.cwd')]:
            with self.subTest(helper=helper, expression=expression):
                result = self.compile(f'let _ = process::{helper}(|p| {expression});')
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('lifetime', result.stderr)

    def test_pid_mutation_borrow_cannot_escape(self):
        result=self.compile('let _ = process::with_process_mut(process::ProcessId(1), |p| Ok(&mut p.pid));')
        self.assertNotEqual(result.returncode,0)
        self.assertIn('lifetime',result.stderr)

    def test_routing_notifications_errors_and_signal(self):
        result=self.compile(r"""
use process::*;
use std::sync::atomic::Ordering;
assert_eq!(with_current_process(|p|p.pid),Some(1));
assert_eq!(with_current_thread(|p|p.pid),Some(2));
with_current_thread_mut(|p|p.cwd=String::from("thread"));
assert_eq!(with_current_process(|p|p.cwd.clone()),Some(String::from("task1")));
assert_eq!(with_current_thread(|p|p.cwd.clone()),Some(String::from("thread")));
assert_eq!(with_process_mut(ProcessId(99), |_| Ok(())),Err(ProcessError::NoSuchProcess));
assert_eq!(with_process_mut(ProcessId(3), |_| Ok(())),Err(ProcessError::ProcessNotPrepared));
let result:Result<(),_>=with_process_mut(ProcessId(1),|p| {p.pending=true;Err(ProcessError::NotRunnable)});
assert_eq!(result,Err(ProcessError::NotRunnable));assert_eq!(WAKES.load(Ordering::SeqCst),1);
let result=with_current_process_mut(|p| {p.pending=true;Err::<(),_>(ProcessError::NotRunnable)});
assert_eq!(result,Some(Err(ProcessError::NotRunnable)));assert_eq!(WAKES.load(Ordering::SeqCst),2);
assert_eq!(signal_current_handle(1,7),Some(Ok(())));
assert_eq!(signal_current_handle(1,3),Some(Ok(())));
assert_eq!(with_current_process(|p|p.signals),Some(10));
assert_eq!(signal_current_handle(2,5),Some(Ok(())));
assert_eq!(with_current_thread(|p|p.signals),Some(5));
assert_eq!(signal_current_handle(0,1),Some(Err(HandleError::AccessDenied)));
assert_eq!(signal_current_handle(3,1),Some(Err(HandleError::WrongType)));
assert_eq!(signal_current_handle(99,1),Some(Err(HandleError::NoSuchTarget)));
CURRENT.store(0,Ordering::SeqCst);
assert_eq!(with_current_process(|p|p.pid),None);
assert_eq!(with_current_thread_mut(|p|p.pid),None);
assert_eq!(signal_current_handle(1,1),None);
println!("task scopes: owner/thread routing, error notifications, self/cross-target signal and missing tasks OK");
""",run=True)
        self.assertEqual(result.returncode,0,result.stderr)

    def test_registry_pins_exclusion_reentry_and_retirement(self):
        result=self.compile('process::probe_registry();',run=True)
        self.assertEqual(result.returncode,0,result.stderr)

    def test_static_argument_callback_is_rejected(self):
        result = self.compile('''
fn needs_static(_: &'static process::Process) -> u64 { 0 }
let _ = process::with_current_process(needs_static);
''')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('not general enough', result.stderr)


if __name__ == '__main__':
    unittest.main()
