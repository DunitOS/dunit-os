//! One top-level window per process, over the existing compositor bootstrap.
//! This session owns the process IPC inbox. Do not receive IPC independently.
//! Published pixels stay immutable until RELEASE and completed object teardown.
//! A bounded two-buffer pool repairs retired storage before publishing it again.
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
    Input, IN_DOWN, IN_FOCUS_IN, IN_FOCUS_OUT, IN_KEY, IN_KEY_UP, IN_LEAVE, IN_MOVE, IN_QUIT,
    IN_SCROLL, IN_UP, IN_RELATIVE, IN_POINTER_MODE, PointerMode,
};
pub use gui_protocol_v1::wire::{Event as ProtocolEvent, Rect as DamageRect};
use gui_protocol_v1::{
    control::{Bootstrap, Capability},
    wire::*,
};

pub use libdunit::{KEYMOD_CTRL,KEYMOD_SHIFT,KEYMOD_ALT,KEYMOD_SUPER};

const TRANSACTION: Duration = Duration::from_secs(5);
const MAX_RETRIES: usize = 8;
const MAX_DEFERRED: usize = 128;
static STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub enum WindowEvent {
    Resized { width: u32, height: u32 },
    Input(Input),
    CloseRequested,
    Protocol(Event),
    Transfer(dunit_transfer::Event),
}

/// Existing gui-v1 surface formats. Alpha uses straight ARGB pixels.
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum PixelFormat { Xrgb8888,Argb8888 }
impl PixelFormat {
    fn raw(self)->u32{match self{Self::Xrgb8888=>FORMAT_XRGB8888,Self::Argb8888=>FORMAT_ARGB8888}}
    fn bit(self)->u32{match self{Self::Xrgb8888=>FORMATS_XRGB8888,Self::Argb8888=>FORMATS_ARGB8888}}
}

