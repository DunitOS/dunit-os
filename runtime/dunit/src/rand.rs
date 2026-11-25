//! Native kernel CSPRNG bindings over `SYSCALL_GET_RANDOM`.
use crate::{error::checked, Result};

pub fn fill_bytes(mut out: &mut [u8]) -> Result<()> {
    while !out.is_empty() {
        let got = checked(libdunit::get_random(out, 0))?;
        if got == 0 || got > out.len() {
            return Err(crate::Error(libdunit::EIO));
        }
        out = &mut out[got..];
    }
    Ok(())
}

pub fn bytes<const N: usize>() -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    fill_bytes(&mut buf)?;
    Ok(buf)
}

pub fn next_u64() -> Result<u64> {
    Ok(u64::from_le_bytes(bytes::<8>()?))
}

