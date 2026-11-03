//! Deterministic transfer policy. Platform code owns capabilities and performs
//! effects; this model never looks up a filesystem path or reads a clock.
use super::*;
use alloc::{collections::BTreeMap, vec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Offer(Offer),
    Data(Bundle),
    Move(FileMove),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Send {
        to: u32,
        message: Message,
        body: Option<Body>,
        custody: Option<u64>,
    },
    /// Drop the platform's escrow handles for this transfer identity.
    Retire(u64),
    Launch {
        opener: u64,
        transfer: u64,
    },
    Move {
        to: u32,
        request: u64,
        transfer: u64,
        custody: u64,
        operation: FileMove,
    },
}
struct Peer {
    last_request: u64,
    target: Option<Offer>,
}
struct Publication {
    source: u32,
    request: u64,
    id: u64,
    offer: Offer,
    deadline: u64,
}
struct Snapshot {
    id: u64,
    offer: Offer,
    bundle: Bundle,
}
struct Read {
    to: u32,
    request: u64,
    custody: u64,
    action: Action,
    deadline: u64,
    moves: Vec<bool>,
    bytes: usize,
}
struct Drag {
    source: u32,
    request: u64,
    id: u64,
    offer: Offer,
    target: Option<u32>,
    opener: Option<u64>,
    launching: bool,
    released: bool,
    rejected: bool,
    proposed: Action,
    accepted: Option<Negotiated>,
    dropped: bool,
    delivered: bool,
    deadline: u64,
    moves: Vec<bool>,
}
/// All deadlines use the caller's monotonic millisecond clock. A finite timeout
/// is supplied by the compositor's transfer configuration.
pub struct Broker {
    peers: BTreeMap<u32, Peer>,
    focus: Option<u32>,
    next: u64,
    timeout: u64,
    clipboard: Option<Snapshot>,
    publishing: Option<Publication>,
    drag: Option<Drag>,
    reads: BTreeMap<u64, Read>,
}
impl Broker {
    pub fn new(timeout_ms: u64) -> Self {
        Self {
            peers: BTreeMap::new(),
            focus: None,
            next: 0,
            timeout: timeout_ms.max(1),
            clipboard: None,
            publishing: None,
            drag: None,
            reads: BTreeMap::new(),
        }
    }
    pub fn set_timeout(&mut self, timeout_ms: u64) {
        self.timeout = timeout_ms.max(1);
    }
    pub fn connect(&mut self, pid: u32) {
        self.peers.entry(pid).or_insert(Peer {
            last_request: 0,
            target: None,
        });
    }
    pub fn focus(&mut self, pid: Option<u32>) {
        self.focus = pid.filter(|p| self.peers.contains_key(p));
    }
    pub fn focused(&self, pid: u32) -> bool {
        self.focus == Some(pid)
    }
    pub fn active_drag(&self) -> Option<(u64, u32, Option<u32>, Option<Action>)> {
        self.drag
            .as_ref()
            .map(|d| (d.id, d.source, d.target, d.accepted.map(|a| a.action)))
    }
    pub fn dragging(&self) -> bool {
        self.drag.as_ref().is_some_and(|d| !d.released)
    }
    fn id(&mut self) -> core::result::Result<u64, Status> {
        self.next = self.next.checked_add(1).ok_or(Status::TooLarge)?;
        Ok(self.next)
    }
    fn deadline(&self, now: u64) -> u64 {
        now.saturating_add(self.timeout)
    }
    fn result(to: u32, m: Message, status: Status) -> Effect {
        send(
            to,
            Kind::Result,
            m.request,
            m.transfer,
            status as u32,
            0,
            None,
            None,
        )
    }
    /// Authenticate a capability grant before the platform accepts custody.
    /// Only the producer of the exact outstanding data request may grant source
    /// handles. A consumer may grant a destination only for an accepted Move.
    pub fn can_grant_source(&self, pid: u32, id: u64) -> bool {
        self.publishing
            .as_ref()
            .is_some_and(|p| p.source == pid && p.id == id)
            || self.drag.as_ref().is_some_and(|d| {
                d.source == pid && d.id == id && d.dropped && !d.delivered && !d.launching
            })
    }
    pub fn can_grant_destination(&self, pid: u32, id: u64) -> bool {
        self.reads
            .get(&id)
            .is_some_and(|r| r.to == pid && r.action == Action::Move)
            || self.drag.as_ref().is_some_and(|d| {
                d.id == id
                    && d.target == Some(pid)
                    && d.delivered
                    && d.accepted.is_some_and(|a| a.action == Action::Move)
            })
    }
    pub fn custody(&self, id: u64) -> Option<u64> {
        self.reads
            .get(&id)
            .map(|r| r.custody)
            .or_else(|| self.drag.as_ref().filter(|d| d.id == id).map(|d| d.id))
    }
    pub fn request(
        &mut self,
        pid: u32,
        m: Message,
        body: Option<Body>,
        now: u64,
        captured: bool,
    ) -> Vec<Effect> {
        if let Err(status) = self.authorize(pid, m) {
            return vec![Self::result(pid, m, status)];
        }
        let result = self.apply(pid, m, body, now, captured);
        result.unwrap_or_else(|status| vec![Self::result(pid, m, status)])
    }
    /// Native Grant/GrantReceived requests share the same replay protection as
    /// modelled requests, before their attachment-specific platform checks.
    pub fn authorize(&mut self, pid: u32, m: Message) -> core::result::Result<(), Status> {
        let peer = self.peers.get_mut(&pid).ok_or(Status::Denied)?;
        if !m.kind.is_request() || m.request == 0 || m.request <= peer.last_request {
            return Err(Status::Stale);
        }
        peer.last_request = m.request;
        Ok(())
    }
    pub fn source_actions(&self, pid: u32, id: u64) -> Option<u32> {
        self.publishing
            .as_ref()
            .filter(|p| p.id == id && p.source == pid)
            .map(|p| p.offer.actions)
            .or_else(|| {
                self.drag
                    .as_ref()
                    .filter(|d| d.id == id && d.source == pid)
                    .map(|d| d.offer.actions)
            })
    }
    fn apply(
        &mut self,
        pid: u32,
        m: Message,
        body: Option<Body>,
        now: u64,
        captured: bool,
    ) -> core::result::Result<Vec<Effect>, Status> {
        match m.kind {
            Kind::ClipboardOffer | Kind::DragOffer => {
                if m.kind == Kind::ClipboardOffer && self.focus != Some(pid) {
                    return Err(Status::Denied);
                }
                if m.transfer != 0 {
                    return Err(Status::Invalid);
                }
                let Some(Body::Offer(offer)) = body else {
                    return Err(Status::Invalid);
                };
                offer.validate().map_err(|_| Status::Invalid)?;
                if m.kind == Kind::DragOffer && (!captured || self.drag.is_some()) {
                    return Err(Status::Busy);
                }
                let id = self.id()?;
                let deadline = self.deadline(now);
                let mut out = Vec::new();
                if m.kind == Kind::ClipboardOffer {
                    if let Some(old) = self.publishing.take() {
                        out.push(send(
                            old.source,
                            Kind::Cancelled,
                            old.request,
                            old.id,
                            Status::Cancelled as u32,
                            0,
                            None,
                            None,
                        ));
                        out.push(Effect::Retire(old.id));
                    }
                    self.publishing = Some(Publication {
                        source: pid,
                        request: m.request,
                        id,
                        offer,
                        deadline,
                    });
                    out.push(send(
                        pid,
                        Kind::RequestData,
                        m.request,
                        id,
                        ALL_FORMATS,
                        0,
                        None,
                        None,
                    ));
                } else {
                    let proposed = offer.preferred;
                    self.drag = Some(Drag {
                        source: pid,
                        request: m.request,
                        id,
                        offer,
                        target: None,
                        opener: None,
                        launching: false,
                        released: false,
                        rejected: false,
                        proposed,
                        accepted: None,
                        dropped: false,
                        delivered: false,
                        deadline,
                        moves: Vec::new(),
                    });
                    out.push(send(
                        pid,
                        Kind::Result,
                        m.request,
                        id,
                        Status::Ok as u32,
                        0,
                        None,
                        None,
                    ));
                }
                Ok(out)
            }
            Kind::RegisterTarget => {
                if m.transfer != 0 {
                    return Err(Status::Invalid);
                }
                let target = match body {
                    Some(Body::Offer(offer)) => {
                        offer.validate().map_err(|_| Status::Invalid)?;
                        Some(offer)
                    }
                    None if m.arg0 | m.arg1 == 0 => None,
                    _ => return Err(Status::Invalid),
                };
                self.peers.get_mut(&pid).ok_or(Status::Denied)?.target = target;
                // Registration changes invalidate a previous acceptance.
                if let Some(d) = self
                    .drag
                    .as_mut()
                    .filter(|d| d.target == Some(pid) && !d.dropped)
                {
                    d.accepted = None;
                }
                Ok(vec![Self::result(pid, m, Status::Ok)])
            }
            Kind::RegisterOpener => {
                if m.transfer != 0 || m.arg0 != 0 || m.arg1 != 0 {
                    return Err(Status::Invalid);
                }
                let Some(Body::Offer(offer)) = body else {
                    return Err(Status::Invalid);
                };
                offer.validate().map_err(|_| Status::Invalid)?;
                if offer.actions != Action::Copy as u32 {
                    return Err(Status::Invalid);
                }
                let mut out = vec![Self::result(pid, m, Status::Ok)];
                if let Some(d) = self
                    .drag
                    .as_mut()
                    .filter(|d| d.launching && d.target == Some(pid))
                {
                    let selected = d.accepted.ok_or(Status::Stale)?;
                    if !offer.formats.contains(&d.offer.formats[selected.format]) {
                        return Ok(self.cancel_drag(Status::Unsupported));
                    }
                    d.launching = false;
                    d.deadline = now.saturating_add(self.timeout);
                    out.push(send(
                        pid,
                        Kind::OpenOffer,
                        d.request,
                        d.id,
                        selected.format as u32,
                        Action::Copy as u32,
                        Some(Body::Offer(d.offer.clone())),
                        None,
                    ));
                    out.push(send(
                        d.source,
                        Kind::RequestData,
                        d.request,
                        d.id,
                        selected.format as u32,
                        0,
                        None,
                        None,
                    ));
                }
                Ok(out)
            }
            Kind::ProvideData => {
                let Some(Body::Data(bundle)) = body else {
                    return Err(Status::Invalid);
                };
                bundle.validate().map_err(|_| Status::Invalid)?;
                if let Some(p) = self
                    .publishing
                    .as_ref()
                    .filter(|p| p.id == m.transfer && p.source == pid)
                {
                    if bundle.representations.len() != p.offer.formats.len()
                        || p.offer.formats.iter().any(|f| bundle.get(f).is_none())
                    {
                        return Err(Status::Invalid);
                    }
                    let p = self.publishing.take().ok_or(Status::Stale)?;
                    let mut out = vec![
                        send(
                            pid,
                            Kind::Result,
                            p.request,
                            p.id,
                            Status::Ok as u32,
                            0,
                            None,
                            None,
                        ),
                        Self::result(pid, m, Status::Ok),
                    ];
                    if let Some(old) = self.clipboard.replace(Snapshot {
                        id: p.id,
                        offer: p.offer,
                        bundle,
                    }) {
                        // Readers retain escrow independently until Complete.
                        if !self.reads.values().any(|r| r.custody == old.id) {
                            out.push(Effect::Retire(old.id));
                        }
                    }
                    self.changed(&mut out);
                    Ok(out)
                } else {
                    let d = self
                        .drag
                        .as_mut()
                        .filter(|d| {
                            d.id == m.transfer
                                && d.source == pid
                                && d.dropped
                                && !d.delivered
                                && !d.launching
                        })
                        .ok_or(Status::Stale)?;
                    let selected = d.accepted.ok_or(Status::Stale)?;
                    if bundle.representations.len() != 1
                        || bundle.representations[0].format != d.offer.formats[selected.format]
                    {
                        return Err(Status::Invalid);
                    }
                    d.moves = move_slots(&bundle, selected.action);
                    d.delivered = true;
                    let to = d.target.ok_or(Status::TargetGone)?;
                    Ok(vec![
                        Self::result(pid, m, Status::Ok),
                        send(
                            to,
                            if d.opener.is_some() {
                                Kind::OpenData
                            } else {
                                Kind::Data
                            },
                            d.request,
                            d.id,
                            selected.format as u32,
                            selected.action as u32,
                            Some(Body::Data(bundle)),
                            Some(d.id),
                        ),
                    ])
                }
            }
            Kind::ClipboardRead => {
                if self.focus != Some(pid) {
                    return Err(Status::Denied);
                }
                if m.transfer != 0 {
                    return Err(Status::Invalid);
                }
                let Some(Body::Offer(target)) = body else {
                    return Err(Status::Invalid);
                };
                target.validate().map_err(|_| Status::Invalid)?;
                if self.reads.len() >= 128 {
                    return Err(Status::Busy);
                }
                let snapshot = self.clipboard.as_ref().ok_or(Status::Stale)?;
                let selection = negotiate(&snapshot.offer, &target, target.preferred)
                    .ok_or(Status::Unsupported)?;
                let offer = snapshot.offer.clone();
                let custody = snapshot.id;
                if selection.action == Action::Move
                    && self
                        .reads
                        .values()
                        .any(|r| r.custody == custody && r.action == Action::Move)
                {
                    return Err(Status::Busy);
                }
                let rep = snapshot
                    .bundle
                    .representations
                    .iter()
                    .find(|r| r.format == offer.formats[selection.format])
                    .ok_or(Status::Failed)?;
                let bytes = rep.data.len() + rep.format.name().len() + 16;
                if self
                    .reads
                    .values()
                    .map(|r| r.bytes)
                    .sum::<usize>()
                    .saturating_add(bytes)
                    > MAX_IN_FLIGHT_BYTES
                {
                    return Err(Status::Busy);
                }
                let data = Bundle {
                    representations: vec![rep.clone()],
                };
                let id = self.id()?;
                self.reads.insert(
                    id,
                    Read {
                        to: pid,
                        request: m.request,
                        custody,
                        action: selection.action,
                        deadline: self.deadline(now),
                        moves: move_slots(&data, selection.action),
                        bytes,
                    },
                );
                Ok(vec![
                    send(
                        pid,
                        Kind::Offer,
                        m.request,
                        id,
                        selection.format as u32,
                        selection.action as u32,
                        Some(Body::Offer(offer)),
                        None,
                    ),
                    send(
                        pid,
                        Kind::Data,
                        m.request,
                        id,
                        selection.format as u32,
                        selection.action as u32,
                        Some(Body::Data(data)),
                        Some(custody),
                    ),
                ])
            }
            Kind::DragAccept => {
                if body.is_some() {
                    return Err(Status::Invalid);
                }
                let d = self
                    .drag
                    .as_mut()
                    .filter(|d| d.id == m.transfer && d.target == Some(pid) && !d.dropped)
                    .ok_or(Status::Stale)?;
                if m.arg0 == ALL_FORMATS && m.arg1 == 0 {
                    d.accepted = None;
                    d.rejected = true;
                    let source = d.source;
                    let request = d.request;
                    let id = d.id;
                    let released = d.released;
                    let mut out = vec![
                        Self::result(pid, m, Status::Ok),
                        send(
                            source,
                            Kind::DragAction,
                            request,
                            id,
                            ALL_FORMATS,
                            0,
                            None,
                            None,
                        ),
                    ];
                    if released {
                        out.extend(self.cancel_drag(Status::Unsupported));
                    }
                    return Ok(out);
                }
                let action = Action::from_u32(m.arg1).ok_or(Status::Invalid)?;
                if action != d.proposed {
                    return Err(Status::Denied);
                }
                let target = self
                    .peers
                    .get(&pid)
                    .and_then(|p| p.target.as_ref())
                    .ok_or(Status::Unsupported)?;
                let selected = negotiate(&d.offer, target, action).ok_or(Status::Unsupported)?;
                if m.arg0 as usize != selected.format {
                    return Err(Status::Invalid);
                }
                d.accepted = Some(selected);
                d.rejected = false;
                let source = d.source;
                let request = d.request;
                let id = d.id;
                let mut out = vec![
                    Self::result(pid, m, Status::Ok),
                    send(
                        source,
                        Kind::DragAction,
                        request,
                        id,
                        m.arg0,
                        m.arg1,
                        None,
                        None,
                    ),
                ];
                if d.released {
                    d.dropped = true;
                    d.deadline = now.saturating_add(self.timeout);
                    out.push(send(
                        pid,
                        Kind::DragDrop,
                        request,
                        id,
                        m.arg0,
                        m.arg1,
                        None,
                        None,
                    ));
                    out.push(send(
                        source,
                        Kind::RequestData,
                        request,
                        id,
                        m.arg0,
                        0,
                        None,
                        None,
                    ));
                }
                Ok(out)
            }
            Kind::Complete => {
                if body.is_some() || m.arg1 != 0 {
                    return Err(Status::Invalid);
                }
                let status = Status::from_u32(m.arg0).ok_or(Status::Invalid)?;
                let mut out = vec![Self::result(pid, m, Status::Ok)];
                if let Some(read) = self.reads.get(&m.transfer).filter(|r| r.to == pid) {
                    if status == Status::Ok && read.moves.iter().any(|done| !*done) {
                        return Err(Status::Busy);
                    }
                    let custody = read.custody;
                    let moved = read.action == Action::Move
                        && (status == Status::Ok || read.moves.iter().any(|done| *done));
                    let read = self.reads.remove(&m.transfer).ok_or(Status::Stale)?;
                    out.push(send(
                        pid,
                        Kind::Finished,
                        read.request,
                        m.transfer,
                        status as u32,
                        0,
                        None,
                        None,
                    ));
                    if moved && self.clipboard.as_ref().is_some_and(|s| s.id == custody) {
                        self.clipboard = None;
                        self.changed(&mut out);
                    }
                    self.retire_unused(custody, &mut out);
                } else {
                    let d = self
                        .drag
                        .as_ref()
                        .filter(|d| {
                            d.id == m.transfer
                                && d.target == Some(pid)
                                && d.dropped
                                && (d.delivered || status != Status::Ok)
                        })
                        .ok_or(Status::Stale)?;
                    if status == Status::Ok && d.moves.iter().any(|done| !*done) {
                        return Err(Status::Busy);
                    }
                    let source = d.source;
                    let request = d.request;
                    out.push(send(
                        source,
                        Kind::Finished,
                        request,
                        m.transfer,
                        status as u32,
                        0,
                        None,
                        None,
                    ));
                    out.push(send(
                        pid,
                        Kind::Finished,
                        request,
                        m.transfer,
                        status as u32,
                        0,
                        None,
                        None,
                    ));
                    self.drag = None;
                    out.push(Effect::Retire(m.transfer));
                }
                Ok(out)
            }
            Kind::FileMove => {
                let Some(Body::Move(operation)) = body else {
                    return Err(Status::Invalid);
                };
                operation.encode().map_err(|_| Status::Invalid)?;
                if !self.can_grant_destination(pid, m.transfer) {
                    return Err(Status::Denied);
                }
                let remaining = self
                    .reads
                    .get(&m.transfer)
                    .map(|r| &r.moves)
                    .or_else(|| {
                        self.drag
                            .as_ref()
                            .filter(|d| d.id == m.transfer)
                            .map(|d| &d.moves)
                    })
                    .ok_or(Status::Stale)?;
                if remaining.get(operation.item as usize) != Some(&false) {
                    return Err(Status::Stale);
                }
                let custody = self.custody(m.transfer).ok_or(Status::Stale)?;
                Ok(vec![Effect::Move {
                    to: pid,
                    request: m.request,
                    transfer: m.transfer,
                    custody,
                    operation,
                }])
            }
            Kind::Cancel => {
                if body.is_some() {
                    return Err(Status::Invalid);
                }
                if self.drag.as_ref().is_some_and(|d| {
                    d.id == m.transfer && (d.source == pid || d.target == Some(pid))
                }) {
                    let mut out = self.cancel_drag(Status::Cancelled);
                    out.push(Self::result(pid, m, Status::Ok));
                    Ok(out)
                } else if self
                    .publishing
                    .as_ref()
                    .is_some_and(|p| p.id == m.transfer && p.source == pid)
                {
                    let p = self.publishing.take().ok_or(Status::Stale)?;
                    Ok(vec![
                        send(
                            pid,
                            Kind::Cancelled,
                            p.request,
                            p.id,
                            Status::Cancelled as u32,
                            0,
                            None,
                            None,
                        ),
                        Effect::Retire(p.id),
                        Self::result(pid, m, Status::Ok),
                    ])
                } else {
                    Err(Status::Stale)
                }
            }
            Kind::ClipboardClear => {
                if self.focus != Some(pid) {
                    return Err(Status::Denied);
                }
                if body.is_some() || m.transfer != 0 {
                    return Err(Status::Invalid);
                }
                let mut out = vec![Self::result(pid, m, Status::Ok)];
                if let Some(p) = self.publishing.take() {
                    out.push(send(
                        p.source,
                        Kind::Cancelled,
                        p.request,
                        p.id,
                        Status::Cancelled as u32,
                        0,
                        None,
                        None,
                    ));
                    out.push(Effect::Retire(p.id));
                }
                if let Some(old) = self.clipboard.take() {
                    self.retire_unused(old.id, &mut out);
                    self.changed(&mut out);
                }
                Ok(out)
            }
            Kind::ClipboardQuery => {
                if self.focus != Some(pid) {
                    return Err(Status::Denied);
                }
                if body.is_some() || m.transfer != 0 {
                    return Err(Status::Invalid);
                }
                let s = self.clipboard.as_ref().ok_or(Status::Stale)?;
                Ok(vec![send(
                    pid,
                    Kind::Offer,
                    m.request,
                    s.id,
                    0,
                    0,
                    Some(Body::Offer(s.offer.clone())),
                    None,
                )])
            }
            _ => Err(Status::Unsupported),
        }
    }
    /// Called only by the native platform after the capability-scoped move has
    /// returned. A consumer cannot declare a file Move successful itself.
    pub fn moved(&mut self, to: u32, request: u64, id: u64, item: u32, status: Status) -> Effect {
        let slots = if let Some(r) = self
            .reads
            .get_mut(&id)
            .filter(|r| r.to == to && r.action == Action::Move)
        {
            Some(&mut r.moves)
        } else {
            self.drag
                .as_mut()
                .filter(|d| {
                    d.id == id
                        && d.target == Some(to)
                        && d.delivered
                        && d.accepted.is_some_and(|a| a.action == Action::Move)
                })
                .map(|d| &mut d.moves)
        };
        let status = match slots.and_then(|s| s.get_mut(item as usize)) {
            Some(done) if !*done => {
                if status == Status::Ok {
                    *done = true;
                }
                status
            }
            _ => Status::Stale,
        };
        send(to, Kind::Result, request, id, status as u32, 0, None, None)
    }
    fn changed(&self, out: &mut Vec<Effect>) {
        for &to in self.peers.keys() {
            out.push(send(
                to,
                Kind::ClipboardChanged,
                0,
                self.clipboard.as_ref().map_or(0, |s| s.id),
                0,
                0,
                None,
                None,
            ));
        }
    }
    fn abandoned_read(&mut self, id: u64, out: &mut Vec<Effect>) -> Option<Read> {
        let read = self.reads.remove(&id)?;
        if read.moves.iter().any(|done| *done)
            && self
                .clipboard
                .as_ref()
                .is_some_and(|s| s.id == read.custody)
        {
            self.clipboard = None;
            self.changed(out);
        }
        self.retire_unused(read.custody, out);
        Some(read)
    }
    fn retire_unused(&self, id: u64, out: &mut Vec<Effect>) {
        if !self.clipboard.as_ref().is_some_and(|s| s.id == id)
            && !self.reads.values().any(|r| r.custody == id)
        {
            out.push(Effect::Retire(id));
        }
    }
    /// Dock policy supplies metadata, not an application PID or file path. The
    /// source is contacted only after drop and the newly launched peer opts in.
    pub fn pointer_opener(
        &mut self,
        opener: u64,
        offer: &Offer,
        requested: Option<Action>,
        now: u64,
    ) -> Vec<Effect> {
        let Some(d) = self.drag.as_mut().filter(|d| !d.released) else {
            return Vec::new();
        };
        let selected = if requested.is_none_or(|a| a == Action::Copy) {
            negotiate(&d.offer, offer, Action::Copy)
        } else {
            None
        };
        let mut out = Vec::new();
        if let Some(old) = d.target.take() {
            out.push(send(
                old,
                Kind::DragLeave,
                d.request,
                d.id,
                0,
                0,
                None,
                None,
            ));
        }
        d.opener = Some(opener);
        d.proposed = Action::Copy;
        d.accepted = selected;
        d.deadline = now.saturating_add(self.timeout);
        if let Some(selected) = selected {
            out.push(send(
                d.source,
                Kind::DragAction,
                d.request,
                d.id,
                selected.format as u32,
                Action::Copy as u32,
                None,
                None,
            ));
        }
        out
    }
    pub fn active_opener(&self) -> Option<u64> {
        self.drag
            .as_ref()
            .filter(|d| d.accepted.is_some())
            .and_then(|d| d.opener)
    }
    pub fn expect_opener(&mut self, id: u64, pid: u32, now: u64) -> bool {
        let Some(d) = self
            .drag
            .as_mut()
            .filter(|d| d.id == id && d.launching && d.target.is_none())
        else {
            return false;
        };
        d.target = Some(pid);
        d.deadline = now.saturating_add(self.timeout);
        true
    }
    pub fn fail_opener(&mut self, id: u64, status: Status) -> Vec<Effect> {
        if self
            .drag
            .as_ref()
            .is_some_and(|d| d.id == id && d.launching)
        {
            self.cancel_drag(status)
        } else {
            Vec::new()
        }
    }
    /// Input routing supplies the topmost visible target and local coordinates.
    /// It deliberately remains independent of keyboard focus/workspace changes.
    pub fn pointer(
        &mut self,
        hit: Option<(u32, i32, i32)>,
        requested: Option<Action>,
        now: u64,
    ) -> Vec<Effect> {
        let Some(d) = self.drag.as_mut().filter(|d| !d.released) else {
            return Vec::new();
        };
        if d.opener.take().is_some() {
            d.accepted = None;
        }
        let target = hit.and_then(|hit| {
            self.peers
                .get(&hit.0)
                .and_then(|p| p.target.as_ref())
                .and_then(|target| {
                    let action = if let Some(requested) = requested {
                        requested
                    } else {
                        [
                            d.offer.preferred,
                            target.preferred,
                            Action::Copy,
                            Action::Move,
                            Action::Link,
                        ]
                        .into_iter()
                        .find(|action| d.offer.actions & target.actions & *action as u32 != 0)?
                    };
                    negotiate(&d.offer, target, action).map(|_| (hit, action))
                })
        });
        let action = target.map_or(requested.unwrap_or(d.offer.preferred), |(_, action)| action);
        let target = target.map(|(hit, _)| hit);
        let pid = target.map(|t| t.0);
        let mut out = Vec::new();
        if d.target != pid || d.proposed != action {
            if let Some(old) = d.target {
                out.push(send(
                    old,
                    Kind::DragLeave,
                    d.request,
                    d.id,
                    0,
                    0,
                    None,
                    None,
                ));
            }
            d.target = pid;
            d.proposed = action;
            d.accepted = None;
            d.rejected = false;
            if let Some(to) = pid {
                out.push(send(
                    to,
                    Kind::DragEnter,
                    d.request,
                    d.id,
                    0,
                    action as u32,
                    Some(Body::Offer(d.offer.clone())),
                    None,
                ));
            }
        }
        if let Some((to, x, y)) = target {
            out.push(send(
                to,
                Kind::DragMotion,
                d.request,
                d.id,
                x as u32,
                y as u32,
                None,
                None,
            ));
        }
        d.deadline = now.saturating_add(self.timeout);
        out
    }
    /// Called for the source's pointer-up; the caller consumes it instead of
    /// delivering a normal client click. No provider data is requested earlier.
    pub fn drop_pointer(&mut self, now: u64) -> Vec<Effect> {
        let Some(d) = self.drag.as_mut().filter(|d| !d.released) else {
            return Vec::new();
        };
        d.released = true;
        d.deadline = now.saturating_add(self.timeout);
        if d.target.is_some() && d.accepted.is_none() && !d.rejected {
            return Vec::new();
        }
        if let (Some(opener), Some(_)) = (d.opener, d.accepted) {
            d.dropped = true;
            d.launching = true;
            d.deadline = now.saturating_add(self.timeout);
            return vec![Effect::Launch {
                opener,
                transfer: d.id,
            }];
        }
        let (Some(to), Some(selected)) = (d.target, d.accepted) else {
            return self.cancel_drag(Status::Unsupported);
        };
        d.dropped = true;
        d.deadline = now.saturating_add(self.timeout);
        vec![
            send(
                to,
                Kind::DragDrop,
                d.request,
                d.id,
                selected.format as u32,
                selected.action as u32,
                None,
                None,
            ),
            send(
                d.source,
                Kind::RequestData,
                d.request,
                d.id,
                selected.format as u32,
                0,
                None,
                None,
            ),
        ]
    }
    pub fn cancel_drag(&mut self, status: Status) -> Vec<Effect> {
        let Some(d) = self.drag.take() else {
            return Vec::new();
        };
        let mut out = vec![send(
            d.source,
            Kind::Cancelled,
            d.request,
            d.id,
            status as u32,
            0,
            None,
            None,
        )];
        if let Some(to) = d.target.filter(|p| *p != d.source) {
            out.push(send(
                to,
                Kind::Cancelled,
                d.request,
                d.id,
                status as u32,
                0,
                None,
                None,
            ));
        }
        out.push(Effect::Retire(d.id));
        out
    }
    pub fn disconnect(&mut self, pid: u32) -> Vec<Effect> {
        self.peers.remove(&pid);
        if self.focus == Some(pid) {
            self.focus = None;
        }
        let mut out = Vec::new();
        if self
            .drag
            .as_ref()
            .is_some_and(|d| d.source == pid || d.target == Some(pid))
        {
            out.extend(
                self.cancel_drag(if self.drag.as_ref().is_some_and(|d| d.source == pid) {
                    Status::SourceGone
                } else {
                    Status::TargetGone
                }),
            );
        }
        if self.publishing.as_ref().is_some_and(|p| p.source == pid) {
            let p = self.publishing.take().unwrap();
            out.push(Effect::Retire(p.id));
        }
        let ids: Vec<_> = self
            .reads
            .iter()
            .filter(|(_, r)| r.to == pid)
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            self.abandoned_read(id, &mut out);
        }
        // A published clipboard has independent escrow and survives source exit.
        out
    }
    pub fn tick(&mut self, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if self.drag.as_ref().is_some_and(|d| now >= d.deadline) {
            out.extend(self.cancel_drag(Status::Timeout));
        }
        if self.publishing.as_ref().is_some_and(|p| now >= p.deadline) {
            let p = self.publishing.take().unwrap();
            out.push(send(
                p.source,
                Kind::Cancelled,
                p.request,
                p.id,
                Status::Timeout as u32,
                0,
                None,
                None,
            ));
            out.push(Effect::Retire(p.id));
        }
        let ids: Vec<_> = self
            .reads
            .iter()
            .filter(|(_, r)| now >= r.deadline)
            .map(|(&id, _)| id)
            .collect();
        for id in ids {
            if let Some(r) = self.abandoned_read(id, &mut out) {
                out.push(send(
                    r.to,
                    Kind::Cancelled,
                    r.request,
                    id,
                    Status::Timeout as u32,
                    0,
                    None,
                    None,
                ));
            }
        }
        out
    }
}
fn send(
    to: u32,
    kind: Kind,
    request: u64,
    transfer: u64,
    arg0: u32,
    arg1: u32,
    body: Option<Body>,
    custody: Option<u64>,
) -> Effect {
    Effect::Send {
        to,
        message: Message {
            kind,
            request,
            transfer,
            arg0,
            arg1,
        },
        body,
        custody,
    }
}

fn move_slots(bundle: &Bundle, action: Action) -> Vec<bool> {
    if action != Action::Move {
        return Vec::new();
    }
    bundle
        .get(&Format::Files)
        .and_then(|b| FileList::decode(b).ok())
        .map_or_else(Vec::new, |files| vec![false; files.items.len()])
}
