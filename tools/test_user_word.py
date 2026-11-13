#!/usr/bin/env python3
"""Actual futex admission/word code with host mapping and CPU/HAL shims."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT


SOURCE = r'''
extern crate alloc;
#[allow(dead_code)] #[path=@SYNC@] mod actual_sync;
mod sync {
    pub use crate::actual_sync::SpinLock as IrqSafeSpinLock;
    pub struct InterruptGuard;
    impl InterruptGuard { pub fn new() -> Self { Self } }
}
mod memory { pub mod vmm {
    use core::sync::atomic::{AtomicU32, AtomicU8, Ordering};
    pub static WORD: AtomicU32 = AtomicU32::new(7);
    pub static FLAGS: AtomicU8 = AtomicU8::new(7);
    #[derive(Clone, Copy)] pub struct PageFlags(u8);
    impl PageFlags {
        pub const PRESENT: Self=Self(1); pub const USER: Self=Self(2);
        pub const WRITABLE: Self=Self(4); pub fn empty() -> Self { Self(0) }
        pub fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
    }
    impl core::ops::BitOr for PageFlags { type Output=Self; fn bitor(self,b:Self)->Self { Self(self.0|b.0) } }
    pub struct VirtualAddress(pub usize);
    pub struct PhysicalAddress(usize);
    impl PhysicalAddress { pub fn as_usize(&self)->usize { self.0 } }
    pub fn phys_to_virt(addr:usize)->usize { addr }
    pub struct AddressSpace;
    impl AddressSpace {
        pub fn user_page_mapping(&self, addr:VirtualAddress)->Result<Option<(PhysicalAddress, PageFlags)>,()> {
            crate::process::assert_queue_unlocked();
            if addr.0 != 4092 { return Ok(None); }
            Ok(Some((PhysicalAddress(&WORD as *const AtomicU32 as usize), PageFlags(FLAGS.load(Ordering::SeqCst)))))
        }
    }
} }
mod cpu {
    use core::sync::atomic::{AtomicBool,AtomicU64};
    pub struct Runtime { pub yield_requested:AtomicBool, pub schedule_hint:AtomicU64 }
    static CPU:Runtime=Runtime { yield_requested:AtomicBool::new(false),schedule_hint:AtomicU64::new(0) };
    pub fn runtime()->&'static Runtime { &CPU }
}
mod hal {
    pub unsafe fn syscall_capture_user_context(_: *mut u64, _:u64) {
        crate::process::assert_compare_locks_held();
    }
}
#[allow(dead_code)] mod process {
    use alloc::sync::Arc;
    use crate::memory::vmm::{AddressSpace,FLAGS,WORD};
    use crate::sync::{InterruptGuard,IrqSafeSpinLock};
    use core::sync::atomic::Ordering;
    use std::sync::OnceLock;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct ProcessId(pub u64);
    #[derive(Clone, Copy, Debug, PartialEq, Eq)] enum ProcessState { Running,Blocked }
    #[derive(Debug,PartialEq,Eq)] enum ProcessError { NoCurrentProcess,NoSuchProcess,ProcessNotPrepared,InvalidMemoryRange,NotRunnable,InvalidUserContext }
    struct Process { state:ProcessState,is_kernel:bool,context:u64,clear_child_tid:u64 }
    struct Record { state:ProcessState, process:Option<Box<Process>> }
    struct Table(core::cell::UnsafeCell<Vec<Record>>); unsafe impl Sync for Table {}
    static TABLE:Table=Table(core::cell::UnsafeCell::new(Vec::new()));
    static SPACE:OnceLock<Arc<IrqSafeSpinLock<AddressSpace>>>=OnceLock::new();
    #[path=@WAIT@] mod wait_queue;
    use wait_queue::{wake_matching,WaitQueue,WaitKey,WaitEntry};
    static WAIT_QUEUE:IrqSafeSpinLock<WaitQueue>=IrqSafeSpinLock::new(WaitQueue::new());
    #[path=@WORD@] pub mod user_word;
    #[path=@FUTEX@] mod futex;
    pub fn assert_queue_unlocked() { assert!(WAIT_QUEUE.try_lock().is_some(),"queue -> task/mapping edge"); }
    pub fn assert_compare_locks_held() {
        assert!(WAIT_QUEUE.try_lock().is_none()); assert!(SPACE.get().unwrap().try_lock().is_none());
    }
    fn current_entity_id()->Option<ProcessId> { assert_queue_unlocked();Some(ProcessId(1)) }
    fn current_pid()->Option<ProcessId> { assert_queue_unlocked();Some(ProcessId(1)) }
    fn owner_of(pid:ProcessId)->Option<ProcessId> { assert_queue_unlocked();Some(pid) }
    fn process_address_space(_:ProcessId)->Result<Arc<IrqSafeSpinLock<AddressSpace>>,ProcessError> {
        assert_queue_unlocked();Ok(SPACE.get().unwrap().clone())
    }
    fn process_table_mut()->&'static mut Vec<Record> {
        assert_queue_unlocked();unsafe { &mut *TABLE.0.get() }
    }
    fn process_record_index(_: &[Record],pid:ProcessId)->Option<usize> { assert_queue_unlocked();(pid.0==1).then_some(0) }
    fn current_thread()->Option<&'static Process> { process_table_mut()[0].process.as_deref() }
    fn current_thread_mut()->Option<&'static mut Process> { process_table_mut()[0].process.as_deref_mut() }
    fn wake_entry(_:WaitEntry) { assert_queue_unlocked();assert!(SPACE.get().unwrap().try_lock().is_some()); }
    pub fn run() {
        use futex::{futex_wait,futex_wake,FutexWaitOutcome::*};
        SPACE.set(Arc::new(IrqSafeSpinLock::new(AddressSpace))).ok().unwrap();
        process_table_mut().push(Record {state:ProcessState::Running,process:Some(Box::new(Process {
            state:ProcessState::Running,is_kernel:false,context:0,clear_child_tid:0 }))});
        for flags in [0,1,2,4,5,6] {
            FLAGS.store(flags,Ordering::SeqCst);
            assert_eq!(futex_wait(4092,7,None).err(),Some(ProcessError::InvalidMemoryRange));
        }
        FLAGS.store(3,Ordering::SeqCst); // read-only user page is valid for comparison
        assert!(matches!(futex_wait(4092,8,None),Ok(ValueMismatch)));
        assert_eq!(futex_wake(4092,0),0);
        for addr in [0,4093,8192] {
            assert_eq!(futex_wait(addr,7,None).err(),Some(ProcessError::InvalidMemoryRange));
        }
        assert!(matches!(futex_wait(4092,7,Some(10)),Ok(Parked)));
        assert_eq!(process_table_mut()[0].state,ProcessState::Blocked);
        assert_eq!(futex_wake(4092,1),1); assert_eq!(futex_wake(4092,0),0);
        let space=SPACE.get().unwrap().lock();
        let word=user_word::UserWord::new(&space,4092).unwrap();assert_eq!(word.load(),7);
        assert!(!user_word::UserWord::clear(&space,4092));assert_eq!(word.load(),7);
        FLAGS.store(7,Ordering::SeqCst);assert!(user_word::UserWord::clear(&space,4092));
        assert_eq!(WORD.load(Ordering::Acquire),0);
        drop(word); drop(space);
        futex::set_current_clear_child_tid(4092);
        WORD.store(42,Ordering::SeqCst);
        WAIT_QUEUE.lock().push(WaitEntry { entity:ProcessId(1),
            key:WaitKey::Word {owner:ProcessId(1),addr:4092}, deadline:None });
        futex::run_clear_child_tid_on_exit(); // callback asserts mapping guard released
        assert_eq!(WORD.load(Ordering::Acquire),0);
        assert_eq!(futex_wake(4092,0),0);
        println!("futex: task/mapping before queue; held compare/register locks; flags/alignment/mismatch/wake/clear OK");
    }
    @EXTRA@
}
fn main() { process::run(); }
'''


class UserWordTest(unittest.TestCase):
    def compile_run(self, extra='', expect_ok=True):
        source = SOURCE
        for key, path in {'SYNC':'kernel/src/sync.rs', 'WAIT':'kernel/src/process/wait_queue.rs',
                          'WORD':'kernel/src/process/user_word.rs', 'FUTEX':'kernel/src/process/futex.rs'}.items():
            source = source.replace('@'+key+'@', json.dumps(str(ROOT/path)))
        source = source.replace('@EXTRA@',extra)
        with tempfile.TemporaryDirectory() as folder:
            src=Path(folder)/'word.rs'; binary=Path(folder)/'word';src.write_text(source)
            result=subprocess.run([os.environ.get('RUSTC','rustc'),'--edition=2021','-O',str(src),'-o',str(binary)],capture_output=True,text=True)
            if expect_ok:
                self.assertEqual(result.returncode,0,result.stderr)
                subprocess.run([str(binary)],check=True,timeout=30)
            else:
                self.assertNotEqual(result.returncode,0)
                self.assertIn('E0515',result.stderr)

    def test_mapping_and_admission(self):
        self.compile_run()

    def test_word_cannot_outlive_mapping_guard(self):
        self.compile_run("fn escape(space: &IrqSafeSpinLock<AddressSpace>) -> user_word::UserWord<'_> { let guard=space.lock(); user_word::UserWord::new(&guard,4092).unwrap() }",False)


if __name__=='__main__':
    unittest.main()
