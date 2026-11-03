//! Native capability custody for the deterministic XFR1 broker. The compositor
//! supplies real focus/capture/hit tests; paths are display metadata only.
use alloc::{collections::BTreeMap, vec::Vec};
use dunit_transfer::{
    broker::{Body, Broker, Effect},
    transport::{Outbox, Packet},
    *,
};
struct Delivery {
    to: u32,
    message: Message,
    bundle: Bundle,
    handles: Vec<u32>,
}
pub(super) struct Runtime {
    last_clipboard: Option<u64>,
    trace: bool,
    launches: Vec<(u64, u64)>,
    broker: Broker,
    outbox: Outbox,
    peers: Vec<u32>,
    capture: Option<u32>,
    press: Option<(u32, i32, i32)>,
    position: (i32, i32),
    threshold: u32,
    escrow: BTreeMap<u64, BTreeMap<u32, OwnedHandle>>,
    destinations: BTreeMap<(u32, u64, u32), OwnedHandle>,
    deliveries: BTreeMap<u64, Delivery>,
}
impl Runtime {
    pub fn new(timeout_ms: u64) -> Self {
        Self {
            last_clipboard: None,
            trace: false,
            launches: Vec::new(),
            broker: Broker::new(timeout_ms),
            outbox: Outbox::default(),
            peers: Vec::new(),
            capture: None,
            press: None,
            position: (0, 0),
            threshold: 1,
            escrow: BTreeMap::new(),
            destinations: BTreeMap::new(),
            deliveries: BTreeMap::new(),
        }
    }
    pub fn configure(&mut self, policy: super::settings::Transfers) {
        self.broker.set_timeout(policy.timeout_ms as u64);
        self.threshold = policy.drag_threshold_px;
        self.trace = policy.trace;
    }
    pub fn begin_press(&mut self, pid: u32, x: i32, y: i32) {
        self.press = Some((pid, x, y));
    }
    pub fn position(&mut self, x: i32, y: i32) {
        self.position = (x, y);
    }
    pub fn end_press(&mut self) {
        self.press = None;
    }
    fn captured(&self, pid: u32) -> bool {
        self.capture == Some(pid)
            && self.press.is_some_and(|(p, x, y)| {
                let dx = i64::from(x) - i64::from(self.position.0);
                let dy = i64::from(y) - i64::from(self.position.1);
                p == pid && dx * dx + dy * dy >= i64::from(self.threshold).pow(2)
            })
    }
    pub fn input(&mut self, focus: Option<u32>, capture: Option<u32>) {
        self.broker.focus(focus);
        self.capture = capture;
    }
    pub fn dragging(&self) -> bool {
        self.broker.dragging()
    }
    pub fn action(&self, mods: u8) -> Option<Action> {
        let ctrl = mods & libdunit::KEYMOD_CTRL != 0;
        let shift = mods & libdunit::KEYMOD_SHIFT != 0;
        if ctrl && shift {
            Some(Action::Link)
        } else if ctrl {
            Some(Action::Copy)
        } else if shift {
            Some(Action::Move)
        } else {
            None
        }
    }
    pub fn active(&self) -> Option<(u64, u32, Option<u32>, Option<Action>)> {
        self.broker.active_drag()
    }
    pub fn peer(&mut self, pid: u32, enabled: bool) {
        if enabled {
            if !self.peers.contains(&pid) {
                self.peers.push(pid);
                self.broker.connect(pid);
            }
        } else if self.peers.contains(&pid) {
            self.disconnected(pid);
        }
    }
    pub fn disconnected(&mut self, pid: u32) {
        self.peers.retain(|peer| *peer != pid);
        let out = self.broker.disconnect(pid);
        self.effects(out);
        self.outbox.discard_peer(pid);
    }
    pub fn tracing(&self) -> bool {
        self.trace
    }
    pub fn tick(&mut self, now: u64) {
        let out = self.broker.tick(now);
        self.effects(out);
        self.flush();
    }
    pub fn pointer(&mut self, hit: Option<(u32, i32, i32)>, action: Option<Action>, now: u64) {
        let out = self.broker.pointer(hit, action, now);
        self.effects(out);
    }
    pub fn pointer_opener(
        &mut self,
        index: usize,
        formats: &[alloc::string::String],
        action: Option<Action>,
        now: u64,
    ) {
        let formats = formats
            .iter()
            .map(|name| Format::parse(name))
            .collect::<core::result::Result<Vec<_>, _>>();
        if let Ok(formats) = formats {
            let offer = Offer {
                formats,
                actions: 1,
                preferred: Action::Copy,
            };
            if offer.validate().is_ok() {
                let out = self
                    .broker
                    .pointer_opener(index as u64, &offer, action, now);
                self.effects(out);
                return;
            }
        }
        self.pointer(None, action, now);
    }
    pub fn opener(&self) -> Option<usize> {
        self.broker.active_opener().map(|index| index as usize)
    }
    pub fn take_launch(&mut self) -> Option<(usize, u64)> {
        if self.launches.is_empty() {
            None
        } else {
            let (index, id) = self.launches.remove(0);
            Some((index as usize, id))
        }
    }
    pub fn launch_result(&mut self, id: u64, pid: Option<u32>, now: u64) {
        if let Some(pid) = pid {
            self.broker.expect_opener(id, pid, now);
        } else {
            let out = self.broker.fail_opener(id, Status::TargetGone);
            self.effects(out);
        }
    }
    pub fn drop_pointer(&mut self, now: u64) {
        let out = self.broker.drop_pointer(now);
        self.effects(out);
    }
    pub fn cancel(&mut self) {
        let out = self.broker.cancel_drag(Status::Cancelled);
        self.effects(out);
    }
    pub fn flush(&mut self) {
        for (pid, _, _) in self.outbox.flush() {
            self.peer(pid, false);
        }
    }
    fn reply(&mut self, pid: u32, message: Message, status: Status) {
        let message = Message {
            kind: Kind::Result,
            arg0: status as u32,
            arg1: 0,
            ..message
        };
        if self.outbox.push(Packet::control(pid, message)).is_err() {
            self.peer(pid, false);
        }
    }
    /// Called before CAP1/DGUI dispatch, with the actual syscall attachment.
    /// Unknown senders never reach this function (their owner drops at the pump).
    pub fn packet(
        &mut self,
        pid: u32,
        bytes: &[u8],
        attachment: Option<OwnedHandle>,
        now: u64,
    ) -> bool {
        if !Message::is_message(bytes) {
            return false;
        }
        let Ok(mut message) = Message::decode(bytes) else {
            return true;
        };
        if self.trace {
            libdunit::println(&alloc::format!(
                "[XFR] rx pid={} kind={:?} transfer={} captured={}",
                pid,
                message.kind,
                message.transfer,
                self.captured(pid)
            ));
        }
        if !self.peers.contains(&pid) {
            self.reply(pid, message, Status::Denied);
            return true;
        }
        if matches!(message.kind, Kind::Grant | Kind::GrantReceived) {
            if let Err(status) = self.broker.authorize(pid, message) {
                self.reply(pid, message, status);
                return true;
            }
            let result = if message.kind == Kind::Grant {
                self.grant(pid, message, attachment)
            } else {
                if attachment.is_some() {
                    Err(Error(libdunit::EINVAL))
                } else {
                    self.received(pid, message)
                }
            };
            if let Err(error) = result {
                self.reply(pid, message, errno_status(error));
            }
            return true;
        }
        // Reject impossible producers/consumers before mapping and copying a
        // potentially large shared blob. The model still performs replay and
        // full state validation after the bounded private copy.
        let denied = match message.kind {
            Kind::ProvideData => !self.broker.can_grant_source(pid, message.transfer),
            Kind::FileMove => !self.broker.can_grant_destination(pid, message.transfer),
            Kind::ClipboardOffer
            | Kind::ClipboardRead
            | Kind::ClipboardQuery
            | Kind::ClipboardClear => !self.broker.focused(pid),
            _ => false,
        };
        if denied {
            self.reply(pid, message, Status::Denied);
            return true;
        }
        if message.kind == Kind::DragOffer && !self.captured(pid) {
            self.reply(pid, message, Status::Busy);
            return true;
        }
        let body = if matches!(
            message.kind,
            Kind::ClipboardOffer
                | Kind::DragOffer
                | Kind::ClipboardRead
                | Kind::ProvideData
                | Kind::FileMove
        ) || (matches!(message.kind, Kind::RegisterTarget | Kind::RegisterOpener)
            && attachment.is_some())
        {
            let limit = match message.kind {
                Kind::ProvideData => MAX_BLOB_BYTES,
                Kind::FileMove => BLOB_HEADER_LEN + 16 + 255,
                _ => BLOB_HEADER_LEN + 16 + MAX_FORMATS * (4 + MAX_MIME_BYTES),
            };
            let parsed = transport::read_blob_limited(message, attachment, limit).and_then(
                |(decoded, bytes)| {
                    message = decoded;
                    match message.kind {
                        Kind::ProvideData => Bundle::decode(&bytes)
                            .map(Body::Data)
                            .map_err(|_| Error(libdunit::EINVAL)),
                        Kind::FileMove => FileMove::decode(&bytes)
                            .map(Body::Move)
                            .map_err(|_| Error(libdunit::EINVAL)),
                        _ => Offer::decode(&bytes)
                            .map(Body::Offer)
                            .map_err(|_| Error(libdunit::EINVAL)),
                    }
                },
            );
            match parsed {
                Ok(body) => Some(body),
                Err(error) => {
                    self.reply(pid, message, errno_status(error));
                    return true;
                }
            }
        } else {
            if attachment.is_some() {
                self.reply(pid, message, Status::Invalid);
                return true;
            }
            None
        };
        if let Some(Body::Data(bundle)) = &body {
            if !self.broker.can_grant_source(pid, message.transfer)
                || !self.valid_files(message.transfer, bundle)
            {
                self.reply(pid, message, Status::Denied);
                return true;
            }
        }
        let out = self
            .broker
            .request(pid, message, body, now, self.captured(pid));
        self.effects(out);
        true
    }
    fn valid_files(&self, id: u64, bundle: &Bundle) -> bool {
        let grants = self.escrow.get(&id);
        match bundle.get(&Format::Files) {
            Some(bytes) => {
                let Ok(list) = FileList::decode(bytes) else {
                    return false;
                };
                let Some(grants) = grants else { return false };
                grants.len() == list.items.len()
                    && list.items.iter().enumerate().all(|(index, file)| {
                        grants.get(&(index as u32)).is_some_and(|h| {
                            h.raw() == file.capability && native_kind(h) == Ok(file.kind)
                        })
                    })
            }
            None => grants.is_none_or(|g| g.is_empty()),
        }
    }
    fn grant(&mut self, pid: u32, m: Message, attachment: Option<OwnedHandle>) -> NativeResult<()> {
        let handle = attachment.ok_or(Error(libdunit::EACCES))?;
        if m.arg0 != handle.raw() {
            return Err(Error(libdunit::EACCES));
        }
        let rights = handle.rights()?;
        let allowed = libdunit::RIGHT_READ | libdunit::RIGHT_WRITE | libdunit::RIGHT_TRANSFER;
        if rights & !allowed != 0
            || rights & (libdunit::RIGHT_READ | libdunit::RIGHT_TRANSFER)
                != (libdunit::RIGHT_READ | libdunit::RIGHT_TRANSFER)
        {
            return Err(Error(libdunit::EACCES));
        }
        let kind = native_kind(&handle)?;
        let raw = handle.raw();
        if m.arg1 == ALL_FORMATS {
            if !self.broker.can_grant_destination(pid, m.transfer)
                || kind != FileKind::Directory
                || rights & libdunit::RIGHT_WRITE == 0
            {
                return Err(Error(libdunit::EACCES));
            }
            if self
                .destinations
                .keys()
                .filter(|(p, id, _)| *p == pid && *id == m.transfer)
                .count()
                >= MAX_FILE_ITEMS
            {
                return Err(Error(libdunit::ENOBUFS));
            }
            self.destinations.insert((pid, m.transfer, raw), handle);
        } else {
            if m.arg1 as usize >= MAX_FILE_ITEMS || !self.broker.can_grant_source(pid, m.transfer) {
                return Err(Error(libdunit::EACCES));
            }
            let move_allowed = self
                .broker
                .source_actions(pid, m.transfer)
                .is_some_and(|a| a & Action::Move as u32 != 0);
            if (rights & libdunit::RIGHT_WRITE != 0) != move_allowed {
                return Err(Error(libdunit::EACCES));
            }
            let grants = self.escrow.entry(m.transfer).or_default();
            if grants.contains_key(&m.arg1) {
                return Err(Error(libdunit::EINVAL));
            }
            grants.insert(m.arg1, handle);
        }
        self.outbox.push(Packet::control(
            pid,
            Message {
                kind: Kind::GrantAck,
                arg0: raw,
                arg1: Status::Ok as u32,
                ..m
            },
        ))?;
        Ok(())
    }
    fn received(&mut self, pid: u32, m: Message) -> NativeResult<()> {
        let d = self
            .deliveries
            .get_mut(&m.transfer)
            .ok_or(Error(libdunit::EINVAL))?;
        if d.to != pid || m.arg0 == 0 || d.handles.contains(&m.arg0) {
            return Err(Error(libdunit::EACCES));
        }
        let slot = d
            .handles
            .get_mut(m.arg1 as usize)
            .ok_or(Error(libdunit::EINVAL))?;
        if *slot != 0 {
            return Err(Error(libdunit::EINVAL));
        }
        *slot = m.arg0;
        if !d.handles.contains(&0) {
            let mut d = self
                .deliveries
                .remove(&m.transfer)
                .ok_or(Error(libdunit::EINVAL))?;
            let rep = d
                .bundle
                .representations
                .iter_mut()
                .find(|r| r.format == Format::Files)
                .ok_or(Error(libdunit::EINVAL))?;
            let mut list = FileList::decode(&rep.data).map_err(|_| Error(libdunit::EINVAL))?;
            for (item, raw) in list.items.iter_mut().zip(&d.handles) {
                item.capability = *raw;
            }
            rep.data = list.encode().map_err(|_| Error(libdunit::EINVAL))?;
            self.outbox.push(Packet::blob(
                pid,
                d.message,
                &d.bundle.encode().map_err(|_| Error(libdunit::EINVAL))?,
            )?)?;
        }
        self.reply(pid, m, Status::Ok);
        Ok(())
    }
    fn deliver(
        &mut self,
        to: u32,
        message: Message,
        bundle: Bundle,
        custody: u64,
    ) -> NativeResult<()> {
        let Some(bytes) = bundle.get(&Format::Files) else {
            return self.outbox.push(Packet::blob(
                to,
                message,
                &bundle.encode().map_err(|_| Error(libdunit::EINVAL))?,
            )?);
        };
        let files = FileList::decode(bytes).map_err(|_| Error(libdunit::EINVAL))?;
        let grants = self.escrow.get(&custody).ok_or(Error(libdunit::EACCES))?;
        let mut packets = Vec::new();
        for (index, file) in files.items.iter().enumerate() {
            let handle = grants
                .get(&(index as u32))
                .filter(|h| h.raw() == file.capability)
                .ok_or(Error(libdunit::EACCES))?;
            // Retain local TRANSFER only to let the queued atomic send execute;
            // the recipient may forward READ authority, but never receives WRITE.
            let duplicate = handle.duplicate(libdunit::RIGHT_READ | libdunit::RIGHT_TRANSFER)?;
            packets.push(Packet::handle(
                to,
                Message {
                    kind: Kind::Grant,
                    arg0: 0,
                    arg1: index as u32,
                    ..message
                },
                duplicate,
                libdunit::RIGHT_READ | libdunit::RIGHT_TRANSFER,
            ));
        }
        self.outbox.extend(packets)?;
        self.deliveries.insert(
            message.transfer,
            Delivery {
                to,
                message,
                bundle,
                handles: alloc::vec![0;files.items.len()],
            },
        );
        Ok(())
    }
    fn effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Retire(id) => {
                    self.escrow.remove(&id);
                    self.launches.retain(|(_, transfer)| *transfer != id);
                }
                Effect::Launch { opener, transfer } => self.launches.push((opener, transfer)),
                Effect::Move {
                    to,
                    request,
                    transfer,
                    custody,
                    operation,
                } => {
                    let result = self
                        .escrow
                        .get(&custody)
                        .and_then(|g| g.get(&operation.item))
                        .zip(
                            self.destinations
                                .get(&(to, transfer, operation.destination)),
                        )
                        .map(|(source, dest)| {
                            libdunit::file_cap::move_to(source.raw(), dest.raw(), &operation.leaf)
                        })
                        .unwrap_or(libdunit::EACCES);
                    let status = if result >= 0 {
                        Status::Ok
                    } else {
                        errno_status(Error(result))
                    };
                    let effect = self
                        .broker
                        .moved(to, request, transfer, operation.item, status);
                    self.effects(alloc::vec![effect]);
                }
                Effect::Send {
                    to,
                    message,
                    body,
                    custody,
                } => {
                    if self.trace
                        && message.kind == Kind::ClipboardChanged
                        && self.last_clipboard != Some(message.transfer)
                    {
                        self.last_clipboard = Some(message.transfer);
                        libdunit::println(&alloc::format!(
                            "[XFR] clipboard transfer={}",
                            message.transfer
                        ));
                    }
                    if self.trace {
                        libdunit::println(&alloc::format!(
                            "[XFR] tx pid={} kind={:?} transfer={} arg0={}",
                            to,
                            message.kind,
                            message.transfer,
                            message.arg0
                        ));
                    }
                    if matches!(message.kind, Kind::Finished | Kind::Cancelled) {
                        self.outbox.discard_data(message.transfer);
                        self.deliveries.remove(&message.transfer);
                        self.destinations
                            .retain(|(_, id, _), _| *id != message.transfer);
                    }
                    let result = match body {
                        Some(Body::Offer(offer)) => offer
                            .encode()
                            .map_err(|_| Error(libdunit::EINVAL))
                            .and_then(|b| Packet::blob(to, message, &b))
                            .and_then(|p| self.outbox.push(p)),
                        Some(Body::Data(bundle)) => {
                            self.deliver(to, message, bundle, custody.unwrap_or(message.transfer))
                        }
                        Some(Body::Move(_)) => Err(Error(libdunit::EINVAL)),
                        None => self.outbox.push(Packet::control(to, message)),
                    };
                    if result.is_err() {
                        self.peer(to, false);
                    }
                }
            }
        }
    }
}
fn native_kind(handle: &OwnedHandle) -> NativeResult<FileKind> {
    let mut s = libdunit::FileStat::default();
    let n = libdunit::file_cap::stat(handle.raw(), &mut s);
    if n < 0 {
        return Err(Error(n));
    }
    match s.file_type {
        libdunit::FILE_TYPE_FILE => Ok(FileKind::File),
        libdunit::FILE_TYPE_DIRECTORY => Ok(FileKind::Directory),
        _ => Err(Error(libdunit::EOPNOTSUPP)),
    }
}
fn errno_status(error: Error) -> Status {
    match error.0 {
        libdunit::EACCES => Status::Denied,
        libdunit::EINVAL => Status::Invalid,
        libdunit::EOPNOTSUPP => Status::Unsupported,
        libdunit::ENOBUFS => Status::TooLarge,
        _ => Status::Failed,
    }
}
