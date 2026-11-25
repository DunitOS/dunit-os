//! Pure `no_std + alloc` browser network policy, bounded atomic cache/cookies, and IPC/SHM wire types.
#![no_std]
extern crate alloc;

pub mod cache;
pub mod cookies;
pub mod policy;
pub mod wire;

#[cfg(test)]
mod tests;

