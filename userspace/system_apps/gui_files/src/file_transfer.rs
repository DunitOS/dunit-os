//! Clipboard/DnD operations retain actual destination authority and acknowledge
//! file moves only after the broker's identity-scoped filesystem operation.
use super::*;
use dunit::transfer::{
    self, Action as TransferAction, FileMove, Format, Kind, Offer, OwnedHandle, Payload, Status,
};
struct Destination {
    directory: OwnedHandle,
    generation: u64,
    epoch: u64,
}
struct Read {
    request: u64,
    id: Option<u64>,
    destination: Destination,
}
struct Operation {
    id: u64,
    payload: Payload,
    directory: OwnedHandle,
    index: usize,
    grant: Option<u64>,
    destination: u32,
    waiting: Option<u64>,
    copy: Option<transfer::file_ops::CopyJob>,
    retired: bool,
}
struct Hover {
    id: u64,
    offer: Offer,
    action: TransferAction,
    index: Option<usize>,
    checked: bool,
    destination: Option<Destination>,
    accepted: bool,
    decided: bool,
}
#[derive(Default)]
pub(super) struct Transfers {
    publication: Option<(u64, bool)>,
    query: Option<(u64, Destination)>,
    read: Option<Read>,
    operation: Option<Operation>,
    hover: Option<Hover>,
    candidate: Option<(i32, i32, u32, bool, usize)>,
    source: Option<u64>,
    registered: Option<Offer>,
    files_target: bool,
    focused: bool,
    epoch: u64,
}
fn formats(preferred: TransferAction) -> Offer {
    Offer {
        formats: alloc::vec![Format::Files],
        actions: 3,
        preferred,
    }
}
fn native(error: transfer::Error) -> dunit::Error {
    dunit::Error(error.0)
}
impl Transfers {
    pub fn start() -> Self {
        let mut state = Self::default();
        state.focused = true;
        state
    }
    pub fn sync(&mut self, window: &mut Window, mode: &Mode, files: &Files) -> dunit::Result<()> {
        self.files_target =
            matches!(mode, Mode::Browse) && files.can_write() && self.operation.is_none();
        let desired = if self.files_target {
            Some(formats(TransferAction::Copy))
        } else {
            None
        };
        if self.registered != desired {
            window.set_transfer_target(desired.clone())?;
            self.registered = desired;
            self.hover = None;
        }
        Ok(())
    }
    pub fn focus(&mut self, focused: bool) {
        if self.focused != focused {
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.focused = focused;
        if !focused {
            self.candidate = None;
        }
    }
    pub fn publish(&mut self, window: &mut Window, files: &mut Files, cut: bool) {
        match files.payload(cut).and_then(|payload| {
            window
                .transfers()
                .map_err(|e| transfer::Error(e.0))?
                .publish_clipboard(
                    payload,
                    if cut { 3 } else { 1 },
                    if cut {
                        TransferAction::Move
                    } else {
                        TransferAction::Copy
                    },
                )
        }) {
            Ok(request) => {
                self.publication = Some((request, cut));
                files.status = "Preparing clipboard".into();
            }
            Err(error) => files.error("Clipboard", error),
        }
    }
    pub fn paste(&mut self, window: &mut Window, files: &mut Files) {
        if self.query.is_some() || self.read.is_some() || self.operation.is_some() {
            files.error("Paste", transfer::Error(dunit::errno::EBUSY));
            return;
        }
        if !files.can_write() {
            files.error("Paste", transfer::Error(dunit::errno::EPERM));
            return;
        }
        let result = files.directory().and_then(|directory| {
            let request = window
                .transfers()
                .map_err(|e| transfer::Error(e.0))?
                .query()?;
            Ok((
                request,
                Destination {
                    directory,
                    generation: files.generation,
                    epoch: self.epoch,
                },
            ))
        });
        match result {
            Ok(query) => self.query = Some(query),
            Err(error) => files.error("Paste destination", error),
        }
    }
    pub fn press(&mut self, files: &Files, index: usize, x: i32, y: i32, was_selected: bool) {
        self.candidate = Some((
            x,
            y,
            dwm_settings::load().transfers.drag_threshold_px,
            was_selected,
            index,
        ));
        if !files.can_export(false) {
            self.candidate = None;
        }
    }
    pub fn motion(&mut self, window: &mut Window, files: &mut Files, x: i32, y: i32) {
        let Some((sx, sy, threshold, _, _)) = self.candidate else {
            return;
        };
        let dx = i64::from(x) - i64::from(sx);
        let dy = i64::from(y) - i64::from(sy);
        if dx * dx + dy * dy < i64::from(threshold).pow(2) {
            return;
        }
        let writable = files.can_export(true);
        let result = files.payload(writable).and_then(|payload| {
            window
                .transfers()
                .map_err(|e| transfer::Error(e.0))?
                .start_drag(
                    payload,
                    if writable { 3 } else { 1 },
                    if writable {
                        TransferAction::Move
                    } else {
                        TransferAction::Copy
                    },
                )
        });
        self.candidate = None;
        match result {
            Ok(request) => self.source = Some(request),
            Err(error) => files.error("Drag", error),
        }
    }
    pub fn release(&mut self) -> Option<(bool, usize)> {
        self.candidate
            .take()
            .map(|(_, _, _, selected, index)| (selected, index))
    }
    pub fn cancel_candidate(&mut self) {
        self.candidate = None;
    }
    fn finish(&mut self, window: &mut Window, status: Status) -> dunit::Result<()> {
        if let Some(operation) = self.operation.take() {
            if !operation.retired {
                window
                    .transfers()?
                    .complete(operation.id, status)
                    .map_err(native)?;
            }
        }
        Ok(())
    }
    fn move_next(&mut self, window: &mut Window) -> dunit::Result<()> {
        let Some(operation) = self.operation.as_mut() else {
            return Ok(());
        };
        if operation.index == operation.payload.files.len() {
            return self.finish(window, Status::Ok);
        }
        let file = &operation.payload.files[operation.index];
        let leaf = file.path.rsplit('/').next().unwrap_or("");
        operation.waiting = Some(
            window
                .transfers()?
                .file_move(
                    operation.id,
                    FileMove {
                        item: operation.index as u32,
                        destination: operation.destination,
                        leaf: String::from(leaf),
                    },
                )
                .map_err(native)?,
        );
        Ok(())
    }
    pub fn copying(&self) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|op| op.grant.is_none() && op.waiting.is_none() && op.destination == 0)
    }
    /// One bounded copy step per event-loop turn, so IPC/input continue to run.
    pub fn service(&mut self, window: &mut Window, files: &mut Files) -> dunit::Result<bool> {
        if !self.copying() {
            return Ok(false);
        }
        let operation = self.operation.as_mut().unwrap();
        if operation.index == operation.payload.files.len() {
            files.status = alloc::format!("Copied {} item(s)", operation.index);
            self.finish(window, Status::Ok)?;
            files.reload();
            return Ok(true);
        }
        if operation.copy.is_none() {
            let file = &operation.payload.files[operation.index];
            let leaf = file.path.rsplit('/').next().unwrap_or("");
            match transfer::file_ops::CopyJob::new(file, &operation.directory, leaf) {
                Ok(job) => operation.copy = Some(job),
                Err(error) => {
                    files.error("Copy", error);
                    self.finish(window, Status::Failed)?;
                    files.reload();
                    return Ok(true);
                }
            }
        }
        match operation.copy.as_mut().unwrap().poll() {
            Ok(false) => Ok(false),
            Ok(true) => {
                operation.copy = None;
                operation.index += 1;
                Ok(false)
            }
            Err(error) => {
                files.status = alloc::format!(
                    "Copy stopped after {} item(s): {}",
                    operation.index,
                    dunit::Error(error.operation.0).name()
                );
                if let Some(cleanup) = error.cleanup {
                    files.error("Copy cleanup", cleanup);
                }
                self.finish(
                    window,
                    if error.operation.0 == dunit::errno::EINTR {
                        Status::Cancelled
                    } else {
                        Status::Failed
                    },
                )?;
                files.reload();
                Ok(true)
            }
        }
    }
    pub fn cancel_operation(
        &mut self,
        window: &mut Window,
        files: &mut Files,
    ) -> dunit::Result<bool> {
        let Some(operation) = self.operation.as_mut() else {
            return Ok(false);
        };
        files.status = alloc::format!(
            "Transfer cancelled after {} completed item(s)",
            operation.index
        );
        if let Some(job) = &mut operation.copy {
            job.request_cancel();
            return Ok(true);
        }
        self.finish(window, Status::Cancelled)?;
        files.reload();
        Ok(true)
    }
    pub fn event(
        &mut self,
        window: &mut Window,
        files: &mut Files,
        event: transfer::Event,
    ) -> dunit::Result<()> {
        let m = event.message;
        match m.kind {
            Kind::Offer if self.query.as_ref().is_some_and(|(r, _)| *r == m.request) => {
                let (_, destination) = self.query.take().unwrap();
                let offer = event.offer.ok_or(dunit::Error(dunit::errno::EINVAL))?;
                if !offer.formats.contains(&Format::Files) {
                    files.error("Paste", transfer::Error(dunit::errno::EOPNOTSUPP));
                    return Ok(());
                }
                if !self.focused
                    || destination.generation != files.generation
                    || destination.epoch != self.epoch
                {
                    return Ok(());
                }
                let request = window
                    .transfers()?
                    .read_clipboard(formats(offer.preferred))
                    .map_err(native)?;
                self.read = Some(Read {
                    request,
                    id: None,
                    destination,
                });
            }
            Kind::Offer => {
                if let Some(read) = &mut self.read {
                    if read.request == m.request {
                        read.id = Some(m.transfer);
                    }
                }
            }
            Kind::DragEnter if self.files_target => {
                if let (Some(offer), Some(action)) = (event.offer, TransferAction::from_u32(m.arg1))
                {
                    self.hover = Some(Hover {
                        id: m.transfer,
                        offer,
                        action,
                        index: None,
                        checked: false,
                        destination: None,
                        accepted: false,
                        decided: false,
                    });
                }
            }
            Kind::DragMotion => {
                if let Some(hover) = self.hover.as_mut().filter(|h| h.id == m.transfer) {
                    let index = match hit_cell(files, m.arg0 as i32, m.arg1 as i32) {
                        Some(Hit::Item(index)) if files.items[index].is_dir() => Some(index),
                        _ => None,
                    };
                    if !hover.checked
                        || hover.index != index
                        || hover
                            .destination
                            .as_ref()
                            .is_some_and(|d| d.generation != files.generation)
                    {
                        hover.index = index;
                        hover.checked = true;
                        hover.destination =
                            files.destination(index).ok().map(|directory| Destination {
                                directory,
                                generation: files.generation,
                                epoch: self.epoch,
                            });
                    }
                    let selected =
                        transfer::negotiate(&hover.offer, &formats(hover.action), hover.action);
                    let accepting = hover.destination.is_some() && selected.is_some();
                    if accepting && !hover.accepted {
                        window
                            .transfers()?
                            .drag_accept(hover.id, selected.unwrap().format, hover.action)
                            .map_err(native)?;
                    } else if !accepting && (hover.accepted || !hover.decided) {
                        window.transfers()?.drag_reject(hover.id).map_err(native)?;
                    }
                    hover.accepted = accepting;
                    hover.decided = true;
                }
            }
            Kind::DragDrop => {
                if let Some(hover) = self.hover.as_mut().filter(|h| h.id == m.transfer) {
                    if let Some(mut destination) = hover
                        .destination
                        .take()
                        .filter(|d| d.generation == files.generation)
                    {
                        // The compositor focuses an accepted target before Drop.
                        destination.epoch = self.epoch;
                        self.read = Some(Read {
                            request: m.request,
                            id: Some(m.transfer),
                            destination,
                        });
                    } else {
                        window
                            .transfers()?
                            .complete(m.transfer, Status::Cancelled)
                            .map_err(native)?;
                    }
                }
            }
            Kind::Data
                if self.read.as_ref().is_some_and(|read| {
                    read.request == m.request && read.id == Some(m.transfer)
                }) =>
            {
                let read = self.read.take().unwrap();
                if !self.focused
                    || files.generation != read.destination.generation
                    || self.epoch != read.destination.epoch
                {
                    window
                        .transfers()?
                        .complete(m.transfer, Status::Cancelled)
                        .map_err(native)?;
                    return Ok(());
                }
                let Some(payload) = event.payload.filter(|p| !p.files.is_empty()) else {
                    window
                        .transfers()?
                        .complete(m.transfer, Status::Unsupported)
                        .map_err(native)?;
                    return Ok(());
                };
                let Some(action) = TransferAction::from_u32(m.arg1) else {
                    return Err(dunit::Error(dunit::errno::EINVAL));
                };
                if self.operation.is_some() {
                    window
                        .transfers()?
                        .complete(m.transfer, Status::Busy)
                        .map_err(native)?;
                    return Ok(());
                }
                self.operation = Some(Operation {
                    id: m.transfer,
                    payload,
                    directory: read.destination.directory,
                    index: 0,
                    grant: None,
                    destination: 0,
                    waiting: None,
                    copy: None,
                    retired: false,
                });
                if action == TransferAction::Move {
                    let operation = self.operation.as_mut().unwrap();
                    operation.grant = Some(
                        window
                            .transfers()?
                            .grant_destination(operation.id, &operation.directory)
                            .map_err(native)?,
                    );
                } else if action == TransferAction::Copy {
                    files.status = "Copying selection; Esc cancels".into();
                } else {
                    self.finish(window, Status::Unsupported)?;
                }
            }
            Kind::GrantAck => {
                if let Some(operation) = self
                    .operation
                    .as_mut()
                    .filter(|op| op.grant == Some(m.request) && op.id == m.transfer)
                {
                    operation.grant = None;
                    if m.arg1 == Status::Ok as u32 {
                        operation.destination = m.arg0;
                        self.move_next(window)?;
                    } else {
                        self.finish(window, Status::Failed)?;
                    }
                }
            }
            Kind::Result => {
                if self
                    .publication
                    .is_some_and(|(request, _)| request == m.request)
                {
                    let (_, cut) = self.publication.take().unwrap();
                    files.status = if m.arg0 == Status::Ok as u32 {
                        if cut {
                            "Cut ready; files move only on paste".into()
                        } else {
                            "Selection copied".into()
                        }
                    } else {
                        alloc::format!("Clipboard rejected ({})", m.arg0)
                    };
                    if m.arg0 == Status::Ok as u32 {
                        log("gui_files: clipboard published");
                    }
                }
                if self.source == Some(m.request) && m.arg0 != Status::Ok as u32 {
                    self.source = None;
                    files.status = alloc::format!("Drag rejected ({})", m.arg0);
                }
                if self.read.as_ref().is_some_and(|r| r.request == m.request)
                    && m.arg0 != Status::Ok as u32
                {
                    self.read = None;
                    files.status = alloc::format!("Paste rejected ({})", m.arg0);
                }
                if self.query.as_ref().is_some_and(|(r, _)| *r == m.request)
                    && m.arg0 != Status::Ok as u32
                {
                    self.query = None;
                    files.status = "Clipboard is empty or unavailable".into();
                }
                if let Some(operation) = self
                    .operation
                    .as_mut()
                    .filter(|op| op.waiting == Some(m.request) && op.id == m.transfer)
                {
                    operation.waiting = None;
                    if m.arg0 == Status::Ok as u32 {
                        operation.index += 1;
                        files.status = alloc::format!("Moved {} item(s)", operation.index);
                        self.move_next(window)?;
                    } else {
                        files.status = alloc::format!(
                            "Move stopped after {} item(s), status {}",
                            operation.index,
                            m.arg0
                        );
                        self.finish(window, Status::Failed)?;
                    }
                    files.reload();
                }
            }
            Kind::Cancelled | Kind::Finished => {
                if self
                    .read
                    .as_ref()
                    .is_some_and(|read| read.id == Some(m.transfer))
                {
                    self.read = None;
                }
                if event.source && self.publication.is_some_and(|(r, _)| r == m.request) {
                    self.publication = None;
                    files.status = "Clipboard publication cancelled".into();
                }
                if event.source && self.source == Some(m.request) {
                    self.source = None;
                    files.reload();
                }
                if self.hover.as_ref().is_some_and(|h| h.id == m.transfer) {
                    self.hover = None;
                }
                if self
                    .operation
                    .as_ref()
                    .is_some_and(|op| op.id == m.transfer)
                {
                    let operation = self.operation.as_mut().unwrap();
                    if let Some(job) = &mut operation.copy {
                        job.request_cancel();
                        operation.retired = true;
                    } else {
                        self.operation = None;
                        files.reload();
                    }
                }
            }
            Kind::DragLeave => {
                self.hover = None;
            }
            _ => {}
        }
        Ok(())
    }
}
