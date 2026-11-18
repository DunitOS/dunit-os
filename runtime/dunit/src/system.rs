//! Facts already exposed by the native kernel; no invented platform identity.
pub use libdunit::SystemStats;
pub fn stats() -> crate::Result<SystemStats> {
    let mut stats = SystemStats::default();
    crate::error::checked(libdunit::get_system_stats(&mut stats))?;
    Ok(stats)
}

/// Display metadata from the existing framebuffer query, without exposing its
/// mapping address. Absence remains absence; no display geometry is fabricated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}
pub fn display_info() -> Option<DisplayInfo> {
    let mut info = libdunit::FbInfo {
        addr: 0,
        width: 0,
        height: 0,
        pitch: 0,
    };
    libdunit::get_framebuffer(&mut info).then_some(DisplayInfo {
        width: info.width,
        height: info.height,
        pitch: info.pitch,
    })
}

/// Render an existing privileged terminal diagnostic into caller-owned storage.
/// Native capability checks and command validation remain authoritative.
pub fn terminal_diagnostic(command: &str, output: &mut [u8]) -> crate::Result<usize> {
    let count = crate::error::checked(libdunit::terminal_diag(command, output))?;
    if count > output.len() {
        return Err(crate::Error(libdunit::EIO));
    }
    Ok(count)
}
