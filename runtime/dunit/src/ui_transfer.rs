//! Clipboard defaults run after the one capture/bubble pass. Deferred Cut and
//! Paste use exact controlled-editor snapshots, never whichever field is focused
//! when the IPC reply happens to arrive.
use super::*;
use dunit_transfer::{Action, Kind, Offer, Payload, Status};
#[derive(Default)]
pub(super) struct Transfers {
    clipboard: crate::text_clipboard::Session<InputContext>,
    drop: Option<TextDrop>,
    registered: Option<Offer>,
    external: Option<Offer>,
    candidate: Option<TextDrag>,
    source: Option<u64>,
}
struct TextDrag {
    context: InputContext,
    point: (i32, i32),
    index: usize,
    threshold: u32,
}
struct TextDrop {
    id: u64,
    offer: Offer,
    action: Action,
    point: (i32, i32),
    key: Option<alloc::string::String>,
    accepted: bool,
    decided: bool,
    context: Option<InputContext>,
}
impl crate::text_clipboard::Context for InputContext {
    fn selected_text(&self) -> Option<&str> {
        self.export_text()
    }
    fn writable(&self) -> bool {
        self.writable()
    }
}
fn preferences() -> Offer {
    crate::text_clipboard::preferences()
}
fn native_transfer(error: dunit_transfer::Error) -> Error {
    Error(error.0)
}
impl<C: Component> ComponentWindow<C> {
    pub fn transfers(&mut self) -> NativeResult<&mut dunit_transfer::Client> {
        self.window.transfers()
    }
    /// Register an application-specific drop target beneath SDK text controls.
    /// A writable Input has priority; its removal restores this explicit offer.
    pub fn set_transfer_target(&mut self, offer: Option<Offer>) -> NativeResult<()> {
        self.transfers.external = offer;
        self.sync_transfer_target()
    }
    pub(super) fn sync_transfer_target(&mut self) -> NativeResult<()> {
        let desired = if self.application.has_text_drop_target() {
            Some(preferences())
        } else {
            self.transfers.external.clone()
        };
        if desired != self.transfers.registered {
            let client = self.window.transfers()?;
            if let Some(offer) = desired.as_ref() {
                client.register_target(offer.clone())
            } else {
                client.unregister_target()
            }
            .map_err(native_transfer)?;
            self.transfers.registered = desired;
        }
        Ok(())
    }
    pub(super) fn transfer_key(&mut self, input: &crate::window::Input) -> NativeResult<bool> {
        if input.kind != IN_KEY
            || input.y as u8 & libdunit::KEYMOD_CTRL == 0
            || input.y as u8 & !(libdunit::KEYMOD_CTRL | libdunit::KEYMOD_SHIFT) != 0
        {
            return Ok(false);
        }
        if !matches!(input.x, 0x2e | 0x2d | 0x2f) {
            return Ok(false);
        }
        let Some(context) = self.application.input_context() else {
            if input.x == 0x2e {
                if let Some(text) = self.application.collection_clipboard_text() {
                    self.window
                        .transfers()?
                        .publish_clipboard(
                            Payload::text(&text).map_err(native_transfer)?,
                            1,
                            Action::Copy,
                        )
                        .map_err(native_transfer)?;
                    return Ok(true);
                }
            }
            return Ok(false);
        };
        let command = crate::text_clipboard::Command::from_scancode(input.x)
            .ok_or(Error(libdunit::EINVAL))?;
        self.transfers
            .clipboard
            .command(self.window.transfers()?, command, context)?;
        Ok(true)
    }
    pub(super) fn cancel_transfer_candidate(&mut self) {
        self.transfers.candidate = None;
    }
    pub(super) fn transfer_pointer(&mut self, input: &crate::window::Input) -> NativeResult<bool> {
        if matches!(input.kind, IN_UP | IN_DOWN) && input.pointer_button() != 0 {
            return Ok(false);
        }
        if input.kind == IN_LEAVE {
            self.transfers.candidate = None;
            return Ok(false);
        }
        if input.kind == IN_DOWN {
            self.transfers.candidate = None;
            let Some(context) = self.application.input_context() else {
                return Ok(false);
            };
            if context.export_text().is_none()
                || input.pointer_modifiers() & libdunit::KEYMOD_SHIFT != 0
            {
                return Ok(false);
            }
            if self.renderer.hit_key(input.x as f32, input.y as f32) != Some(context.key.as_str()) {
                return Ok(false);
            }
            let Some(position) =
                self.renderer
                    .input_position_at(&context.key, input.x as f32, input.y as f32)
            else {
                return Ok(false);
            };
            if !context.editor().selection().contains(&position.index) {
                return Ok(false);
            }
            let sample = TextClickSample {
                time_ms: self
                    .motion_clock
                    .elapsed()?
                    .as_millis()
                    .min(u64::MAX as u128) as u64,
                x: input.x as f32,
                y: input.y as f32,
                config: self
                    .renderer
                    .input_click_config(&context.key)
                    .unwrap_or_default(),
            };
            if self
                .text_clicks
                .continues_click(&context.key, context.editor().text(), sample)
            {
                return Ok(false);
            }
            self.transfers.candidate = Some(TextDrag {
                context,
                point: (input.x, input.y),
                index: position.index,
                threshold: dwm_settings::load().transfers.drag_threshold_px,
            });
            return Ok(true);
        }
        if input.kind == IN_MOVE {
            if let Some(candidate) = self.transfers.candidate.as_ref() {
                if !self.application.input_context_current(&candidate.context) {
                    self.transfers.candidate = None;
                    return Ok(false);
                }
                let dx = i64::from(input.x) - i64::from(candidate.point.0);
                let dy = i64::from(input.y) - i64::from(candidate.point.1);
                if dx * dx + dy * dy >= i64::from(candidate.threshold).pow(2) {
                    let payload = Payload::text(candidate.context.export_text().unwrap_or(""))
                        .map_err(native_transfer)?;
                    let request = self
                        .window
                        .transfers()?
                        .start_drag(payload, Action::Copy as u32, Action::Copy)
                        .map_err(native_transfer)?;
                    self.transfers.source = Some(request);
                    self.transfers.candidate = None;
                    self.text_clicks.cancel();
                }
                return Ok(true);
            }
            if self.transfers.source.is_some() {
                return Ok(true);
            }
        }
        if input.kind == IN_UP {
            if let Some(candidate) = self.transfers.candidate.take() {
                if self
                    .application
                    .place_at_context(&candidate.context, candidate.index)
                    .map_err(native)?
                {
                    self.redraw()?;
                }
                return Ok(true);
            }
            if self.transfers.source.is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(super) fn transfer_event(&mut self, event: &mut dunit_transfer::Event) -> NativeResult<()> {
        let mut changed =
            self.transfers
                .clipboard
                .event(self.window.transfers()?, event, |context, text| {
                    self.application
                        .insert_at_context(context, text)
                        .map_err(native)
                })?;
        let m = event.message;
        if self.transfers.external.is_some()
            && !self.application.has_text_drop_target()
            && !event.source
        {
            return Ok(());
        }
        match m.kind {
            Kind::Result => {
                if self.transfers.source == Some(m.request) && m.arg0 != Status::Ok as u32 {
                    self.transfers.source = None;
                }
            }
            Kind::DragEnter => {
                if let (Some(offer), Some(action)) =
                    (event.offer.as_ref(), Action::from_u32(m.arg1))
                {
                    self.transfers.drop = Some(TextDrop {
                        id: m.transfer,
                        offer: offer.clone(),
                        action,
                        point: (0, 0),
                        key: None,
                        accepted: false,
                        decided: false,
                        context: None,
                    });
                }
            }
            Kind::DragMotion => {
                if let Some(drop) = self.transfers.drop.as_mut().filter(|d| d.id == m.transfer) {
                    drop.point = (m.arg0 as i32, m.arg1 as i32);
                    drop.key = self
                        .renderer
                        .hit_key(drop.point.0 as f32, drop.point.1 as f32)
                        .filter(|key| self.application.text_drop_target(key))
                        .map(alloc::string::String::from);
                    let selection =
                        dunit_transfer::negotiate(&drop.offer, &preferences(), drop.action);
                    let accepting = drop.key.is_some() && selection.is_some();
                    if accepting && !drop.accepted {
                        self.window
                            .transfers()?
                            .drag_accept(drop.id, selection.unwrap().format, drop.action)
                            .map_err(native_transfer)?;
                    } else if !accepting && (drop.accepted || !drop.decided) {
                        self.window
                            .transfers()?
                            .drag_reject(drop.id)
                            .map_err(native_transfer)?;
                    }
                    drop.accepted = accepting;
                    drop.decided = true;
                }
            }
            Kind::DragDrop => {
                if let Some(drop) = self.transfers.drop.as_mut().filter(|d| d.id == m.transfer) {
                    drop.context = drop.key.as_ref().and_then(|key| {
                        self.renderer
                            .input_position_at(key, drop.point.0 as f32, drop.point.1 as f32)
                            .and_then(|p| self.application.text_drop_context(key, p.index))
                    });
                }
            }
            Kind::Data => {
                let context = if self
                    .transfers
                    .drop
                    .as_ref()
                    .is_some_and(|d| d.id == m.transfer)
                {
                    self.transfers.drop.take().and_then(|d| d.context)
                } else {
                    if changed {
                        self.caret_frame = Some(crate::time::Instant::now()?);
                        self.redraw()?;
                    }
                    return Ok(());
                };
                let status = if let Some(context) = context {
                    if let Some(text) = event.payload.as_ref().and_then(|p| p.text_value()) {
                        changed = self
                            .application
                            .insert_at_context(&context, text)
                            .map_err(native)?;
                        if changed {
                            Status::Ok
                        } else {
                            Status::Cancelled
                        }
                    } else {
                        Status::Unsupported
                    }
                } else {
                    Status::Cancelled
                };
                self.window
                    .transfers()?
                    .complete(m.transfer, status)
                    .map_err(native_transfer)?;
            }
            Kind::Cancelled | Kind::Finished | Kind::DragLeave => {
                if event.source
                    && matches!(m.kind, Kind::Cancelled | Kind::Finished)
                    && self.transfers.source == Some(m.request)
                {
                    self.transfers.source = None;
                }
                if self
                    .transfers
                    .drop
                    .as_ref()
                    .is_some_and(|d| d.id == m.transfer)
                {
                    self.transfers.drop = None;
                }
            }
            _ => {}
        }
        if changed {
            self.caret_frame = Some(crate::time::Instant::now()?);
            self.redraw()?;
        }
        Ok(())
    }
}
