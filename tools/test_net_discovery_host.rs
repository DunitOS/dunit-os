//! Actual production discovery path must not grant or restore firmware DMA.
#![allow(dead_code)]
use std::sync::{Mutex,atomic::{AtomicBool,AtomicU16,AtomicUsize,Ordering}};
static COMMAND:AtomicU16=AtomicU16::new(0);
static WRITES:Mutex<Vec<u16>>=Mutex::new(Vec::new());
static MMIO:AtomicUsize=AtomicUsize::new(0);
static MAP_FAIL:AtomicBool=AtomicBool::new(false);
static UNMAPS:AtomicUsize=AtomicUsize::new(0);
fn serial_write(_: &str) {}
mod serial {
    pub fn write_dec(_:u64){} pub fn write_hex(_:u64,_:usize){} pub fn write_hex16(_:u16){}
    pub fn write_hex8(_:u8){} pub fn write_mac(_:u32,_:u32){}
}
mod sync {
    pub struct SpinLock<T>(std::sync::Mutex<T>);
    impl<T> SpinLock<T>{pub const fn new(value:T)->Self{Self(std::sync::Mutex::new(value))} pub fn lock(&self)->std::sync::MutexGuard<'_,T>{self.0.lock().unwrap()}}
}
mod drivers {
    pub mod registry {pub enum DeviceClass{Network}pub fn register(_: &str,_:DeviceClass,_:&str){}}
    pub mod pci {
        use crate::{COMMAND,WRITES,Ordering};
        #[derive(Clone,Copy)]pub struct PciDevice{pub bus:u8,pub device:u8,pub function:u8,pub class_code:u8,pub vendor_id:u16,pub device_id:u16}
        pub enum PciBar{Memory32(u32),Memory64(u64),Io(u32),None}
        pub fn scan(mut callback:impl FnMut(PciDevice)){callback(PciDevice{bus:0,device:3,function:0,class_code:2,vendor_id:0x8086,device_id:0x100e})}
        pub fn read_bar_decoded(_:PciDevice,index:u8)->PciBar{if index==0{PciBar::Memory32(0x100000)}else{PciBar::None}}
        pub fn command(_:PciDevice)->u16{COMMAND.load(Ordering::SeqCst)}
        pub fn set_command(_:PciDevice,value:u16){WRITES.lock().unwrap().push(value);COMMAND.store(value,Ordering::SeqCst);}
    }
}
mod memory {
    pub mod vmm {
        use crate::{MMIO,MAP_FAIL,UNMAPS,COMMAND,Ordering};
        pub fn map_mmio_region(_:usize,_:usize)->Option<usize>{assert_eq!(COMMAND.load(Ordering::SeqCst)&4,0,"discovery must not enable DMA");if MAP_FAIL.load(Ordering::SeqCst){None}else{Some(MMIO.load(Ordering::SeqCst))}}
        pub fn unmap_mmio_region(_:usize,_:usize)->bool{assert_eq!(COMMAND.load(Ordering::SeqCst)&4,0);UNMAPS.fetch_add(1,Ordering::SeqCst);true}
    }
}
#[path="../kernel/src/drivers/net.rs"]mod net;
#[test]
fn discovery_maps_only_registers_and_never_restores_ambient_bus_master() {
    let mut memory=vec![0u32;0x20000/4];
    MMIO.store(memory.as_mut_ptr() as usize,Ordering::SeqCst);
    for (status,map_failure,success) in [(2,false,true),(u32::MAX,false,false),(2,true,false)] {
        memory[8/4]=status;memory[0x5400/4]=0x12005452;memory[0x5404/4]=0x80005634;
        COMMAND.store(0x107,Ordering::SeqCst);WRITES.lock().unwrap().clear();UNMAPS.store(0,Ordering::SeqCst);MAP_FAIL.store(map_failure,Ordering::SeqCst);
        net::init();let snapshot=net::snapshot();assert_eq!(snapshot.total_nics,1);assert_eq!(snapshot.supported_nics,1);
        assert_eq!(snapshot.mmio_ready_nics,usize::from(success));assert_eq!(snapshot.mac_ready_nics,usize::from(success));
        assert_eq!(COMMAND.load(Ordering::SeqCst),0x103);
        assert!(WRITES.lock().unwrap().iter().all(|command|command&4==0));
        assert_eq!(UNMAPS.load(Ordering::SeqCst),usize::from(!map_failure));
    }
}
