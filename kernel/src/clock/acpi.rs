//! Bounded, checksum-validated ACPI discovery. Physical reads are supplied by
//! the boot mapper, so no firmware pointer becomes a static Rust reference.
use alloc::vec::Vec;
const HEADER: usize = 36;
const TABLE_LIMIT: usize = 1024 * 1024;
fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}
fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}
fn checksum(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) == 0
}
fn table(read: &mut impl FnMut(u64, usize) -> Option<Vec<u8>>, addr: u64) -> Option<Vec<u8>> {
    let header = read(addr, HEADER)?;
    let length = u32_at(&header, 4)? as usize;
    if !(HEADER..=TABLE_LIMIT).contains(&length) {
        return None;
    }
    let bytes = read(addr, length)?;
    if bytes.len() != length || !checksum(&bytes) {
        return None;
    }
    Some(bytes)
}
/// Accept ACPI 1 RSDT and ACPI 2+ XSDT. Invalid/missing firmware is a fallback,
/// not an excuse to guess a platform's HPET physical address.
pub(super) fn hpet_address(
    rsdp: u64,
    mut read: impl FnMut(u64, usize) -> Option<Vec<u8>>,
) -> Option<u64> {
    let base = read(rsdp, 20)?;
    if base.get(..8)? != b"RSD PTR " || !checksum(&base) {
        return None;
    }
    let mut root = u32_at(&base, 16)? as u64;
    let mut stride = 4;
    if *base.get(15)? >= 2 {
        let header = read(rsdp, HEADER)?;
        let length = u32_at(&header, 20)? as usize;
        if !(HEADER..=4096).contains(&length) {
            return None;
        }
        let extended = read(rsdp, length)?;
        if extended.len() != length || !checksum(&extended) {
            return None;
        }
        let xsdt = u64_at(&extended, 24)?;
        if xsdt != 0 {
            root = xsdt;
            stride = 8;
        }
    }
    if root == 0 {
        return None;
    }
    let root = table(&mut read, root)?;
    if &root[..4] != if stride == 8 { b"XSDT" } else { b"RSDT" }
        || (root.len() - HEADER) % stride != 0
    {
        return None;
    }
    for pointer in root[HEADER..].chunks_exact(stride) {
        let addr = if stride == 8 {
            u64_at(pointer, 0)?
        } else {
            u32_at(pointer, 0)? as u64
        };
        let Some(header) = read(addr, HEADER) else {
            continue;
        };
        if header.get(..4) != Some(b"HPET") {
            continue;
        }
        let bytes = table(&mut read, addr)?;
        // Generic address: system memory, zero bit offset. HPET register
        // accesses have their own fixed widths defined by the hardware spec.
        if bytes.len() < 56 || bytes[40] != 0 || bytes[42] != 0 {
            return None;
        }
        let address = u64_at(&bytes, 44)?;
        if address == 0 || address & 7 != 0 || address >= (1u64 << 52) {
            return None;
        }
        return Some(address);
    }
    None
}
