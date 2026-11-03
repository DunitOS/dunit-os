//! System text clipboard and typed drop for the Settings draft fields. Replies
//! bind to the precise editor snapshot; they never silently switch fields.
use super::*;
use dunit::{
    text_clipboard::{self, Context, Session},
    transfer::{self, Action as TransferAction, Kind, Status},
    ui::TextEditor,
};
#[derive(Clone)]
struct Snapshot {
    page: usize,
    profile: bool,
    index: usize,
    revision: u64,
    editor: TextEditor,
}
impl Context for Snapshot {
    fn selected_text(&self) -> Option<&str> {
        let range = self.editor.selection();
        if range.is_empty() {
            None
        } else {
            Some(&self.editor.text()[range])
        }
    }
    fn writable(&self) -> bool {
        true
    }
}
fn active(app: &App) -> Option<(&Editor, bool)> {
    if app.page == 12 && app.profile_focused {
        Some((&app.profile_path, true))
    } else if app.editor_section().is_some() {
        app.editor.as_ref().map(|editor| (editor, false))
    } else {
        None
    }
}
fn snapshot(app: &App, revision: u64) -> Option<Snapshot> {
    let (editor, profile) = active(app)?;
    Some(Snapshot {
        page: app.page,
        profile,
        index: editor.selected?,
        revision,
        editor: editor.input.clone(),
    })
}
fn apply(app: &mut App, revision: u64, context: &Snapshot, text: &str) -> bool {
    if context.page != app.page || context.revision != revision {
        return false;
    }
    let Some((current, profile)) = active(app) else {
        return false;
    };
    if context.profile != profile
        || current.selected != Some(context.index)
        || current.input != context.editor
    {
        return false;
    }
    let editor = if profile {
        &mut app.profile_path
    } else {
        app.editor.as_mut().unwrap()
    };
    let changed = editor.insert(text);
    if changed {
        app.document_error.clear();
        app.error = false;
        app.status = "Draft edited; apply explicitly to validate and save.";
    }
    changed
}
fn native(error: transfer::Error) -> dunit::Error {
    dunit::Error(error.0)
}
struct Candidate {
    context: Snapshot,
    point: (i32, i32),
    threshold: u32,
}
struct Drop {
    id: u64,
    offer: transfer::Offer,
    point: (i32, i32),
    accepting: bool,
    decided: bool,
    context: Option<Snapshot>,
}
pub(super) struct Transfers {
    clipboard: Session<Snapshot>,
    revision: u64,
    focused: bool,
    registered: bool,
    drop: Option<Drop>,
    candidate: Option<Candidate>,
    source: Option<u64>,
}
impl Default for Transfers {
    fn default() -> Self {
        Self {
            clipboard: Session::default(),
            revision: 0,
            focused: true,
            registered: false,
            drop: None,
            candidate: None,
            source: None,
        }
    }
}
fn target(app: &App, spots: &[Hotspot], point: (i32, i32)) -> Option<Action> {
    spots
        .iter()
        .find(|spot| spot.hit(point.0, point.1))
        .and_then(|spot| match spot.act {
            Action::EditLine(index) if app.editor_section().is_some() => {
                Some(Action::EditLine(index))
            }
            Action::FocusProfilePath if app.page == 12 => Some(Action::FocusProfilePath),
            _ => None,
        })
}
fn place(app: &mut App, index: usize) {
    if let Some((_, profile)) = active(app) {
        let editor = if profile {
            &mut app.profile_path
        } else {
            app.editor.as_mut().unwrap()
        };
        editor.input.place(index, false);
        editor.cursor = editor.input.cursor();
    }
}
fn caret_at(app: &App, spots: &[Hotspot], point: (i32, i32), font: &Font) -> Option<usize> {
    let spot = spots.iter().find(|spot| spot.hit(point.0, point.1))?;
    let (text, cursor, selected, limit, px, padding) = match spot.act {
        Action::FocusProfilePath => (
            app.profile_path.lines.first()?.as_str(),
            app.profile_path.cursor,
            app.profile_focused,
            42,
            12.0,
            4,
        ),
        Action::EditLine(index) => {
            let editor = app.editor.as_ref()?;
            (
                editor.lines.get(index)?.as_str(),
                editor.cursor,
                editor.selected == Some(index),
                100,
                13.0,
                0,
            )
        }
        _ => return None,
    };
    let start = if selected {
        text[..cursor.min(text.len())]
            .char_indices()
            .rev()
            .nth(limit)
            .map_or(0, |p| p.0)
    } else {
        0
    };
    let x = (point.0 - spot.x - padding) as f32;
    let mut left = 0.0;
    let mut previous = start;
    for (index, ch) in text[start..].char_indices().take(120) {
        let end = start + index + ch.len_utf8();
        let right = font.layout_line(&text[start..end], px).1;
        if x < (left + right) * 0.5 {
            return Some(previous);
        }
        previous = end;
        left = right;
    }
    Some(previous)
}
impl Transfers {
    pub fn register_opener(&mut self, window: &mut Window) -> dunit::Result<()> {
        window
            .transfers()?
            .register_opener(transfer::Offer {
                formats: alloc::vec![transfer::Format::Files],
                actions: 1,
                preferred: TransferAction::Copy,
            })
            .map_err(native)?;
        Ok(())
    }
    pub fn invalidate(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn focus(&mut self, focused: bool) {
        if !focused {
            self.candidate = None;
        }
        if self.focused != focused {
            self.invalidate();
            self.focused = focused;
        }
    }
    pub fn sync(&mut self, window: &mut Window, app: &App) -> dunit::Result<()> {
        let enabled = (app.page == 12) || (app.editor_section().is_some() && app.editor.is_some());
        if self.registered != enabled {
            if enabled {
                window
                    .transfers()?
                    .register_target(text_clipboard::preferences())
            } else {
                window.transfers()?.unregister_target()
            }
            .map_err(native)?;
            self.registered = enabled;
        }
        Ok(())
    }
    pub fn key(
        &mut self,
        window: &mut Window,
        app: &App,
        input: &dunit::window::Input,
    ) -> dunit::Result<bool> {
        if input.y as u8 & libdunit::KEYMOD_CTRL == 0
            || input.y as u8 & !(libdunit::KEYMOD_CTRL | libdunit::KEYMOD_SHIFT) != 0
        {
            return Ok(false);
        }
        let Some(command) = text_clipboard::Command::from_scancode(input.x) else {
            return Ok(false);
        };
        let Some(context) = snapshot(app, self.revision) else {
            return Ok(false);
        };
        self.clipboard
            .command(window.transfers()?, command, context)?;
        Ok(true)
    }
    pub fn pointer(
        &mut self,
        window: &mut Window,
        app: &mut App,
        spots: &[Hotspot],
        font: &Font,
        input: &dunit::window::Input,
    ) -> dunit::Result<Option<bool>> {
        use gui_protocol_v1::control::{IN_LEAVE, IN_MOVE, IN_UP};
        if input.kind == IN_LEAVE {
            self.candidate = None;
            return Ok(None);
        }
        if input.kind == IN_DOWN && input.pointer_button() == 0 {
            if let Some(context) = snapshot(app, self.revision) {
                let same =
                    target(app, spots, (input.x, input.y)).is_some_and(|action| match action {
                        Action::EditLine(index) => !context.profile && index == context.index,
                        Action::FocusProfilePath => context.profile,
                        _ => false,
                    });
                if same && input.pointer_modifiers() == 0 && context.selected_text().is_some() {
                    let position = caret_at(app, spots, (input.x, input.y), font)
                        .unwrap_or(context.editor.text().len());
                    if context.editor.selection().contains(&position) {
                        self.candidate = Some(Candidate {
                            context,
                            point: (input.x, input.y),
                            threshold: dwm_settings::load().transfers.drag_threshold_px,
                        });
                        return Ok(Some(false));
                    }
                }
            }
        }
        if input.kind == IN_MOVE {
            if let Some(candidate) = &self.candidate {
                let current = snapshot(app, self.revision).is_some_and(|current| {
                    current.page == candidate.context.page
                        && current.profile == candidate.context.profile
                        && current.index == candidate.context.index
                        && current.revision == candidate.context.revision
                        && current.editor == candidate.context.editor
                });
                if !current {
                    self.candidate = None;
                    return Ok(None);
                }
                let dx = i64::from(input.x) - i64::from(candidate.point.0);
                let dy = i64::from(input.y) - i64::from(candidate.point.1);
                if dx * dx + dy * dy >= i64::from(candidate.threshold).pow(2) {
                    let payload =
                        transfer::Payload::text(candidate.context.selected_text().unwrap())
                            .map_err(native)?;
                    self.source = Some(
                        window
                            .transfers()?
                            .start_drag(payload, 1, TransferAction::Copy)
                            .map_err(native)?,
                    );
                    self.candidate = None;
                }
                return Ok(Some(false));
            }
        }
        if input.kind == IN_UP && input.pointer_button() == 0 && self.candidate.take().is_some() {
            if let Some(position) = caret_at(app, spots, (input.x, input.y), font) {
                place(app, position);
                self.invalidate();
                return Ok(Some(true));
            }
        }
        Ok(None)
    }
    pub fn event(
        &mut self,
        window: &mut Window,
        app: &mut App,
        spots: &[Hotspot],
        font: &Font,
        event: transfer::Event,
    ) -> dunit::Result<bool> {
        let revision = self.revision;
        let mut changed = self
            .clipboard
            .event(window.transfers()?, &event, |context, text| {
                Ok(self.focused && apply(app, revision, context, text))
            })?;
        let m = event.message;
        if event.source
            && matches!(m.kind, Kind::Cancelled | Kind::Finished)
            && self.source == Some(m.request)
        {
            self.source = None;
        }
        if m.kind == Kind::Result && self.source == Some(m.request) && m.arg0 != Status::Ok as u32 {
            self.source = None;
        }
        match m.kind {
            Kind::OpenData => {
                let result = (|| {
                    let payload = event.payload.ok_or(transfer::Error(libdunit::EINVAL))?;
                    if payload.files.len() != 1 {
                        return Err(transfer::Error(libdunit::EOPNOTSUPP));
                    }
                    let text =
                        String::from_utf8(payload.files[0].read_all(document_editor::MAX_BYTES)?)
                            .map_err(|_| transfer::Error(libdunit::EINVAL))?;
                    dwm_settings::validate_installed_document(&text)
                        .map_err(|_| transfer::Error(libdunit::EINVAL))?;
                    let baseline = dwm_settings::load_document()
                        .map_err(|_| transfer::Error(libdunit::EIO))?;
                    let mut editor = Editor::new(text);
                    editor.original = baseline;
                    editor.dirty = true;
                    app.editor = Some(editor);
                    app.page = 6;
                    app.editor_page = 0;
                    app.scroll_x = 0;
                    app.scroll_y = 0;
                    app.profile_focused = false;
                    app.document_error.clear();
                    app.error = false;
                    app.status = "Opened a document draft. Apply explicitly to validate and save.";
                    Ok(())
                })();
                let status = if result.is_ok() {
                    Status::Ok
                } else {
                    Status::Failed
                };
                if let Err(error) = result {
                    app.status = "Cannot open this document.";
                    app.document_error = String::from(libdunit::error_name(error.0));
                    app.error = true;
                }
                window
                    .transfers()?
                    .complete(m.transfer, status)
                    .map_err(native)?;
                changed = true;
            }
            Kind::DragEnter => {
                if let Some(offer) = event.offer {
                    self.drop = Some(Drop {
                        id: m.transfer,
                        offer,
                        point: (0, 0),
                        accepting: false,
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
                        &text_clipboard::preferences(),
                        TransferAction::Copy,
                    );
                    let accepting = selected.is_some() && target(app, spots, drop.point).is_some();
                    if accepting && !drop.accepting {
                        window
                            .transfers()?
                            .drag_accept(drop.id, selected.unwrap().format, TransferAction::Copy)
                            .map_err(native)?;
                    } else if !accepting && (drop.accepting || !drop.decided) {
                        window.transfers()?.drag_reject(drop.id).map_err(native)?;
                    }
                    drop.accepting = accepting;
                    drop.decided = true;
                }
            }
            Kind::DragDrop => {
                if let Some(drop) = self.drop.as_mut().filter(|d| d.id == m.transfer) {
                    if let Some(action) = target(app, spots, drop.point) {
                        let position = caret_at(app, spots, drop.point, font);
                        app.apply(action);
                        if let Some(position) = position {
                            place(app, position);
                        }
                        drop.context = snapshot(app, self.revision);
                        changed = true;
                    }
                }
            }
            Kind::Data => {
                if self.drop.as_ref().is_some_and(|d| d.id == m.transfer) {
                    let drop = self.drop.take().unwrap();
                    let accepted = drop
                        .context
                        .as_ref()
                        .zip(event.payload.as_ref().and_then(|p| p.text_value()))
                        .is_some_and(|(context, text)| {
                            self.focused && apply(app, self.revision, context, text)
                        });
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
                    changed |= accepted;
                }
            }
            Kind::Cancelled | Kind::Finished | Kind::DragLeave => {
                if self.drop.as_ref().is_some_and(|d| d.id == m.transfer) {
                    self.drop = None;
                }
            }
            _ => {}
        }
        if changed && m.kind != Kind::DragDrop {
            self.invalidate();
        }
        Ok(changed)
    }
}
