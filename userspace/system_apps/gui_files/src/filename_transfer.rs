//! Text-field drag policy for New Folder/Rename; the file-operation controller
//! never sees a text payload as file authority.
use super::*;
use dunit::transfer::{self, Action as TransferAction, Kind, Payload, Status};
struct Drop {
    id: u64,
    offer: transfer::Offer,
    point: (i32, i32),
    accepted: bool,
    decided: bool,
    context: Option<filename_editor::Snapshot>,
}
struct Candidate {
    context: filename_editor::Snapshot,
    point: (i32, i32),
    threshold: u32,
    revision: u64,
}
#[derive(Default)]
pub(super) struct Transfers {
    drop: Option<Drop>,
    candidate: Option<Candidate>,
    selecting: bool,
    source: Option<u64>,
}
fn native(error: transfer::Error) -> dunit::Error {
    dunit::Error(error.0)
}
fn field(files: &Files, x: i32, y: i32) -> bool {
    let left = (files.w - DLG_W) / 2 + 14;
    let top = (files.h - DLG_H) / 2 + 38;
    x >= left && x < left + DLG_W - 28 && y >= top && y < top + 22
}
fn index(editor: &Editor, files: &Files, font: &Font, x: i32) -> usize {
    editor.position(font, (x - ((files.w - DLG_W) / 2 + 20)) as f32)
}
impl Transfers {
    pub fn pointer(
        &mut self,
        window: &mut Window,
        mode: &mut Mode,
        files: &Files,
        font: &Font,
        input: &dunit::window::Input,
        revision: u64,
    ) -> dunit::Result<bool> {
        if matches!(input.kind, IN_FOCUS_OUT | IN_LEAVE) {
            self.candidate = None;
            self.selecting = false;
            return Ok(false);
        }
        let Mode::Text { editor, .. } = mode else {
            self.candidate = None;
            self.selecting = false;
            return Ok(false);
        };
        if input.kind == IN_DOWN && input.pointer_button() == 0 && field(files, input.x, input.y) {
            let position = index(editor, files, font, input.x);
            if editor.selection().contains(&position) && input.pointer_modifiers() == 0 {
                self.candidate = Some(Candidate {
                    context: editor.snapshot(revision.wrapping_add(1)),
                    point: (input.x, input.y),
                    threshold: dwm_settings::load().transfers.drag_threshold_px,
                    revision: revision.wrapping_add(1),
                });
            } else {
                editor.place(
                    position,
                    input.pointer_modifiers() & libdunit::KEYMOD_SHIFT != 0,
                );
                self.selecting = true;
            }
            return Ok(true);
        }
        if input.kind == IN_MOVE {
            if let Some(candidate) = &self.candidate {
                if !candidate.context.current(editor, candidate.revision)
                    || revision != candidate.revision
                {
                    self.candidate = None;
                    return Ok(false);
                }
                let dx = i64::from(input.x) - i64::from(candidate.point.0);
                let dy = i64::from(input.y) - i64::from(candidate.point.1);
                if dx * dx + dy * dy >= i64::from(candidate.threshold).pow(2) {
                    use dunit::text_clipboard::Context;
                    let payload = Payload::text(candidate.context.selected_text().unwrap_or(""))
                        .map_err(native)?;
                    self.source = Some(
                        window
                            .transfers()?
                            .start_drag(payload, 1, TransferAction::Copy)
                            .map_err(native)?,
                    );
                    self.candidate = None;
                }
                return Ok(false);
            }
            if self.selecting {
                let position = index(editor, files, font, input.x);
                return Ok(editor.place(position, true));
            }
        }
        if input.kind == IN_UP && input.pointer_button() == 0 {
            self.selecting = false;
            if self.candidate.take().is_some() {
                let position = index(editor, files, font, input.x);
                return Ok(editor.place(position, false));
            }
        }
        Ok(false)
    }
    pub fn event(
        &mut self,
        window: &mut Window,
        mode: &mut Mode,
        revision: u64,
        files: &Files,
        font: &Font,
        event: &transfer::Event,
    ) -> dunit::Result<bool> {
        let m = event.message;
        if event.source
            && matches!(m.kind, Kind::Finished | Kind::Cancelled)
            && self.source == Some(m.request)
        {
            self.source = None;
        }
        if m.kind == Kind::Result && self.source == Some(m.request) && m.arg0 != Status::Ok as u32 {
            self.source = None;
        }
        match m.kind {
            Kind::DragEnter if matches!(mode, Mode::Text { .. }) => {
                if let Some(offer) = &event.offer {
                    self.drop = Some(Drop {
                        id: m.transfer,
                        offer: offer.clone(),
                        point: (0, 0),
                        accepted: false,
                        decided: false,
                        context: None,
                    });
                }
            }
            Kind::DragMotion => {
                if let Some(drop) = self.drop.as_mut().filter(|d| d.id == m.transfer) {
                    drop.point = (m.arg0 as i32, m.arg1 as i32);
                    let selected = transfer::negotiate(
                        &drop.offer,
                        &dunit::text_clipboard::preferences(),
                        TransferAction::Copy,
                    );
                    let accepted = matches!(mode, Mode::Text { .. })
                        && field(files, drop.point.0, drop.point.1)
                        && selected.is_some();
                    if accepted && !drop.accepted {
                        window
                            .transfers()?
                            .drag_accept(drop.id, selected.unwrap().format, TransferAction::Copy)
                            .map_err(native)?;
                    } else if !accepted && (drop.accepted || !drop.decided) {
                        window.transfers()?.drag_reject(drop.id).map_err(native)?;
                    }
                    drop.accepted = accepted;
                    drop.decided = true;
                }
            }
            Kind::DragDrop => {
                if let Some(drop) = self.drop.as_mut().filter(|d| d.id == m.transfer) {
                    if let Mode::Text { editor, .. } = mode {
                        let position = index(editor, files, font, drop.point.0);
                        editor.place(position, false);
                        drop.context = Some(editor.snapshot(revision));
                        return Ok(true);
                    }
                }
            }
            Kind::Data if self.drop.as_ref().is_some_and(|d| d.id == m.transfer) => {
                let drop = self.drop.take().unwrap();
                let accepted = if let Mode::Text { editor, .. } = mode {
                    drop.context
                        .as_ref()
                        .filter(|context| context.current(editor, revision))
                        .zip(
                            event
                                .payload
                                .as_ref()
                                .and_then(|payload| payload.text_value()),
                        )
                        .is_some_and(|(_, text)| editor.insert(text))
                } else {
                    false
                };
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
                return Ok(accepted);
            }
            Kind::Finished | Kind::Cancelled | Kind::DragLeave => {
                if self.drop.as_ref().is_some_and(|d| d.id == m.transfer) {
                    self.drop = None;
                }
            }
            _ => {}
        }
        Ok(false)
    }
}
