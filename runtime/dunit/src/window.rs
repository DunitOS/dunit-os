//! One top-level window per process, over the existing compositor bootstrap.
//! This session owns the process IPC inbox. Do not receive IPC independently.
//! Drawing uses a fresh buffer: published pixels are never mutated again.
use crate::{
    error::checked,
    event::{EventLoop, Interest},
    time::Instant,
    Error, Result,
};
use alloc::{collections::VecDeque, vec::Vec};
use core::{
    marker::PhantomData,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
pub use gui_protocol_v1::control::{
    Input, IN_DOWN, IN_KEY, IN_LEAVE, IN_MOVE, IN_QUIT, IN_SCROLL, IN_UP,
};
pub use gui_protocol_v1::wire::Event as ProtocolEvent;
use gui_protocol_v1::{
    control::{Bootstrap, Capability},
    wire::*,
};

const TRANSACTION: Duration = Duration::from_secs(5);
const MAX_RETRIES: usize = 8;
const MAX_DEFERRED: usize = 128;
static STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowEvent {
    Resized { width: u32, height: u32 },
    Input(Input),
    CloseRequested,
    Protocol(Event),
}

/// A private, writable frame before publication. Pixels are XRGB8888 words.
/// The borrow cannot outlive the drawing callback or alias a committed buffer.
pub struct Frame<'a> {
    pixels: &'a mut [u32],
    width: u32,
    height: u32,
}
impl Frame<'_> {
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        self.pixels
    }
}
struct Buffer {
    handle: u32,
    address: usize,
    bytes: usize,
    object: u64,
}
impl Buffer {
    fn new(width: u32, height: u32, object: u64) -> Result<Self> {
        let bytes = buffer_bytes(width, height)?;
        let handle = checked(libdunit::handle_create_shared(bytes))? as u32;
        let mut buffer = Self {
            handle,
            address: 0,
            bytes,
            object,
        };
        buffer.address = checked(libdunit::handle_map(handle, 0, bytes))?;
        if buffer.address == 0 {
            return Err(Error(libdunit::EFAULT));
        }
        Ok(buffer)
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        libdunit::shared_buffer::release(self.handle, self.address, self.bytes);
    }
}
fn buffer_bytes(width: u32, height: u32) -> Result<usize> {
    if !(DIM_MIN..=DIM_MAX).contains(&width) || !(DIM_MIN..=DIM_MAX).contains(&height) {
        return Err(Error(libdunit::EINVAL));
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or(Error(libdunit::EINVAL))
}
/// A single IPC session. `open` is supported once per process, including after
/// close/failure: existing gui-v1 bootstrap has no reconnect/session identifier.
/// This type is deliberately neither Send nor Sync (one inbox and serial stream).
pub struct Window {
    surface: u64,
    live_buffers: Vec<u64>,
    protocol_error: Option<(u64, u64, u32, u32)>,
    resized: bool,
    peer: u32,
    serial: u64,
    server_serial: u64,
    configure: u64,
    width: u32,
    height: u32,
    next_object: u64,
    current: Option<Buffer>,
    pending: VecDeque<(Header, Event)>,
    inputs: VecDeque<Input>,
    released: Vec<u64>,
    waiter: EventLoop,
    failed: bool,
    _single_thread: PhantomData<*mut ()>,
}
impl Window {
    /// The compositor launches the process and sends the existing 8-byte
    /// bootstrap. Its claimed PID must equal the kernel-authenticated sender.
    /// Subsequent messages are accepted only from that sender.
    pub fn open(width: u32, height: u32) -> Result<Self> {
        buffer_bytes(width, height)?;
        if STARTED.swap(true, Ordering::AcqRel) {
            return Err(Error(libdunit::EBUSY));
        }
        let mut window = Self {
            surface: 1,
            live_buffers: Vec::new(),
            protocol_error: None,
            resized: false,
            peer: 0,
            serial: 0,
            server_serial: 0,
            configure: 0,
            width,
            height,
            next_object: 2,
            current: None,
            pending: VecDeque::new(),
            inputs: VecDeque::new(),
            released: Vec::new(),
            waiter: EventLoop::new(&[Interest::ipc(1)])?,
            failed: false,
            _single_thread: PhantomData,
        };
        let mut bytes = [0; MAX_PACKET];
        let start = Instant::now()?;
        loop {
            let (n, sender) = window
                .receive(&mut bytes, start, TRANSACTION)?
                .ok_or(Error(libdunit::EAGAIN))?;
            if Bootstrap::decode(&bytes[..n])
                .is_some_and(|intro| intro.compositor == sender && sender != 0)
            {
                window.peer = sender;
                break;
            }
        }
        let hello = window.send(
            0,
            Request::Hello {
                min_minor: 0,
                max_minor: 0,
                offered_features: 0,
                required_features: 0,
            },
        )?;
        let (_, welcome) = window
            .until(|_, e| matches!(e, Event::Welcome { request, .. } if *request == hello))?;
        if !matches!(welcome, Event::Welcome { formats, .. } if formats & FORMATS_XRGB8888 != 0) {
            return Err(Error(libdunit::EOPNOTSUPP));
        }
        window.request(
            window.surface,
            Request::CreateSurface {
                role: ROLE_TOPLEVEL,
                width,
                height,
                format: FORMAT_XRGB8888,
            },
        )?;
        let surface = window.surface;
        let (header, event) =
            window.until(|h, e| h.object == surface && matches!(e, Event::Configure { .. }))?;
        window.configure(header, event)?;
        Ok(window)
    }
    /// Complete the existing disconnect exchange and release local mappings.
    pub fn close(mut self) -> Result<()> {
        self.request(0, Request::Disconnect)?;
        self.peer = 0;
        Ok(())
    }
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn send(&mut self, object: u64, request: Request) -> Result<u64> {
        if self.failed {
            return Err(Error(libdunit::EPIPE));
        }
        self.serial = self.serial.checked_add(1).ok_or(Error(libdunit::EINVAL))?;
        self.protocol_error = None;
        let packet = request.encode(object, self.serial);
        checked(libdunit::ipc_send(self.peer, &packet))?;
        Ok(self.serial)
    }
    fn request(&mut self, object: u64, request: Request) -> Result<()> {
        let serial = self.send(object, request)?;
        self.until(|h, e| {
            h.object == object && matches!(e, Event::Result { request } if *request == serial)
        })?;
        Ok(())
    }
    fn receive(
        &mut self,
        bytes: &mut [u8],
        start: Instant,
        timeout: Duration,
    ) -> Result<Option<(usize, u32)>> {
        loop {
            // Check before consuming too: a continuous stream of malformed
            // bootstrap packets must not keep a transaction alive forever.
            if !timeout.is_zero() && start.elapsed()? >= timeout {
                return Ok(None);
            }
            let mut sender = 0;
            let n = libdunit::ipc_recv_from(bytes, &mut sender);
            if n >= 0 && (self.peer == 0 || sender == self.peer) {
                return Ok(Some((n as usize, sender)));
            }
            if n < 0 && n != libdunit::EAGAIN {
                return Err(Error(n));
            }
            let elapsed = start.elapsed()?;
            if elapsed >= timeout {
                return Ok(None);
            }
            // Foreign traffic never extends the deadline or becomes an event.
            if n == libdunit::EAGAIN {
                self.waiter
                    .wait(crate::time::timeout_ns(timeout - elapsed)?)?;
            }
        }
    }
    fn packet(&mut self, start: Instant, timeout: Duration) -> Result<Option<(Header, Event)>> {
        let mut bytes = [0; MAX_PACKET];
        loop {
            let Some((n, _)) = self.receive(&mut bytes, start, timeout)? else {
                return Ok(None);
            };
            if let Some(input) = Input::decode(&bytes[..n]) {
                if self.inputs.len() == MAX_DEFERRED {
                    return Err(Error(libdunit::ENOBUFS));
                }
                self.inputs.push_back(input);
                // Let next_event return input promptly, including a quit.
                return Ok(None);
            }
            let header = Header::decode(&bytes[..n]).map_err(|_| Error(libdunit::EIO))?;
            if header.major != VERSION_MAJOR
                || header.minor != VERSION_MINOR
                || header.serial <= self.server_serial
            {
                return Err(Error(libdunit::EIO));
            }
            self.server_serial = header.serial;
            let opcode = Opcode::from_u16(header.opcode_raw).ok_or(Error(libdunit::EIO))?;
            let event =
                Event::decode(opcode, &bytes[HEADER_LEN..n]).map_err(|_| Error(libdunit::EIO))?;
            if let Event::Error {
                request,
                code,
                fatal,
            } = event
            {
                self.protocol_error = Some((header.object, request, code, fatal));
                return Err(Error(libdunit::EIO));
            }
            if matches!(event, Event::BufferRelease { .. }) {
                if self.released.len() == MAX_DEFERRED {
                    return Err(Error(libdunit::ENOBUFS));
                }
                self.released.push(header.object);
                continue;
            }
            return Ok(Some((header, event)));
        }
    }
    fn until(&mut self, matches: impl Fn(&Header, &Event) -> bool) -> Result<(Header, Event)> {
        if let Some(i) = self.pending.iter().position(|(h, e)| matches(h, e)) {
            return Ok(self.pending.remove(i).unwrap());
        }
        let start = Instant::now()?;
        loop {
            if let Some((header, event)) = self.packet(start, TRANSACTION)? {
                if matches(&header, &event) {
                    return Ok((header, event));
                }
                if self.pending.len() == MAX_DEFERRED {
                    return Err(Error(libdunit::ENOBUFS));
                }
                self.pending.push_back((header, event));
            }
            if start.elapsed()? >= TRANSACTION {
                return Err(Error(libdunit::EAGAIN));
            }
        }
    }
    fn can_recover(&self) -> bool {
        self.protocol_error == Some((self.surface, self.serial, ErrorCode::BadSerial.code(), 0))
            && self
                .pending
                .iter()
                .any(|(h, e)| h.object == self.surface && matches!(e, Event::Configure { .. }))
    }
    fn take_latest_configure(&mut self) -> Option<(Header, Event)> {
        let surface = self.surface;
        let mut latest: Option<(Header, Event)> = None;
        self.pending.retain(|(h, e)| {
            if h.object != surface || !matches!(e, Event::Configure { .. }) {
                return true;
            }
            if latest
                .as_ref()
                .map_or(true, |(old, _)| h.serial > old.serial)
            {
                latest = Some((*h, e.clone()));
            }
            false
        });
        latest
    }
    fn recover(&mut self) -> Result<()> {
        let start = Instant::now()?;
        let mut error = Error(libdunit::EAGAIN);
        for _ in 0..MAX_RETRIES {
            if !self.can_recover() {
                return Err(error);
            }
            match self.recreate_surface() {
                Ok(()) => {
                    self.resized = true;
                    return Ok(());
                }
                Err(e) => error = e,
            }
            if start.elapsed()? >= TRANSACTION {
                break;
            }
        }
        Err(error)
    }
    fn recreate_surface(&mut self) -> Result<()> {
        let (_, mut geometry) = self.take_latest_configure().ok_or(Error(libdunit::EIO))?;
        // v1 cannot cancel an attached-but-uncommitted buffer. DESTROY_SURFACE
        // is its cleanup barrier: all pending/committed buffers become available
        // before RESULT. Recreate with fresh IDs on the same connection/PID;
        // the compositor retains placement, state and app identity for that PID.
        self.request(self.surface, Request::DestroySurface)?;
        if let Some((_, newer)) = self.take_latest_configure() {
            geometry = newer;
        }
        self.pending.retain(|(h, _)| h.object != self.surface);
        while let Some(object) = self.live_buffers.last().copied() {
            self.request(object, Request::DestroyBuffer)?;
            self.live_buffers.pop();
        }
        self.current = None;
        self.released.clear();
        let Event::Configure {
            width,
            height,
            scale,
            ..
        } = geometry
        else {
            return Err(Error(libdunit::EIO));
        };
        buffer_bytes(width, height)?;
        if scale != 1 {
            return Err(Error(libdunit::EOPNOTSUPP));
        }
        self.surface = self.next_object;
        self.next_object = self
            .next_object
            .checked_add(1)
            .ok_or(Error(libdunit::EINVAL))?;
        self.request(
            self.surface,
            Request::CreateSurface {
                role: ROLE_TOPLEVEL,
                width,
                height,
                format: FORMAT_XRGB8888,
            },
        )?;
        let surface = self.surface;
        let (header, event) =
            self.until(|h, e| h.object == surface && matches!(e, Event::Configure { .. }))?;
        self.configure(header, event)?;
        Ok(())
    }
    fn configure(&mut self, header: Header, event: Event) -> Result<()> {
        let Event::Configure {
            width,
            height,
            scale,
            ..
        } = event
        else {
            return Err(Error(libdunit::EIO));
        };
        buffer_bytes(width, height)?;
        if scale != 1 {
            return Err(Error(libdunit::EOPNOTSUPP));
        }
        self.request(
            self.surface,
            Request::AckConfigure {
                configure: header.serial,
            },
        )?;
        self.width = width;
        self.height = height;
        self.configure = header.serial;
        Ok(())
    }
    /// Paint an unpublished buffer, then wait for the compositor frame callback.
    /// The callback may run again at new geometry after a concurrent configure;
    /// keep application state updates outside it. Recovery is bounded.
    /// An unrecoverable failure poisons the session; close/drop releases resources.
    pub fn draw(&mut self, mut paint: impl FnMut(&mut Frame<'_>)) -> Result<()> {
        self.try_draw(|frame| { paint(frame); Ok(()) })
    }
    /// Fallible painting, with the same retry semantics as `draw`. An error prevents transfer/import/publication of the
    /// private buffer and poisons the session like other drawing failures.
    pub fn try_draw(&mut self, mut paint: impl FnMut(&mut Frame<'_>) -> Result<()>) -> Result<()> {
        let start = Instant::now()?;
        let mut result = Err(Error(libdunit::EAGAIN));
        for _ in 0..MAX_RETRIES {
            result = self.draw_inner(&mut paint);
            if result.is_ok() {
                return result;
            }
            if start.elapsed()? >= TRANSACTION || !self.can_recover() {
                break;
            }
            result = self.recover();
            if result.is_err() {
                break;
            }
        }
        self.failed = true;
        result.and(Err(Error(libdunit::EAGAIN)))
    }
    fn draw_inner(&mut self, paint: &mut impl FnMut(&mut Frame<'_>) -> Result<()>) -> Result<()> {
        if self.failed {
            return Err(Error(libdunit::EPIPE));
        }
        let object = self.next_object;
        self.next_object = object
            .checked_add(1)
            .filter(|n| *n <= u32::MAX as u64)
            .ok_or(Error(libdunit::EINVAL))?;
        let buffer = Buffer::new(self.width, self.height, object)?;
        // Owned, aligned kernel mapping; no aliases have been transferred yet.
        let pixels = unsafe {
            core::slice::from_raw_parts_mut(buffer.address as *mut u32, buffer.bytes / 4)
        };
        paint(&mut Frame {
            pixels,
            width: self.width,
            height: self.height,
        })?;
        let duplicate = checked(libdunit::handle_dup(
            buffer.handle,
            libdunit::RIGHT_READ | libdunit::RIGHT_MAP | libdunit::RIGHT_TRANSFER,
        ))? as u32;
        let transferred = libdunit::handle_transfer(duplicate, self.peer);
        // Transfer moves the duplicate on success; close only on failure.
        if transferred < 0 {
            libdunit::handle_close(duplicate);
            return Err(Error(transferred));
        }
        let announcement = Capability {
            handle: transferred as u32,
            advertised_bytes: buffer.bytes as u32,
            object: object as u32,
        }
        .encode();
        checked(libdunit::ipc_send(self.peer, &announcement))?;
        self.request(
            object,
            Request::ImportBuffer {
                width: self.width,
                height: self.height,
                stride: self.width * 4,
                format: FORMAT_XRGB8888,
                offset: 0,
            },
        )?;
        self.live_buffers.push(object);
        self.request(
            self.surface,
            Request::AttachBuffer {
                buffer: object,
                damage: Vec::new(),
            },
        )?;
        let commit = self.send(
            self.surface,
            Request::Commit {
                configure: self.configure,
                frame_callback: 1,
            },
        )?;
        let surface = self.surface;
        self.until(|h, e| {
            h.object == surface && matches!(e, Event::Result { request } if *request == commit)
        })?;
        let (_, frame) = self.until(|h, e| {
            h.object == surface && matches!(e, Event::FrameDone { commit: c, .. } if *c == commit)
        })?;
        if !matches!(frame, Event::FrameDone { status: 0, .. }) {
            return Err(Error(libdunit::EIO));
        }
        if let Some(old) = self.current.take() {
            if !self.released.contains(&old.object) {
                return Err(Error(libdunit::EIO));
            }
            self.request(old.object, Request::DestroyBuffer)?;
            self.live_buffers.retain(|id| *id != old.object);
        }
        self.released.clear();
        self.current = Some(buffer);
        Ok(())
    }
    /// WaitSet-backed wait. `None` means the timeout elapsed. Resize is acked
    /// before delivery; redraw at `size()` to publish the new geometry.
    pub fn next_event(&mut self, timeout: Duration) -> Result<Option<WindowEvent>> {
        if self.failed {
            return Err(Error(libdunit::EPIPE));
        }
        let mut result = self.next_inner(timeout);
        if result.is_err() && self.can_recover() {
            result = self.recover().map(|()| {
                self.resized = false;
                Some(WindowEvent::Resized {
                    width: self.width,
                    height: self.height,
                })
            });
        }
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn next_inner(&mut self, timeout: Duration) -> Result<Option<WindowEvent>> {
        if core::mem::take(&mut self.resized) {
            return Ok(Some(WindowEvent::Resized {
                width: self.width,
                height: self.height,
            }));
        }
        if let Some(input) = self.inputs.pop_front() {
            return Ok(Some(if input.kind == IN_QUIT {
                WindowEvent::CloseRequested
            } else {
                WindowEvent::Input(input)
            }));
        }
        let packet = match self.pending.pop_front() {
            Some(packet) => Some(packet),
            None => self.packet(Instant::now()?, timeout)?,
        };
        let Some((header, event)) = packet else {
            return Ok(self.inputs.pop_front().map(|input| {
                if input.kind == IN_QUIT {
                    WindowEvent::CloseRequested
                } else {
                    WindowEvent::Input(input)
                }
            }));
        };
        Ok(Some(match event {
            Event::Configure { .. } if header.object == self.surface => {
                self.configure(header, event)?;
                WindowEvent::Resized {
                    width: self.width,
                    height: self.height,
                }
            }
            Event::RequestClose if header.object == self.surface => WindowEvent::CloseRequested,
            event => WindowEvent::Protocol(event),
        }))
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        // Best effort, nonblocking: process exit also causes compositor cleanup.
        // A failed transaction still needs a disconnect, independent of poison.
        if self.peer != 0 {
            let packet = Request::Disconnect.encode(0, self.serial.saturating_add(1));
            libdunit::ipc_send(self.peer, &packet);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_geometry_fails_before_consuming_the_session() {
        for (width, height) in [(0, 1), (1, 0), (DIM_MAX + 1, 1), (1, DIM_MAX + 1)] {
            assert!(matches!(
                Window::open(width, height),
                Err(Error(libdunit::EINVAL))
            ));
        }
        assert!(!STARTED.load(Ordering::Acquire));
    }
}
