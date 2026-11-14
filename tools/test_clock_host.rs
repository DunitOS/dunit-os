extern crate alloc;
mod sync { pub use std::sync::OnceLock as OnceCell; }
mod memory {
    pub fn serial_write(_: &str) {}
    pub mod vmm {
        pub fn map_firmware_region(_:usize,_:usize)->Option<usize> {None}
        pub fn map_mmio_region(_:usize,_:usize)->Option<usize> {None}
        pub fn unmap_mmio_region(_:usize,_:usize)->bool {true}
        pub fn virt_to_phys(value:usize)->usize {value}
    }
}
struct FmtBuf(String);
impl FmtBuf {
    fn new()->Self {Self(String::new())}
    fn push_str(&mut self,s:&str) {self.0.push_str(s);}
    fn push_u64(&mut self,n:u64) {self.0.push_str(&n.to_string());}
    fn as_str(&self)->&str {&self.0}
}
#[no_mangle] extern "C" fn hal_rsdp_address()->u64 {0}
use std::sync::atomic::{AtomicI64,Ordering};
static EPOCH: AtomicI64=AtomicI64::new(-1);
mod hal {
    pub unsafe fn hal_boot_epoch_seconds()->i64 {super::EPOCH.load(super::Ordering::Relaxed)}
    pub unsafe fn hal_outb(_:u16,_:u8) {}
}
#[path="../kernel/src/clock.rs"] mod clock;
#[test]
fn utc_validity_and_monotonic_progress_are_distinct() {
    clock::init_pit();assert!(clock::realtime().is_none());
    EPOCH.store(1720000000,Ordering::Relaxed);clock::init_pit();
    assert_eq!(clock::realtime(),Some((1720000000,0)));
    let deadline=clock::Deadline::after_ms(10);assert!(!deadline.expired());
    for _ in 0..101 {clock::on_tick();}
    assert!(deadline.expired());assert_eq!(deadline.tick(),1);
    assert_eq!(clock::monotonic_ns(),1010000000);
    assert_eq!(clock::realtime(),Some((1720000001,10000000)));
}

#[path="../kernel/src/clock/acpi.rs"] mod acpi;
#[path="../kernel/src/clock/hpet.rs"] mod hpet;
#[test]
fn hpet_conversion_is_wide_and_saturating() {
    assert_eq!(hpet::counter_to_ns(1,10_000_000),10);
    assert_eq!(hpet::counter_to_ns(123_456_789,10_000_000),1_234_567_890);
    assert_eq!(hpet::counter_to_ns(u64::MAX,100_000_000),u64::MAX);
}
fn sum_fix(bytes:&mut [u8],offset:usize) {
    bytes[offset]=0;let sum=bytes.iter().fold(0u8,|a,b|a.wrapping_add(*b));bytes[offset]=0u8.wrapping_sub(sum);
}
fn sdt(signature:&[u8;4],size:usize)->Vec<u8> {
    let mut bytes=vec![0;size];bytes[..4].copy_from_slice(signature);bytes[4..8].copy_from_slice(&(size as u32).to_le_bytes());bytes
}
#[test]
fn acpi_discovery_checks_root_and_device_without_guessing_addresses() {
    for xsdt in [false,true] {
        let mut tables=std::collections::BTreeMap::new();
        let mut rsdp=vec![0;36];rsdp[..8].copy_from_slice(b"RSD PTR ");rsdp[16..20].copy_from_slice(&0x2000u32.to_le_bytes());
        if xsdt {rsdp[15]=2;rsdp[20..24].copy_from_slice(&36u32.to_le_bytes());rsdp[24..32].copy_from_slice(&0x2000u64.to_le_bytes());}
        sum_fix(&mut rsdp[..20],8);if xsdt {sum_fix(&mut rsdp,32);}
        let mut root=sdt(if xsdt {b"XSDT"} else {b"RSDT"},if xsdt {44} else {40});
        if xsdt {root[36..44].copy_from_slice(&0x3000u64.to_le_bytes());} else {root[36..40].copy_from_slice(&0x3000u32.to_le_bytes());}sum_fix(&mut root,9);
        let mut hpet=sdt(b"HPET",56);hpet[41]=64;hpet[44..52].copy_from_slice(&0xfed01230u64.to_le_bytes());sum_fix(&mut hpet,9);
        tables.insert(0x1000,rsdp);tables.insert(0x2000,root);tables.insert(0x3000,hpet);
        let find=|tables:&std::collections::BTreeMap<u64,Vec<u8>>| acpi::hpet_address(0x1000,|addr,size|Some(tables.get(&addr)?.get(..size)?.to_vec()));
        assert_eq!(find(&tables),Some(0xfed01230));
        tables.get_mut(&0x3000).unwrap()[40]=1;sum_fix(tables.get_mut(&0x3000).unwrap(),9);assert_eq!(find(&tables),None);
        tables.get_mut(&0x3000).unwrap()[40]=0;sum_fix(tables.get_mut(&0x3000).unwrap(),9);
        tables.get_mut(&0x2000).unwrap()[9]^=1;assert_eq!(find(&tables),None);
        tables.get_mut(&0x2000).unwrap()[9]^=1;
        tables.get_mut(&0x1000).unwrap()[8]^=1;assert_eq!(find(&tables),None);
    }
}
