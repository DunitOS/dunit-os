//! Owned UTF-8 arguments from the loader-provided, initialized entry stack.
use crate::{Error, Result};
use alloc::{string::String, vec::Vec};
pub fn args() -> Result<Vec<String>> {
    (0..libdunit::argc())
        .map(|index| {
            libdunit::arg(index)
                .map(String::from)
                .ok_or(Error(libdunit::EINVAL))
        })
        .collect()
}
