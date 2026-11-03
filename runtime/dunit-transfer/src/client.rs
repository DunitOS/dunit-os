use super::*;
use alloc::collections::BTreeMap;
use transport::{Outbox, Packet};
#[derive(Debug)]
pub struct Event {
    pub message: Message,
    pub offer: Option<Offer>,
    pub payload: Option<Payload>,
    pub source: bool,
}
struct Source {
    kind: Kind,
    payload: Payload,
    offer: Offer,
    transfer: Option<u64>,
}
struct Work {
    bundle: Bundle,
    waiting: BTreeMap<u64, usize>,
    handles: Vec<u32>,
}
/// Async transfer session. All packets must come from the authenticated GUI
/// server through the embedding window's existing receive pump.
pub struct Client {
    peer: u32,
    features: u64,
    next: u64,
    provided: u64,
    outbox: Outbox,
    sources: BTreeMap<u64, Source>,
    reads: BTreeMap<u64, Option<u64>>,
    work: BTreeMap<u64, Work>,
    receiving: BTreeMap<u64, Vec<(u32, OwnedHandle)>>,
}
impl Client {
    pub fn new(peer: u32, features: u64) -> Self {
        Self {
            peer,
            features,
            next: 0,
            provided: 0,
            outbox: Outbox::default(),
            sources: BTreeMap::new(),
            reads: BTreeMap::new(),
            work: BTreeMap::new(),
            receiving: BTreeMap::new(),
        }
    }
    pub fn provider_requests(&self) -> u64 {
        self.provided
    }
    pub fn set_features(&mut self, features: u64) {
        self.features = features;
    }
    pub fn pending_send(&self) -> bool {
        !self.outbox.is_empty()
    }
    pub fn flush(&mut self) -> NativeResult<()> {
        if let Some((_, _, error)) = self.outbox.flush().into_iter().next() {
            return Err(error);
        }
        Ok(())
    }
    fn serial(&mut self) -> NativeResult<u64> {
        if self.features & gui_protocol_v1::wire::FEATURE_DATA_TRANSFER == 0 {
            return Err(Error(libdunit::EOPNOTSUPP));
        }
        self.next = self.next.checked_add(1).ok_or(Error(libdunit::EINVAL))?;
        Ok(self.next)
    }
    fn control(&mut self, kind: Kind, id: u64, arg0: u32, arg1: u32) -> NativeResult<u64> {
        let request = self.serial()?;
        self.outbox.push(Packet::control(
            self.peer,
            Message {
                kind,
                request,
                transfer: id,
                arg0,
                arg1,
            },
        ))?;
        Ok(request)
    }
    fn blob(&mut self, kind: Kind, id: u64, bytes: &[u8]) -> NativeResult<u64> {
        let request = self.serial()?;
        let packet = Packet::blob(
            self.peer,
            Message {
                kind,
                request,
                transfer: id,
                arg0: 0,
                arg1: 0,
            },
            bytes,
        )?;
        self.outbox.push(packet)?;
        Ok(request)
    }
    fn offer(
        &mut self,
        kind: Kind,
        payload: Payload,
        actions: u32,
        preferred: Action,
    ) -> NativeResult<u64> {
        if self.sources.values().filter(|s| s.kind == kind).count() >= 2 {
            return Err(Error(libdunit::EBUSY));
        }
        let offer = payload.offer(actions, preferred)?;
        if actions & Action::Move as u32 != 0
            && payload.files.iter().any(|f| {
                f.handle
                    .rights()
                    .map_or(true, |r| r & libdunit::RIGHT_WRITE == 0)
            })
        {
            return Err(Error(libdunit::EACCES));
        }
        let request = self.blob(kind, 0, &offer.encode().map_err(invalid)?)?;
        self.sources.insert(
            request,
            Source {
                kind,
                payload,
                offer,
                transfer: None,
            },
        );
        Ok(request)
    }
    pub fn publish_clipboard(
        &mut self,
        payload: Payload,
        actions: u32,
        preferred: Action,
    ) -> NativeResult<u64> {
        self.offer(Kind::ClipboardOffer, payload, actions, preferred)
    }
    pub fn start_drag(
        &mut self,
        payload: Payload,
        actions: u32,
        preferred: Action,
    ) -> NativeResult<u64> {
        self.offer(Kind::DragOffer, payload, actions, preferred)
    }
    pub fn read_clipboard(&mut self, preferences: Offer) -> NativeResult<u64> {
        if self.reads.len() >= 128 {
            return Err(Error(libdunit::ENOBUFS));
        }
        let request = self.blob(
            Kind::ClipboardRead,
            0,
            &preferences.encode().map_err(invalid)?,
        )?;
        self.reads.insert(request, None);
        Ok(request)
    }
    pub fn register_opener(&mut self, preferences: Offer) -> NativeResult<u64> {
        self.blob(
            Kind::RegisterOpener,
            0,
            &preferences.encode().map_err(invalid)?,
        )
    }
    pub fn register_target(&mut self, preferences: Offer) -> NativeResult<u64> {
        self.blob(
            Kind::RegisterTarget,
            0,
            &preferences.encode().map_err(invalid)?,
        )
    }
    pub fn unregister_target(&mut self) -> NativeResult<u64> {
        self.control(Kind::RegisterTarget, 0, 0, 0)
    }
    pub fn drag_reject(&mut self, id: u64) -> NativeResult<u64> {
        self.control(Kind::DragAccept, id, ALL_FORMATS, 0)
    }
    pub fn drag_accept(&mut self, id: u64, format: usize, action: Action) -> NativeResult<u64> {
        self.control(Kind::DragAccept, id, format as u32, action as u32)
    }
    pub fn complete(&mut self, id: u64, status: Status) -> NativeResult<u64> {
        self.control(Kind::Complete, id, status as u32, 0)
    }
    pub fn cancel(&mut self, id: u64) -> NativeResult<u64> {
        self.control(Kind::Cancel, id, 0, 0)
    }
    pub fn clear(&mut self) -> NativeResult<u64> {
        self.control(Kind::ClipboardClear, 0, 0, 0)
    }
    pub fn query(&mut self) -> NativeResult<u64> {
        self.control(Kind::ClipboardQuery, 0, 0, 0)
    }
    /// Grant destination authority first. The returned request is acknowledged
    /// with GrantAck; then submit file_move using that broker-local handle.
    pub fn grant_destination(&mut self, id: u64, directory: &OwnedHandle) -> NativeResult<u64> {
        if stat_kind(directory)? != FileKind::Directory {
            return Err(Error(libdunit::ENOTDIR));
        }
        let rights = libdunit::RIGHT_READ | libdunit::RIGHT_WRITE | libdunit::RIGHT_TRANSFER;
        let handle = directory.duplicate(rights)?;
        let request = self.serial()?;
        self.outbox.push(Packet::handle(
            self.peer,
            Message {
                kind: Kind::Grant,
                request,
                transfer: id,
                arg0: 0,
                arg1: ALL_FORMATS,
            },
            handle,
            rights,
        ))?;
        Ok(request)
    }
    pub fn file_move(&mut self, id: u64, operation: FileMove) -> NativeResult<u64> {
        self.blob(Kind::FileMove, id, &operation.encode().map_err(invalid)?)
    }
    fn provide(&mut self, message: Message) -> NativeResult<()> {
        if self.work.contains_key(&message.transfer) {
            return Err(Error(libdunit::EBUSY));
        }
        let source = self
            .sources
            .get_mut(&message.request)
            .ok_or(Error(libdunit::EINVAL))?;
        if source.transfer.is_some_and(|id| id != message.transfer) {
            return Err(Error(libdunit::EINVAL));
        }
        source.transfer = Some(message.transfer);
        self.provided = self.provided.saturating_add(1);
        let mut bundle = source.payload.bundle.clone();
        if message.arg0 != ALL_FORMATS {
            if source.kind != Kind::DragOffer {
                return Err(Error(libdunit::EINVAL));
            }
            let selected = source
                .offer
                .formats
                .get(message.arg0 as usize)
                .ok_or(Error(libdunit::EINVAL))?;
            bundle.representations.retain(|r| &r.format == selected);
        } else if source.kind != Kind::ClipboardOffer {
            return Err(Error(libdunit::EINVAL));
        }
        let mut grants = Vec::new();
        if bundle.get(&Format::Files).is_some() {
            for file in &source.payload.files {
                let rights = libdunit::RIGHT_READ
                    | libdunit::RIGHT_TRANSFER
                    | if source.offer.actions & Action::Move as u32 != 0 {
                        libdunit::RIGHT_WRITE
                    } else {
                        0
                    };
                grants.push((file.handle.duplicate(rights)?, rights));
            }
        }
        let mut work = Work {
            bundle,
            waiting: BTreeMap::new(),
            handles: alloc::vec![0;grants.len()],
        };
        for (index, (handle, rights)) in grants.into_iter().enumerate() {
            let request = self.serial()?;
            self.outbox.push(Packet::handle(
                self.peer,
                Message {
                    kind: Kind::Grant,
                    request,
                    transfer: message.transfer,
                    arg0: 0,
                    arg1: index as u32,
                },
                handle,
                rights,
            ))?;
            work.waiting.insert(request, index);
        }
        if work.waiting.is_empty() {
            self.blob(
                Kind::ProvideData,
                message.transfer,
                &work.bundle.encode().map_err(invalid)?,
            )?;
        } else {
            self.work.insert(message.transfer, work);
        }
        Ok(())
    }
    /// Takes ownership only of `attachment`, never an untrusted wire number.
    /// The embedding pump must verify sender PID before calling this method.
    pub fn handle_packet(
        &mut self,
        bytes: &[u8],
        attachment: Option<OwnedHandle>,
    ) -> NativeResult<Option<Event>> {
        let mut message = Message::decode(bytes).map_err(invalid)?;
        if message.kind.is_request() && message.kind != Kind::Grant {
            return Err(Error(libdunit::EINVAL));
        }
        let mut offer = None;
        let mut payload = None;
        match message.kind {
            Kind::RequestData => {
                if attachment.is_some() {
                    return Err(Error(libdunit::EINVAL));
                }
                self.provide(message)?;
                return Ok(None);
            }
            Kind::GrantAck => {
                if attachment.is_some() {
                    return Err(Error(libdunit::EINVAL));
                }
                let id = self
                    .work
                    .iter()
                    .find(|(_, w)| w.waiting.contains_key(&message.request))
                    .map(|(&id, _)| id);
                if let Some(id) = id {
                    if message.transfer != id
                        || message.arg1 != Status::Ok as u32
                        || message.arg0 == 0
                    {
                        self.work.remove(&id);
                        self.cancel(id)?;
                        return Err(Error(libdunit::EIO));
                    }
                    let work = self.work.get_mut(&id).ok_or(Error(libdunit::EINVAL))?;
                    let index = work
                        .waiting
                        .remove(&message.request)
                        .ok_or(Error(libdunit::EINVAL))?;
                    work.handles[index] = message.arg0;
                    if work.waiting.is_empty() {
                        let mut work = self.work.remove(&id).ok_or(Error(libdunit::EINVAL))?;
                        let rep = work
                            .bundle
                            .representations
                            .iter_mut()
                            .find(|r| r.format == Format::Files)
                            .ok_or(Error(libdunit::EINVAL))?;
                        let mut list = FileList::decode(&rep.data).map_err(invalid)?;
                        for (file, &handle) in list.items.iter_mut().zip(&work.handles) {
                            file.capability = handle;
                        }
                        rep.data = list.encode().map_err(invalid)?;
                        self.blob(
                            Kind::ProvideData,
                            id,
                            &work.bundle.encode().map_err(invalid)?,
                        )?;
                    }
                    return Ok(None);
                }
            }
            Kind::Grant => {
                let handle = attachment.ok_or(Error(libdunit::EACCES))?;
                if handle.raw() != message.arg0
                    || message.arg1 as usize >= MAX_FILE_ITEMS
                    || handle.rights()? != (libdunit::RIGHT_READ | libdunit::RIGHT_TRANSFER)
                {
                    return Err(Error(libdunit::EACCES));
                }
                let files = self
                    .receiving
                    .get_mut(&message.transfer)
                    .ok_or(Error(libdunit::EINVAL))?;
                if files.iter().any(|(index, _)| *index == message.arg1) {
                    return Err(Error(libdunit::EINVAL));
                }
                stat_kind(&handle)?;
                let raw = handle.raw();
                files.push((message.arg1, handle));
                self.control(Kind::GrantReceived, message.transfer, raw, message.arg1)?;
                return Ok(None);
            }
            Kind::Offer | Kind::DragEnter | Kind::OpenOffer => {
                let (decoded, bytes) = transport::read_blob(message, attachment)?;
                message = decoded;
                offer = Some(Offer::decode(&bytes).map_err(invalid)?);
                if self.receiving.len() >= 128 && !self.receiving.contains_key(&message.transfer) {
                    return Err(Error(libdunit::ENOBUFS));
                }
                if message.kind == Kind::Offer {
                    if let Some(id) = self.reads.get_mut(&message.request) {
                        if id.is_some_and(|id| id != message.transfer) {
                            return Err(Error(libdunit::EINVAL));
                        }
                        *id = Some(message.transfer);
                    }
                }
                if matches!(message.kind, Kind::DragEnter | Kind::OpenOffer)
                    || self.reads.contains_key(&message.request)
                {
                    self.receiving.entry(message.transfer).or_default();
                }
            }
            Kind::Data | Kind::OpenData => {
                let (decoded, bytes) = transport::read_blob(message, attachment)?;
                message = decoded;
                let bundle = Bundle::decode(&bytes).map_err(invalid)?;
                let mut grants = self
                    .receiving
                    .remove(&message.transfer)
                    .ok_or(Error(libdunit::EINVAL))?;
                let mut files = Vec::new();
                if let Some(data) = bundle.get(&Format::Files) {
                    let list = FileList::decode(data).map_err(invalid)?;
                    for (index, item) in list.items.into_iter().enumerate() {
                        let at = grants
                            .iter()
                            .position(|(slot, cap)| {
                                *slot == index as u32 && cap.raw() == item.capability
                            })
                            .ok_or(Error(libdunit::EACCES))?;
                        let (_, handle) = grants.swap_remove(at);
                        if stat_kind(&handle)? != item.kind {
                            return Err(Error(libdunit::EINVAL));
                        }
                        files.push(File {
                            handle,
                            path: item.path,
                            kind: item.kind,
                        });
                    }
                }
                if !grants.is_empty() {
                    return Err(Error(libdunit::EINVAL));
                }
                if self.reads.get(&message.request) == Some(&Some(message.transfer)) {
                    self.reads.remove(&message.request);
                }
                payload = Some(Payload::new(bundle, files)?);
            }
            _ => {
                if attachment.is_some() {
                    return Err(Error(libdunit::EINVAL));
                }
            }
        }
        let source_request = self
            .sources
            .iter()
            .find(|(_, source)| source.transfer == Some(message.transfer))
            .map(|(&request, _)| request);
        let source = source_request.is_some()
            || (message.kind == Kind::Result && self.sources.contains_key(&message.request));
        if matches!(message.kind, Kind::Cancelled | Kind::Finished) {
            self.receiving.remove(&message.transfer);
            self.work.remove(&message.transfer);
            self.outbox.discard_transfer(message.transfer);
            if let Some(request) = source_request {
                self.sources.remove(&request);
            }
            // A target's request field may belong to another source process;
            // only operation identity, not that number, identifies its reads.
            self.reads.retain(|_, id| *id != Some(message.transfer));
        }
        if message.kind == Kind::DragLeave {
            self.receiving.remove(&message.transfer);
        }
        if message.kind == Kind::Result {
            if message.arg0 != Status::Ok as u32 {
                self.reads.remove(&message.request);
                if let Some(id) = self
                    .work
                    .iter()
                    .find(|(_, w)| w.waiting.contains_key(&message.request))
                    .map(|(&id, _)| id)
                {
                    self.work.remove(&id);
                    self.cancel(id)?;
                }
            }
            if let Some(source) = self.sources.get_mut(&message.request) {
                if message.arg0 == Status::Ok as u32 {
                    source.transfer = Some(message.transfer);
                }
                if source.kind == Kind::ClipboardOffer || message.arg0 != Status::Ok as u32 {
                    self.sources.remove(&message.request);
                }
            }
        }
        Ok(Some(Event {
            message,
            offer,
            payload,
            source,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cancelled(request: u64, transfer: u64) -> [u8; MESSAGE_LEN] {
        Message {
            kind: Kind::Cancelled,
            request,
            transfer,
            arg0: Status::Cancelled as u32,
            arg1: 0,
        }
        .encode()
    }
    #[test]
    fn bidirectional_grant_requires_attestation_instead_of_rejecting_its_direction() {
        let mut client = Client::new(9, gui_protocol_v1::wire::FEATURE_DATA_TRANSFER);
        let message = Message {
            kind: Kind::Grant,
            request: 1,
            transfer: 10,
            arg0: 123,
            arg1: 0,
        };
        assert_eq!(
            client.handle_packet(&message.encode(), None).err(),
            Some(Error(libdunit::EACCES))
        );
    }
    #[test]
    fn remote_request_numbers_cannot_cancel_unrelated_local_sources_or_reads() {
        let mut client = Client::new(9, gui_protocol_v1::wire::FEATURE_DATA_TRANSFER);
        let payload = Payload::text("private clipboard").unwrap();
        let offer = payload.offer(1, Action::Copy).unwrap();
        client.sources.insert(
            7,
            Source {
                kind: Kind::ClipboardOffer,
                payload,
                offer,
                transfer: Some(11),
            },
        );
        client.reads.insert(8, Some(21));
        client.receiving.insert(21, Vec::new());
        let event = client
            .handle_packet(&cancelled(7, 101), None)
            .unwrap()
            .unwrap();
        assert!(!event.source);
        assert!(client.sources.contains_key(&7));
        client.handle_packet(&cancelled(8, 102), None).unwrap();
        assert!(client.reads.contains_key(&8));
        assert!(client.receiving.contains_key(&21));
        let event = client
            .handle_packet(&cancelled(7, 11), None)
            .unwrap()
            .unwrap();
        assert!(event.source);
        assert!(!client.sources.contains_key(&7));
        client.handle_packet(&cancelled(8, 21), None).unwrap();
        assert!(!client.reads.contains_key(&8));
        assert!(!client.receiving.contains_key(&21));
    }
    #[test]
    fn drag_ack_binds_source_identity_before_data_is_requested() {
        let mut client = Client::new(9, gui_protocol_v1::wire::FEATURE_DATA_TRANSFER);
        let payload = Payload::text("selection").unwrap();
        let offer = payload.offer(1, Action::Copy).unwrap();
        client.sources.insert(
            4,
            Source {
                kind: Kind::DragOffer,
                payload,
                offer,
                transfer: None,
            },
        );
        let message = Message {
            kind: Kind::Result,
            request: 4,
            transfer: 51,
            arg0: 0,
            arg1: 0,
        };
        assert!(
            client
                .handle_packet(&message.encode(), None)
                .unwrap()
                .unwrap()
                .source
        );
        assert_eq!(client.sources[&4].transfer, Some(51));
        let event = client
            .handle_packet(&cancelled(4, 51), None)
            .unwrap()
            .unwrap();
        assert!(event.source);
        assert!(client.sources.is_empty());
    }
}
