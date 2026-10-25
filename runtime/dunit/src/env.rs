//! Process-entry environment. Mutation is deliberately not exposed.
use crate::{Error, Result};
use alloc::string::String;
pub fn var(key: &str) -> Result<Option<String>> {
    if key.is_empty() || key.contains(['=', '\0']) {
        return Err(Error(libdunit::EINVAL));
    }
    Ok(libdunit::getenv(key).map(String::from))
}
