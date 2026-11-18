//! Read-only DunitFS integrity diagnostics over the existing native fsck ABI.
use crate::{error::checked, Result};

/// Native recovery report. Counters and verdict codes are preserved verbatim,
/// including unknown verdicts from a newer filesystem checker.
pub use libdunit::FsckReport as Report;

/// Check the mounted Dunit partition without repairing or writing metadata.
/// An absent partition and native failures remain typed SDK errors.
pub fn check() -> Result<Report> {
    let mut report = Report::default();
    checked(libdunit::fsck(&mut report))?;
    Ok(report)
}