/// A private, writable frame before publication in the window's pixel format.
/// The borrow cannot outlive the drawing callback or alias a committed buffer.
pub struct Frame<'a> {
    pixels: &'a mut [u32],
    width: u32,
    height: u32,
    full_damage: bool,
    repair: Option<DamageRect>,
    modified: bool,
    deferred: bool,
}
impl Frame<'_> {
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    /// SDK canvas snapshots are geometry-bound. A concurrent configure can
    /// require the application to supply a replacement before any publication.
    pub(crate) fn defer(&mut self) {self.deferred=true;}
    /// A new surface or geometry has no reusable compositor baseline.
    pub fn requires_full_frame(&self) -> bool {
        self.full_damage
    }
    /// Copy only the repair region into a released, unpublished buffer. A
    /// fresh backing or lost baseline requires the whole image. The previous
    /// frame's damage repairs a two-buffer pool's one-frame-old contents.
    pub fn copy_pixels(&mut self, source: &[u32], dirty: Option<DamageRect>) -> Result<u64> {
        if source.len() != self.pixels.len() {
            return Err(Error(libdunit::EINVAL));
        }
        let full = DamageRect {
            x: 0,
            y: 0,
            w: self.width,
            h: self.height,
        };
        if dirty.is_none() && !self.full_damage {
            return Ok(0);
        }
        let dirty = dirty.unwrap_or(full);
        if !valid_damage(dirty, self.width, self.height) {
            return Err(Error(libdunit::EINVAL));
        }
        let region = if self.full_damage || self.repair.is_none() {
            full
        } else {
            let old = self.repair.unwrap();
            if !valid_damage(old, self.width, self.height) {
                return Err(Error(libdunit::EINVAL));
            }
            let x = old.x.min(dirty.x);
            let y = old.y.min(dirty.y);
            DamageRect {
                x,
                y,
                w: ((old.x as u64 + old.w as u64).max(dirty.x as u64 + dirty.w as u64) - x as u64)
                    as u32,
                h: ((old.y as u64 + old.h as u64).max(dirty.y as u64 + dirty.h as u64) - y as u64)
                    as u32,
            }
        };
        self.modified = true;
        for y in region.y as usize..region.y as usize + region.h as usize {
            let row = y * self.width as usize + region.x as usize;
            self.pixels[row..row + region.w as usize]
                .copy_from_slice(&source[row..row + region.w as usize]);
        }
        Ok(region.w as u64 * region.h as u64)
    }
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        self.modified = true;
        self.pixels
    }
}
struct Buffer {
    handle: u32,
    address: usize,
    bytes: usize,
    object: u64,
    size: (u32, u32),
    valid: bool,
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
            size: (width, height),
            valid: false,
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
fn valid_damage(r: DamageRect, w: u32, h: u32) -> bool {
    r.x >= 0
        && r.y >= 0
        && r.w > 0
        && r.h > 0
        && r.x as u64 + r.w as u64 <= w as u64
        && r.y as u64 + r.h as u64 <= h as u64
}
/// A single IPC session. `open` is supported once per process, including after
/// close/failure: existing gui-v1 bootstrap has no reconnect/session identifier.
/// This type is deliberately neither Send nor Sync (one inbox and serial stream).
pub struct Window {
    metrics_enabled: bool,
    metrics: crate::ui::metrics::WindowMetrics,
    features: u64,
    pointer_mode: PointerMode,
    pointer_request_serial:u32,
    format: PixelFormat,
    surface: u64,
    live_buffers: Vec<u64>,
    protocol_error: Option<(u64, u64, u32, u32)>,
    resized: bool,
    peer: u32,
    launch_id:u32,
    serial: u64,
    server_serial: u64,
    configure: u64,
    width: u32,
    height: u32,
    next_object: u64,
    current: Option<Buffer>,
    reusable: Option<Buffer>,
    last_damage: Option<DamageRect>,
    pending: VecDeque<(Header, Event)>,
    inputs: VecDeque<Input>,
    transfer: Option<dunit_transfer::Client>,
    transfers: VecDeque<dunit_transfer::Event>,
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
        Self::open_format(width,height,PixelFormat::Xrgb8888)
    }
    pub fn open_format(width:u32,height:u32,format:PixelFormat)->Result<Self>{
        let features=FEATURE_INPUT_LIFECYCLE|FEATURE_DATA_TRANSFER|FEATURE_POINTER_MODE|if format==PixelFormat::Argb8888{FEATURE_ARGB8888}else{0};
        Self::open_features(width,height,features,format)
    }
    /// Whether the compositor negotiated pointer-mode support.
    pub fn supports_pointer_mode(&self)->bool{self.features&FEATURE_POINTER_MODE!=0}
    /// Effective mode, updated only by authenticated compositor acknowledgement.
    pub fn pointer_mode(&self)->PointerMode{self.pointer_mode}
    /// Queue an asynchronous request. IN_POINTER_MODE carries request id in x,
    /// negative errno in y and the effective mode in value. Zero id is revocation.
    pub fn request_pointer_mode(&mut self,mode:PointerMode)->Result<u32>{
        if !self.supports_pointer_mode(){return Err(Error(libdunit::EOPNOTSUPP));}
        let id=self.pointer_request_serial.checked_add(1).filter(|n|*n<=i32::MAX as u32).ok_or(Error(libdunit::EFBIG))?;
        let message=gui_protocol_v1::control::PointerRequest{request:id,mode};
        checked(libdunit::ipc_send(self.peer,&message.encode()))?;
        self.pointer_request_serial=id;Ok(id)
    }
    /// Compatibility clients can explicitly retain legacy cooked-only input.
    pub fn open_legacy(width: u32, height: u32) -> Result<Self> {
        Self::open_features(width, height, 0,PixelFormat::Xrgb8888)
    }
    /// Compositor-assigned launch/tint identity, not a security credential.
    pub fn launch_id(&self)->u32{self.launch_id}
    /// Enable or disable aggregate diagnostics without resetting prior samples.
    /// Disabled by default; the disabled path makes no diagnostic clock calls.
    pub fn enable_metrics(&mut self, enabled: bool) {
        self.metrics_enabled = enabled;
        if enabled {
            self.metrics.input_queue_high_water =
                self.metrics.input_queue_high_water.max(self.inputs.len());
            self.metrics.protocol_queue_high_water = self
                .metrics
                .protocol_queue_high_water
                .max(self.pending.len());
        }
    }
    pub fn reset_metrics(&mut self) {
        self.metrics = Default::default();
        if self.metrics_enabled {
            self.metrics.input_queue_high_water = self.inputs.len();
            self.metrics.protocol_queue_high_water = self.pending.len();
        }
    }
    pub fn metrics(&self) -> crate::ui::WindowMetrics {
        let mut metrics = self.metrics;
        metrics.input_queued = self.inputs.len();
        metrics.protocol_queued = self.pending.len();
        metrics
    }
    pub fn input_lifecycle(&self) -> bool {
        self.features & FEATURE_INPUT_LIFECYCLE != 0
    }
    fn open_features(width: u32, height: u32, offered: u64,format:PixelFormat) -> Result<Self> {
        buffer_bytes(width, height)?;
        if STARTED.swap(true, Ordering::AcqRel) {
            return Err(Error(libdunit::EBUSY));
        }
        let mut window = Self {
            metrics_enabled: false,
            metrics: Default::default(),
            features: 0,
            pointer_mode: PointerMode::Normal,
            pointer_request_serial:0,
            format,
            surface: 1,
            live_buffers: Vec::new(),
            protocol_error: None,
            resized: false,
            peer: 0,
            launch_id:0,
            serial: 0,
            server_serial: 0,
            configure: 0,
            width,
            height,
            next_object: 2,
            current: None,
            reusable: None,
            last_damage: None,
            pending: VecDeque::new(),
            inputs: VecDeque::new(),
            transfer: None,
            transfers: VecDeque::new(),
            released: Vec::new(),
            waiter: EventLoop::new(&[Interest::ipc(1)])?,
            failed: false,
            _single_thread: PhantomData,
        };
        let mut bytes = [0; MAX_PACKET];
        let start = Instant::now()?;
        loop {
            let (n, sender, attachment) = window
                .receive(&mut bytes, start, TRANSACTION)?
                .ok_or(Error(libdunit::EAGAIN))?;
            if attachment.is_some() { continue; }
            if Bootstrap::decode(&bytes[..n])
                .is_some_and(|intro| intro.compositor == sender && sender != 0)
            {
                window.peer = sender;
                window.launch_id=Bootstrap::decode(&bytes[..n]).unwrap().client;
                window.transfer = Some(dunit_transfer::Client::new(sender, 0));
                break;
            }
        }
        let hello = window.send(
            0,
            Request::Hello {
                min_minor: 0,
                max_minor: 0,
                offered_features: offered,
                required_features: 0,
            },
        )?;
        let (_, welcome) = window
            .until(|_, e| matches!(e, Event::Welcome { request, .. } if *request == hello))?;
        if !matches!(welcome, Event::Welcome { formats, .. } if formats & format.bit() != 0) {
            return Err(Error(libdunit::EOPNOTSUPP));
        }
        if let Event::Welcome { features, .. } = welcome {
            if features & !offered != 0 {
                return Err(Error(libdunit::EIO));
            }
            window.features = features;
            if let Some(transfer) = window.transfer.as_mut() { transfer.set_features(features); }
        }
        window.request(
            window.surface,
            Request::CreateSurface {
                role: ROLE_TOPLEVEL,
                width,
                height,
                format: window.format.raw(),
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
    ) -> Result<Option<(usize, u32, Option<dunit_transfer::OwnedHandle>)>> {
        loop {
            // Check before consuming too: a continuous stream of malformed
            // bootstrap packets must not keep a transaction alive forever.
            if !timeout.is_zero() && start.elapsed()? >= timeout {
                return Ok(None);
            }
            if let Some(transfer) = self.transfer.as_mut() {
                transfer.flush().map_err(|e|Error(e.0))?;
            }
            let mut sender = 0;
            let mut attached = 0;
            let n = libdunit::ipc_recv_handle(bytes, &mut sender, &mut attached);
            // Only the syscall's attested output can create an owner. Foreign
            // traffic drops that owner here, including malformed CAP1/XFR1.
            let attachment = if n >= 0 && attached != 0 {
                Some(unsafe { dunit_transfer::OwnedHandle::adopt(attached) })
            } else { None };
            if n >= 0 && (self.peer == 0 || sender == self.peer) {
                return Ok(Some((n as usize, sender, attachment)));
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
                let remaining = timeout - elapsed;
                let remaining = if self.transfer.as_ref().is_some_and(|t|t.pending_send()) {
                    remaining.min(Duration::from_millis(1))
                } else { remaining };
                self.waiter.wait(crate::time::timeout_ns(remaining)?)?;
            }
        }
    }
    fn packet(&mut self, start: Instant, timeout: Duration) -> Result<Option<(Header, Event)>> {
        let mut bytes = [0; MAX_PACKET];
        loop {
            let Some((n, _, attachment)) = self.receive(&mut bytes, start, timeout)? else {
                return Ok(None);
            };
            if dunit_transfer::Message::is_message(&bytes[..n]) {
                if self.features & FEATURE_DATA_TRANSFER == 0 { return Err(Error(libdunit::EOPNOTSUPP)); }
                let transfer = self.transfer.as_mut().ok_or(Error(libdunit::EIO))?;
                if let Some(event) = transfer.handle_packet(&bytes[..n], attachment).map_err(|e|Error(e.0))? {
                    if self.transfers.len() == MAX_DEFERRED { return Err(Error(libdunit::ENOBUFS)); }
                    self.transfers.push_back(event);
                }
                transfer.flush().map_err(|e|Error(e.0))?;
                return Ok(None);
            }
            if attachment.is_some() { return Err(Error(libdunit::EIO)); }
            if let Some(input) = Input::decode_for_features(&bytes[..n], self.features) {
                if input.kind==IN_POINTER_MODE {
                    self.pointer_mode=PointerMode::from_u32(input.value).ok_or(Error(libdunit::EIO))?;
                }
                if self.metrics_enabled {
                    self.metrics.input_received = self.metrics.input_received.saturating_add(1);
                }
                if self.inputs.len() == MAX_DEFERRED {
                    return Err(Error(libdunit::ENOBUFS));
                }
                self.inputs.push_back(input);
                if self.metrics_enabled {
                    self.metrics.input_queue_high_water =
                        self.metrics.input_queue_high_water.max(self.inputs.len());
                }
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
                if self.metrics_enabled {
                    self.metrics.protocol_queue_high_water = self
                        .metrics
                        .protocol_queue_high_water
                        .max(self.pending.len());
                }
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
        self.reusable = None;
        self.last_damage = None;
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
                format: self.format.raw(),
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
        self.try_draw(|frame| {
            paint(frame);
            Ok(())
        })
    }
    /// Fallible painting, with the same retry semantics as `draw`. An error prevents transfer/import/publication of the
    /// private buffer and poisons the session like other drawing failures.
    pub fn try_draw(&mut self, mut paint: impl FnMut(&mut Frame<'_>) -> Result<()>) -> Result<()> {
        self.try_draw_damage(|frame| {
            paint(frame)?;
            let (w, h) = frame.size();
            Ok(Some(DamageRect { x: 0, y: 0, w, h }))
        })
    }
    /// Publish a complete valid image with a bounded damage hint. `None` skips
    /// an unchanged frame unless a new surface requires a full baseline.
    /// Published storage stays immutable until retired; drawing may retry after
    /// CONFIGURE. Use `Frame::copy_pixels` to repair a reused buffer safely.
    pub fn try_draw_damage(
        &mut self,
        mut paint: impl FnMut(&mut Frame<'_>) -> Result<Option<DamageRect>>,
    ) -> Result<()> {
        let started = crate::ui::metrics::start(self.metrics_enabled);
        let result = self.try_draw_damage_inner(&mut paint);
        self.metrics
            .draw_total
            .finish(self.metrics_enabled, started);
        result
    }
    fn try_draw_damage_inner(
        &mut self,
        paint: &mut impl FnMut(&mut Frame<'_>) -> Result<Option<DamageRect>>,
    ) -> Result<()> {
        let start = Instant::now()?;
        let mut result = Err(Error(libdunit::EAGAIN));
        for _ in 0..MAX_RETRIES {
            result = self.draw_inner(paint);
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
    fn draw_inner(
        &mut self,
        paint: &mut impl FnMut(&mut Frame<'_>) -> Result<Option<DamageRect>>,
    ) -> Result<()> {
        if self.failed {
            return Err(Error(libdunit::EPIPE));
        }
        let object = self.next_object;
        self.next_object = object
            .checked_add(1)
            .filter(|n| *n <= u32::MAX as u64)
            .ok_or(Error(libdunit::EINVAL))?;
        let mut buffer = match self
            .reusable
            .take()
            .filter(|b| b.size == (self.width, self.height))
        {
            Some(mut b) => {
                b.object = object;
                b
            }
            None => Buffer::new(self.width, self.height, object)?,
        };
        let repair = if buffer.valid { self.last_damage } else { None };
        // Owned, aligned mapping; fresh or retired after the last-reader barrier.
        let pixels = unsafe {
            core::slice::from_raw_parts_mut(buffer.address as *mut u32, buffer.bytes / 4)
        };
        let full_damage = self
            .current
            .as_ref()
            .map_or(true, |b| b.size != (self.width, self.height));
        let mut frame = Frame {
            pixels,
            width: self.width,
            height: self.height,
            full_damage,
            repair,
            modified: false,
            deferred: false,
        };
        let damage = paint(&mut frame)?;
        let modified = frame.modified;
        if frame.deferred {
            buffer.valid=false;
            self.reusable=Some(buffer);
            self.resized=true;
            return Ok(());
        }
        if damage.is_none() && !full_damage {
            buffer.valid &= !modified;
            self.reusable = Some(buffer);
            return Ok(());
        }
        let damage = if full_damage {
            DamageRect {
                x: 0,
                y: 0,
                w: self.width,
                h: self.height,
            }
        } else {
            damage.unwrap()
        };
        if !valid_damage(damage, self.width, self.height) {
            return Err(Error(libdunit::EINVAL));
        }
        let import_started = crate::ui::metrics::start(self.metrics_enabled);
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
        // These requests have a defined FIFO dependency: IMPORT consumes the
        // preceding CAP1, ATTACH names that imported object, then COMMIT publishes
        // it. Queue them together instead of adding a server round trip between
        // each request. Every RESULT and the presentation barrier are still read.
        let import = self.send(
            object,
            Request::ImportBuffer {
                width: self.width,
                height: self.height,
                stride: self.width * 4,
                format: self.format.raw(),
                offset: 0,
            },
        )?;
        let attach = self.send(
            self.surface,
            Request::AttachBuffer {
                buffer: object,
                damage: alloc::vec![damage],
            },
        )?;
        let commit = self.send(
            self.surface,
            Request::Commit {
                configure: self.configure,
                frame_callback: 1,
            },
        )?;
        if self.metrics_enabled {
            self.metrics.commits = self.metrics.commits.saturating_add(1);
        }
        let surface = self.surface;
        self.until(|h, e| {
            h.object == object && matches!(e, Event::Result { request } if *request == import)
        })?;
        // Keep the imported object in recovery bookkeeping before consuming a
        // possible stale-configure error from the later COMMIT. CONFIGURE stays
        // queued by until(), and COMMIT is still the latest client serial.
        self.live_buffers.push(object);
        self.until(|h, e| {
            h.object == surface && matches!(e, Event::Result { request } if *request == attach)
        })?;
        self.metrics
            .import_attach
            .finish(self.metrics_enabled, import_started);
        let commit_started = crate::ui::metrics::start(self.metrics_enabled);
        self.until(|h, e| {
            h.object == surface && matches!(e, Event::Result { request } if *request == commit)
        })?;
        self.metrics
            .commit_result
            .finish(self.metrics_enabled, commit_started);
        let frame_started = crate::ui::metrics::start(self.metrics_enabled);
        let (_, frame) = self.until(|h, e| {
            h.object == surface && matches!(e, Event::FrameDone { commit: c, .. } if *c == commit)
        })?;
        self.metrics
            .frame_done
            .finish(self.metrics_enabled, frame_started);
        if !matches!(frame, Event::FrameDone { status: 0, .. }) {
            return Err(Error(libdunit::EIO));
        }
        if self.metrics_enabled {
            self.metrics.completed_frames = self.metrics.completed_frames.saturating_add(1);
        }
        if let Some(old) = self.current.take() {
            if !self.released.contains(&old.object) {
                return Err(Error(libdunit::EIO));
            }
            let retire_started = crate::ui::metrics::start(self.metrics_enabled);
            self.request(old.object, Request::DestroyBuffer)?;
            self.metrics
                .retire
                .finish(self.metrics_enabled, retire_started);
            self.live_buffers.retain(|id| *id != old.object);
            // RELEASE + DESTROY_RESULT is the last-reader barrier. This mapping
            // becomes writable only on the next unpublished drawing callback.
            self.reusable = Some(old);
        }
        self.released.clear();
        buffer.valid = true;
        self.last_damage = Some(damage);
        self.current = Some(buffer);
        Ok(())
    }
    /// WaitSet-backed wait. `None` means the timeout elapsed. Resize is acked
    /// before delivery; redraw at `size()` to publish the new geometry.
    pub fn next_event(&mut self, timeout: Duration) -> Result<Option<WindowEvent>> {
        if self.failed {
            return Err(Error(libdunit::EPIPE));
        }
        if let Some(transfer)=self.transfer.as_mut(){transfer.flush().map_err(|error|Error(error.0))?;}
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
    fn pop_input(&mut self) -> Option<WindowEvent> {
        let input = self.inputs.pop_front()?;
        if self.metrics_enabled {
            self.metrics.input_delivered = self.metrics.input_delivered.saturating_add(1);
        }
        Some(if input.kind == IN_QUIT {
            WindowEvent::CloseRequested
        } else {
            WindowEvent::Input(input)
        })
    }
    /// Request a reload of saved desktop configuration through this window's
    /// authenticated compositor connection. It does not receive from the inbox.
    pub fn reload_desktop(&mut self)->Result<()>{
        let result=libdunit::ipc_send(self.peer,&gui_protocol_v1::control::RELOAD);
        if result<0{Err(Error(result))}else{Ok(())}
    }
    /// Post a bounded notification through the authenticated compositor connection.
    pub fn notify(&mut self, text: &str) -> Result<()> {
        let mut bytes = [0; gui_protocol_v1::control::NOTIFY_LEN];
        let n = gui_protocol_v1::control::notification(text, &mut bytes);
        let result = libdunit::ipc_send(self.peer, &bytes[..n]);
        if result < 0 {
            Err(Error(result))
        } else {
            Ok(())
        }
    }
    /// Queue transfer requests here; the existing receive/event pump flushes
    /// them and delivers replies as WindowEvent::Transfer, also during draw.
    pub fn transfers(&mut self) -> Result<&mut dunit_transfer::Client> {
        if self.features & FEATURE_DATA_TRANSFER == 0 { return Err(Error(libdunit::EOPNOTSUPP)); }
        self.transfer.as_mut().ok_or(Error(libdunit::EIO))
    }
    fn next_inner(&mut self, timeout: Duration) -> Result<Option<WindowEvent>> {
        if core::mem::take(&mut self.resized) {
            return Ok(Some(WindowEvent::Resized {
                width: self.width,
                height: self.height,
            }));
        }
        if let Some(event) = self.pop_input() {
            return Ok(Some(event));
        }
        if let Some(event) = self.transfers.pop_front() { return Ok(Some(WindowEvent::Transfer(event))); }
        let packet = match self.pending.pop_front() {
            Some(packet) => Some(packet),
            None => self.packet(Instant::now()?, timeout)?,
        };
        let Some((header, event)) = packet else {
            return Ok(self.pop_input().or_else(||self.transfers.pop_front().map(WindowEvent::Transfer)));
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
    fn released_buffer_repairs_previous_and_current_damage() {
        let mut pixels = alloc::vec![0;16];
        let mut source = pixels.clone();
        source[..2].fill(1);
        source[8..10].fill(2);
        let mut frame = Frame {
            pixels: &mut pixels,
            width: 4,
            height: 4,
            full_damage: false,
            repair: Some(DamageRect {
                x: 0,
                y: 0,
                w: 2,
                h: 1,
            }),
            modified: false,
            deferred: false,
        };
        assert_eq!(
            frame
                .copy_pixels(
                    &source,
                    Some(DamageRect {
                        x: 0,
                        y: 2,
                        w: 2,
                        h: 1
                    })
                )
                .unwrap(),
            6
        );
        assert_eq!(pixels, source);
    }
    #[test]
    fn fresh_or_resized_buffers_copy_full_and_empty_updates_copy_nothing() {
        let source = alloc::vec![7;16];
        let mut pixels = alloc::vec![0;16];
        let mut frame = Frame {
            pixels: &mut pixels,
            width: 4,
            height: 4,
            full_damage: false,
            repair: None,
            modified: false,
            deferred: false,
        };
        assert_eq!(frame.copy_pixels(&source, None).unwrap(), 0);
        assert!(!frame.modified);
        assert_eq!(
            frame
                .copy_pixels(
                    &source,
                    Some(DamageRect {
                        x: 1,
                        y: 1,
                        w: 1,
                        h: 1
                    })
                )
                .unwrap(),
            16
        );
        frame.full_damage = true;
        frame.repair = Some(DamageRect {
            x: 1,
            y: 1,
            w: 1,
            h: 1,
        });
        assert_eq!(frame.copy_pixels(&source, None).unwrap(), 16);
        assert!(frame.copy_pixels(&source[..8], None).is_err());
        assert_eq!(pixels, source);
    }
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
