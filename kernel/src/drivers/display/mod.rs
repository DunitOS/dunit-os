//! Display ownership, scanout geometry and backend programming.
//! Display policy stays in userspace. All pixel access and modesets use STATE;
//! no callback may take a task/VM lock or retain a framebuffer pointer.

use crate::sync::{InterruptGuard, SpinLock};

pub mod bochs;

#[derive(Clone, Copy)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayError {
    Unsupported,
    InvalidMode,
    Io,
    AccessDenied,
    Unavailable,
    DeviceLost,
}

/// Kernel mapping descriptor. Copying it grants no pixel access; use the
/// scoped callbacks below while its owner and geometry are locked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Framebuffer {
    pub address: usize,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}

impl Framebuffer {
    pub(super) fn byte_len(self) -> Option<usize> {
        (self.pitch as usize).checked_mul(self.height as usize)
    }
}

enum Backend {
    Limine,
    Bochs(bochs::BochsVbe),
}

struct DisplayState {
    owner: u64,
    leases: usize,
    backend_initialized: bool,
    framebuffer: Option<Framebuffer>,
    backend: Backend,
}

// IRQ handlers never acquire this lock, and timer preemption only switches
// CPL3. Kernel console rendering can therefore leave interrupts enabled.
// Task/AS -> display is permitted; display -> task/AS is prohibited.
static STATE: SpinLock<DisplayState> = SpinLock::new(DisplayState {
    owner: 0,
    leases: 0,
    backend_initialized: false,
    framebuffer: None,
    backend: Backend::Limine,
});

/// # Safety
/// The boot mapping must remain writable for the kernel lifetime. Call once
/// before device initialization, AP work or application dispatch.
pub unsafe fn init_boot(address: usize, width: u32, height: u32, pitch: u32) -> bool {
    let framebuffer = Framebuffer {
        address,
        width,
        height,
        pitch,
    };
    if address == 0
        || address & 3 != 0
        || pitch & 3 != 0
        || width == 0
        || height == 0
        || width.checked_mul(4).is_none_or(|bytes| bytes > pitch)
        || framebuffer
            .byte_len()
            .and_then(|bytes| address.checked_add(bytes))
            .is_none()
    {
        return false;
    }
    let mut state = STATE.lock();
    assert!(
        state.framebuffer.is_none(),
        "boot framebuffer initialized twice"
    );
    state.framebuffer = Some(framebuffer);
    true
}

/// Device detection and full-VRAM mapping happen once, before userspace. The
/// selected backend owns its MMIO mapping for the entire published lifetime.
pub fn init() {
    let boot = {
        let mut state = STATE.lock();
        assert!(
            !state.backend_initialized,
            "display backend initialized twice"
        );
        state.backend_initialized = true;
        state.framebuffer
    };
    let Some(boot) = boot else {
        crate::serial_write("[DISPLAY] no boot framebuffer — backend: none\r\n");
        return;
    };
    if let Some(device) = bochs::BochsVbe::detect(boot) {
        let framebuffer = device.boot_framebuffer(boot);
        let mut state = STATE.lock();
        state.framebuffer = Some(framebuffer);
        state.backend = Backend::Bochs(device);
        drop(state);
        crate::serial_write("[DISPLAY] backend: bochs-vbe\r\n");
    } else {
        crate::serial_write("[DISPLAY] backend: limine (fixed)\r\n");
    }
}

pub fn owner() -> u64 {
    STATE.lock().owner
}

pub fn try_acquire(pid: u64) -> bool {
    if pid == 0 {
        return false;
    }
    let mut state = STATE.lock();
    if state.owner != 0 && state.owner != pid {
        return false;
    }
    let Some(leases) = state.leases.checked_add(1) else {
        return false;
    };
    state.leases = leases;
    state.owner = pid;
    true
}

pub fn release(pid: u64) {
    let mut state = STATE.lock();
    if pid != 0 && state.owner == pid {
        state.leases -= 1;
        if state.leases == 0 {
            state.owner = 0;
        }
    }
}

pub(crate) fn framebuffer_snapshot() -> Option<Framebuffer> {
    STATE.lock().framebuffer
}

pub(crate) fn get_framebuffer(pid: u64) -> Result<Framebuffer, DisplayError> {
    with_framebuffer(pid, |framebuffer| *framebuffer)
}

pub(crate) fn with_framebuffer<R>(
    pid: u64,
    op: impl FnOnce(&Framebuffer) -> R,
) -> Result<R, DisplayError> {
    let state = STATE.lock();
    if pid == 0 || state.owner != pid {
        return Err(DisplayError::AccessDenied);
    }
    Ok(op(state
        .framebuffer
        .as_ref()
        .ok_or(DisplayError::Unavailable)?))
}

pub(crate) fn with_console_framebuffer(op: impl FnOnce(&Framebuffer)) {
    let state = STATE.lock();
    if state.owner != 0 {
        return;
    }
    if let Some(framebuffer) = state.framebuffer.as_ref() {
        op(framebuffer);
    }
}

pub fn query_modes(out: &mut [Mode]) -> usize {
    let state = STATE.lock();
    match &state.backend {
        Backend::Limine => {
            let Some(framebuffer) = state.framebuffer else {
                return 0;
            };
            if out.is_empty() {
                return 0;
            }
            out[0] = Mode {
                width: framebuffer.width,
                height: framebuffer.height,
            };
            1
        }
        Backend::Bochs(device) => device.query_modes(out),
    }
}

pub fn set_mode(pid: u64, width: u32, height: u32) -> Result<(), DisplayError> {
    // Mask PS/2 only for programming/publication. The callback cannot wait on
    // an IRQ, task or VM owner. No serial output runs inside this scope.
    {
        let mut state = STATE.lock();
        let _irq = InterruptGuard::new();
        if pid == 0 || state.owner != pid {
            return Err(DisplayError::AccessDenied);
        }
        let result = match &mut state.backend {
            Backend::Limine => return Err(DisplayError::Unsupported),
            Backend::Bochs(device) => device
                .set_mode(width, height)
                .map(|()| device.framebuffer(width, height)),
        };
        match result {
            Ok(framebuffer) => {
                state.framebuffer = Some(framebuffer);
                crate::input::set_mouse_bounds(width as usize, height as usize);
            }
            Err(DisplayError::DeviceLost) => {
                // Failed rollback leaves hardware geometry unknown. Retain the
                // device mapping, refuse all pixel access until a valid modeset.
                state.framebuffer = None;
                crate::input::set_mouse_bounds(0, 0);
                return Err(DisplayError::DeviceLost);
            }
            Err(error) => return Err(error),
        }
    }
    crate::serial_write("[DISPLAY] bochs set_mode ");
    crate::serial::write_dec(width as u64);
    crate::serial_write("x");
    crate::serial::write_dec(height as u64);
    crate::serial_write("\r\n");
    Ok(())
}

/// Copy and backend publication share the same owner/geometry scope. Source
/// validation and its VM pin must already exist before entering this callback.
pub(crate) fn present(
    pid: u64,
    rect: Option<(u32, u32, u32, u32)>,
    copy: impl FnOnce(&Framebuffer),
) -> Result<(), DisplayError> {
    let mut state = STATE.lock();
    if pid == 0 || state.owner != pid {
        return Err(DisplayError::AccessDenied);
    }
    copy(
        state
            .framebuffer
            .as_ref()
            .ok_or(DisplayError::Unavailable)?,
    );
    match &mut state.backend {
        Backend::Limine => Ok(()),
        Backend::Bochs(device) => device.present(rect),
    }
}
