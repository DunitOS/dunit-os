//! Facts already exposed by the native kernel; no invented platform identity.
pub use libdunit::SystemStats;
pub fn stats() -> crate::Result<SystemStats> {
    let mut stats = SystemStats::default();
    crate::error::checked(libdunit::get_system_stats(&mut stats))?;
    Ok(stats)
}
