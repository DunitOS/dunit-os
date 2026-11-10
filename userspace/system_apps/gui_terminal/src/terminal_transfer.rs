//! Text selection export and path/text insertion over the window's transfer pump.
use super::{terminal_model::Term, PAD_X, PAD_Y, ROW_PX};
use alloc::string::String;
use dunit::pty::PendingInput;
use dunit::{
    transfer::{self, *},
    window::{Input, Window},
};
#[derive(Default)]
pub(super) struct Transfers {
    candidate: Option<(i32, i32, u32, String)>,
    source: Option<u64>,
    paste: Option<(u64, Option<u64>, u64)>,
    drop: Option<u64>,
    epoch: u64,
    focused: bool,
    writable: bool,
}
fn preferences() -> Offer {
    Offer {
        formats: alloc::vec![Format::Custom(String::from(URI_LIST_MIME)), Format::Text],
        actions: 1,
        preferred: Action::Copy,
    }
}
fn point(term: &Term, x: i32, y: i32) -> (usize, usize) {
    (
        ((y - PAD_Y).max(0) / ROW_PX) as usize,
        ((x - PAD_X).max(0) / term.cell_w.max(1)) as usize,
    )
}
impl Transfers {
    pub fn start(window: &mut Window) -> Self {
        let mut value = Self::default();
        value.focused = true;
        value.writable = true;
        if let Err(e) = window.transfers().and_then(|t| {
            t.register_target(preferences())
                .map_err(|e| dunit::Error(e.0))
        }) {
            super::fatal("transfer registration", e.0);
        }
        value
    }
    pub fn read_only(&mut self, window: &mut Window) -> dunit::Result<()> {
        if self.writable {
            self.writable = false;
            window.transfers()?.unregister_target().map_err(native)?;
        }
        Ok(())
    }
    pub fn resized(&mut self) {
        self.candidate = None;
    }
    pub fn focus(&mut self, focused: bool, term: &mut Term) {
        if self.focused != focused {
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.focused = focused;
        if !focused {
            self.candidate = None;
            term.select_end();
        }
    }
    pub fn key(&mut self, window: &mut Window, term: &Term, input: &Input) -> dunit::Result<bool> {
        let mods = input.y as u8;
        if mods != dunit::window::KEYMOD_CTRL | dunit::window::KEYMOD_SHIFT {
            return Ok(false);
        }
        if input.x == 0x2e {
            if let Some(text) = term.selected_text() {
                window
                    .transfers()?
                    .publish_clipboard(Payload::text(&text).map_err(native)?, 1, Action::Copy)
                    .map_err(native)?;
            }
            return Ok(true);
        }
        if input.x == 0x2f {
            if !self.writable {
                return Ok(true);
            }
            let request = window
                .transfers()?
                .read_clipboard(preferences())
                .map_err(native)?;
            self.paste = Some((request, None, self.epoch));
            return Ok(true);
        }
        Ok(false)
    }
    pub fn pointer(
        &mut self,
        window: &mut Window,
        term: &mut Term,
        input: &Input,
    ) -> dunit::Result<()> {
        use dunit::window::*;
        if matches!(input.kind, IN_UP | IN_DOWN) && input.pointer_button() != 0 {
            return Ok(());
        }
        let (row, col) = point(term, input.x, input.y);
        match input.kind {
            IN_DOWN => {
                self.candidate = None;
                let at = term.point(row, col);
                if term.selected(at.row, at.col) {
                    if let Some(text) = term.selected_text() {
                        self.candidate = Some((
                            input.x,
                            input.y,
                            dwm_settings::load().transfers.drag_threshold_px,
                            text,
                        ));
                    }
                } else {
                    term.select_start(row, col);
                }
            }
            IN_MOVE => {
                if let Some((x, y, threshold, text)) = &self.candidate {
                    let dx = i64::from(input.x) - i64::from(*x);
                    let dy = i64::from(input.y) - i64::from(*y);
                    if dx * dx + dy * dy >= i64::from(*threshold).pow(2) {
                        self.source = Some(
                            window
                                .transfers()?
                                .start_drag(Payload::text(text).map_err(native)?, 1, Action::Copy)
                                .map_err(native)?,
                        );
                        self.candidate = None;
                        term.select_end();
                    }
                } else {
                    term.select_extend(row, col);
                }
            }
            IN_UP => {
                if self.candidate.take().is_some() {
                    term.select_start(row, col);
                }
                term.select_end();
            }
            IN_LEAVE => {
                self.candidate = None;
                term.select_end();
            }
            _ => {}
        }
        Ok(())
    }
    pub fn event(
        &mut self,
        window: &mut Window,
        event: transfer::Event,
        input: &mut PendingInput,
    ) -> dunit::Result<()> {
        let m = event.message;
        match m.kind {
            Kind::Offer => {
                if let Some((request, id, _)) = &mut self.paste {
                    if *request == m.request {
                        *id = Some(m.transfer);
                    }
                }
            }
            Kind::DragEnter => {
                let selected = event
                    .offer
                    .as_ref()
                    .and_then(|offer| negotiate(offer, &preferences(), Action::Copy));
                if self.writable && m.arg1 == Action::Copy as u32 {
                    if let Some(selected) = selected {
                        window
                            .transfers()?
                            .drag_accept(m.transfer, selected.format, Action::Copy)
                            .map_err(native)?;
                        self.drop = Some(m.transfer);
                    }
                }
            }
            Kind::Data => {
                let paste = self
                    .paste
                    .filter(|(request, id, _)| *request == m.request && *id == Some(m.transfer));
                let drop = self.drop == Some(m.transfer);
                if paste.is_none() && !drop {
                    return Ok(());
                }
                let allowed = self.focused
                    && self.writable
                    && paste.is_none_or(|(_, _, epoch)| epoch == self.epoch);
                let bytes = event.payload.as_ref().and_then(|payload| {
                    if let Some(uri) = payload
                        .bundle
                        .get(&Format::Custom(String::from(URI_LIST_MIME)))
                    {
                        let paths = PathList::decode(uri).ok()?;
                        dunit_shell::quote_paths(paths.paths.iter().map(String::as_str))
                            .ok()
                            .map(String::into_bytes)
                    } else {
                        payload.text_value().map(|s| s.as_bytes().to_vec())
                    }
                });
                let accepted = allowed && bytes.is_some_and(|bytes| input.push(&bytes));
                window
                    .transfers()?
                    .complete(
                        m.transfer,
                        if accepted {
                            Status::Ok
                        } else {
                            Status::Cancelled
                        },
                    )
                    .map_err(native)?;
                if paste.is_some() {
                    self.paste = None;
                }
                if drop {
                    self.drop = None;
                }
            }
            Kind::Result => {
                if m.arg0 != Status::Ok as u32 {
                    if self.source == Some(m.request) {
                        self.source = None;
                    }
                    if self.paste.is_some_and(|(r, _, _)| r == m.request) {
                        self.paste = None;
                    }
                }
            }
            Kind::Cancelled | Kind::Finished | Kind::DragLeave => {
                if self.drop == Some(m.transfer) {
                    self.drop = None;
                }
                if event.source
                    && matches!(m.kind, Kind::Cancelled | Kind::Finished)
                    && self.source == Some(m.request)
                {
                    self.source = None;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
fn native(error: transfer::Error) -> dunit::Error {
    dunit::Error(error.0)
}
