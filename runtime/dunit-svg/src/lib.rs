//! Bounded, static SVG rendering for the no_std userspace GUI.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
mod css;
mod filter;
mod geometry;
mod paint;
mod render;
pub use render::render;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Xml,
    Invalid,
    Unsupported,
    Limit,
    Allocation,
}

pub(crate) type Result<T> = core::result::Result<T, Error>;
pub(crate) const MAX_NODES: u32 = 4096;
pub(crate) const MAX_DEPTH: usize = 32;
pub(crate) const MAX_SEGMENTS: usize = 16384;

pub(crate) fn number(s: &str) -> Result<f32> {
    let n: f32 = s.trim().parse().map_err(|_| Error::Invalid)?;
    if n.is_finite() && n.abs() <= 1e7 {
        Ok(n)
    } else {
        Err(Error::Invalid)
    }
}

pub(crate) fn length(s: &str, axis: f32) -> Result<f32> {
    let l: svgtypes::Length = s.parse().map_err(|_| Error::Invalid)?;
    let n = l.number as f32;
    if !n.is_finite() || n.abs() > 1e7 {
        return Err(Error::Invalid);
    }
    match l.unit {
        svgtypes::LengthUnit::None | svgtypes::LengthUnit::Px => Ok(n),
        svgtypes::LengthUnit::Percent => Ok(n * axis / 100.0),
        _ => Err(Error::Unsupported),
    }
}
