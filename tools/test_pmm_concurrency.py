#!/usr/bin/env python3
"""Exercise the actual PMM with real SpinLock and host-only IRQ/boot shims."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from dunit_cli import ROOT

SOURCE = r'''
#[allow(dead_code)] #[path=@SYNC@] mod actual_sync;
mod sync { pub use crate::actual_sync::{SpinLock as IrqSafeSpinLock,OnceCell}; }
#[allow(dead_code)] mod memory {
    pub fn serial_write(_: &str) {}
    #[path=@PMM@] pub mod pmm;
}
use memory::pmm::{PhysicalMemoryManager as Pmm, PhysicalAddress as Phys, MemRegion};
#[no_mangle] static boot_mem_region_count:u64=2;
#[no_mangle] static boot_mem_regions:[MemRegion;32]={
    let mut map=[MemRegion{base:0,length:0,region_type:1,_pad:0};32];
    map[0]=MemRegion{base:0x100001,length:0x4ffe,region_type:0,_pad:0};
    map[1]=MemRegion{base:0x108000,length:8192,region_type:0,_pad:0};map
};
const PAGE:usize=4096;
fn pool(pages:usize)->Pmm {
    Pmm::new(0x100000,pages*PAGE,Box::leak(vec![0xcc;(pages+7)/8+4].into_boxed_slice()))
}
fn main() {
    if std::env::args().nth(1).as_deref()==Some("bad-capacity") {
        Pmm::new(0x100000,9*PAGE,Box::leak(vec![0;1].into_boxed_slice()));return;
    }
    assert!(memory::pmm::get_pmm().is_none());
    let p=pool(9);
    let frames:Vec<_>=(0..9).map(|_|p.alloc_frame().unwrap()).collect();
    assert_eq!(frames.iter().map(|f|f.0).collect::<Vec<_>>(),(0..9).map(|n|0x100000+n*PAGE).collect::<Vec<_>>());
    assert!(p.alloc_frame().is_none());assert_eq!(p.stats_bytes(),(9*PAGE as u64,0));
    for bad in [0,0x100001,0x100000+9*PAGE] {p.free_frame(Phys(bad));}
    assert_eq!(p.available_memory(),0);
    p.free_frame(frames[8]);p.free_frame(frames[8]);assert_eq!(p.available_memory(),PAGE);
    assert_eq!(p.alloc_contiguous(1).unwrap(),frames[8]);
    assert!(p.alloc_contiguous(0).is_none());assert!(p.alloc_contiguous(10).is_none());
    for f in frames {p.free_frame(f);}
    assert_eq!(p.available_memory(),9*PAGE);
    let block=p.alloc_contiguous(9).unwrap();assert_eq!(block,Phys(0x100000));
    assert!(p.alloc_frame().is_none());for n in 0..9 {p.free_frame(Phys(block.0+n*PAGE));}

    // Boot map retains only complete usable pages and never bridges a hole.
    assert!(memory::pmm::init());let global=memory::pmm::get_pmm().unwrap();
    assert_eq!(global.stats_bytes(),(5*PAGE as u64,5*PAGE as u64));
    let boot_frames:Vec<_>=(0..5).map(|_|global.alloc_frame().unwrap()).collect();
    assert_eq!(boot_frames.iter().map(|f|f.0).collect::<Vec<_>>(),[0x101000,0x102000,0x103000,0x108000,0x109000]);
    assert!(global.alloc_frame().is_none());
    let repeats:Vec<_>=(0..8).map(|_|std::thread::spawn(||assert!(memory::pmm::init()))).collect();
    for t in repeats {t.join().unwrap();}assert_eq!(global.available_memory(),0);
    memory::pmm::init_pmm(0x200000,PAGE,Box::leak(vec![0;1].into_boxed_slice()));
    assert!(core::ptr::eq(global,memory::pmm::get_pmm().unwrap()));
    for f in boot_frames {global.free_frame(f);}
    assert!(global.alloc_contiguous(4).is_none());assert_eq!(global.available_memory(),5*PAGE);
    let block=global.alloc_contiguous(3).unwrap();assert_eq!(block,Phys(0x101000));
    for n in 0..3 {global.free_frame(Phys(block.0+n*PAGE));}

    use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
    const PAGES:usize=257;const LOOPS:usize=10000;
    let p=Arc::new(pool(PAGES));
    let live=Arc::new((0..PAGES).map(|_|AtomicUsize::new(0)).collect::<Vec<_>>());
    let total=Arc::new(AtomicUsize::new(0));
    let workers:Vec<_>=(1..=8).map(|tag| {
        let p=p.clone();let live=live.clone();let total=total.clone();
        std::thread::spawn(move|| {
            for round in 0..LOOPS {
                let count=if round%2==0 {1} else {2+round%6};
                let first=p.alloc_contiguous(count).expect("unexpected exhaustion");
                let start=(first.0-0x100000)/PAGE;assert!(start+count<=PAGES);
                for index in start..start+count {assert_eq!(live[index].compare_exchange(0,tag,Ordering::SeqCst,Ordering::SeqCst),Ok(0),"duplicate live physical frame");}
                let (all,free)=p.stats_bytes();assert_eq!(all,(PAGES*PAGE) as u64);assert!(free<=all);
                std::thread::yield_now();
                for index in start..start+count {
                    assert_eq!(live[index].swap(0,Ordering::SeqCst),tag);
                    p.free_frame(Phys(0x100000+index*PAGE));
                }
                total.fetch_add(count,Ordering::SeqCst);
            }
        })
    }).collect();
    for t in workers {t.join().unwrap();}
    assert_eq!(p.available_memory(),PAGES*PAGE);assert!(live.iter().all(|slot|slot.load(Ordering::SeqCst)==0));
    let expected:usize=(0..LOOPS).map(|round|if round%2==0 {1} else {2+round%6}).sum::<usize>()*8;
    assert_eq!(total.load(Ordering::SeqCst),expected);
    println!("PMM: bounds/tail/alignment/OOM/holes/reinit; 80000 mixed transactions, {expected} frames unique/reclaimed OK");
}
'''


class PmmConcurrency(unittest.TestCase):
    def test_actual_allocator(self):
        source=SOURCE.replace('@SYNC@',json.dumps(str(ROOT/'kernel/src/sync.rs'))).replace('@PMM@',json.dumps(str(ROOT/'kernel/src/memory/pmm.rs')))
        with tempfile.TemporaryDirectory() as folder:
            src=Path(folder)/'pmm.rs';binary=Path(folder)/'pmm';src.write_text(source)
            result=subprocess.run([os.environ.get('RUSTC','rustc'),'--edition=2021','-O',str(src),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            subprocess.run([str(binary)],check=True,timeout=60)
            invalid=subprocess.run([str(binary),'bad-capacity'],capture_output=True,text=True)
            self.assertNotEqual(invalid.returncode,0)
            self.assertIn('physical pool bitmap too small',invalid.stderr)


if __name__=='__main__':
    unittest.main()
