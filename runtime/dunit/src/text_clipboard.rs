//! Shared asynchronous text defaults for controlled widgets and custom native
//! editors. The adapter supplies a context snapshot and validates it when applying
//! the single replacement; this module never edits a model while receiving IPC.
use crate::{Error, Result};
use dunit_transfer::{Action, Client, Event, Format, Kind, Offer, Payload, Status};
/// Password/disabled fields must return no export; read-only fields may export
/// a selection but cannot accept Cut/Paste.
pub trait Context {
    fn selected_text(&self) -> Option<&str>;
    fn writable(&self) -> bool;
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Copy,
    Cut,
    Paste,
}
impl Command {
    pub fn from_scancode(scan: i32) -> Option<Self> {
        match scan {
            0x2e => Some(Self::Copy),
            0x2d => Some(Self::Cut),
            0x2f => Some(Self::Paste),
            _ => None,
        }
    }
}
/// Keyboard defaults shared by custom native editors. Clipboard commands are
/// dispatched separately, after application event cancellation.
pub fn edit_command(
    input: &crate::window::Input,
    multiline: bool,
) -> Option<crate::ui::EditCommand> {
    use crate::ui::EditCommand::*;
    let modifiers = input.y as u8;
    if modifiers & !(libdunit::KEYMOD_CTRL | libdunit::KEYMOD_SHIFT) != 0 {
        return None;
    }
    let word = modifiers & libdunit::KEYMOD_CTRL != 0;
    let extend = modifiers & libdunit::KEYMOD_SHIFT != 0;
    match input.x {
        0x4b => Some(Left { extend, word }),
        0x4d => Some(Right { extend, word }),
        0x47 if word && multiline => Some(DocumentHome { extend }),
        0x4f if word && multiline => Some(DocumentEnd { extend }),
        0x47 => Some(Home { extend }),
        0x4f => Some(End { extend }),
        0x48 if multiline => Some(Up { extend }),
        0x50 if multiline => Some(Down { extend }),
        0x0e => Some(Backspace { word }),
        0x53 => Some(Delete { word }),
        0x1e if word => Some(SelectAll),
        0x2c if word && extend => Some(Redo),
        0x2c if word => Some(Undo),
        0x15 if word => Some(Redo),
        _ if !word => char::from_u32(input.value).and_then(|c| {
            if multiline && matches!(c, '\n' | '\r') {
                Some(Insert('\n'))
            } else if !c.is_control() {
                Some(Insert(c))
            } else {
                None
            }
        }),
        _ => None,
    }
}
pub fn preferences() -> Offer {
    Offer {
        formats: alloc::vec![Format::Text],
        actions: 1,
        preferred: Action::Copy,
    }
}
pub struct Session<C> {
    cut: Option<(u64, C)>,
    paste: Option<(u64, Option<u64>, C)>,
}
impl<C> Default for Session<C> {
    fn default() -> Self {
        Self {
            cut: None,
            paste: None,
        }
    }
}
fn native(error: dunit_transfer::Error) -> Error {
    Error(error.0)
}
impl<C: Context> Session<C> {
    pub fn command(&mut self, client: &mut Client, command: Command, context: C) -> Result<()> {
        match command {
            Command::Copy | Command::Cut => {
                if command == Command::Cut && !context.writable() {
                    return Ok(());
                }
                if let Some(text) = context.selected_text() {
                    let request = client
                        .publish_clipboard(Payload::text(text).map_err(native)?, 1, Action::Copy)
                        .map_err(native)?;
                    self.cut = if command == Command::Cut {
                        Some((request, context))
                    } else {
                        None
                    };
                }
            }
            Command::Paste => {
                if context.writable() {
                    let request = client.read_clipboard(preferences()).map_err(native)?;
                    self.paste = Some((request, None, context));
                }
            }
        }
        Ok(())
    }
    /// `apply` checks the current model/focus against the saved context and
    /// performs one atomic replacement. False means stale/rejected, with no edit.
    /// Returns whether an accepted edit needs painting.
    pub fn event(
        &mut self,
        client: &mut Client,
        event: &Event,
        mut apply: impl FnMut(&C, &str) -> Result<bool>,
    ) -> Result<bool> {
        let m = event.message;
        match m.kind {
            Kind::Result => {
                if self.cut.as_ref().is_some_and(|(r, _)| *r == m.request) {
                    let (_, context) = self.cut.take().unwrap();
                    if m.arg0 == Status::Ok as u32 {
                        return apply(&context, "");
                    }
                }
                if m.arg0 != Status::Ok as u32
                    && self.paste.as_ref().is_some_and(|(r, _, _)| *r == m.request)
                {
                    self.paste = None;
                }
            }
            Kind::Offer => {
                if let Some((request, id, _)) = &mut self.paste {
                    if *request == m.request {
                        *id = Some(m.transfer);
                    }
                }
            }
            Kind::Data => {
                if self
                    .paste
                    .as_ref()
                    .is_some_and(|(r, id, _)| *r == m.request && *id == Some(m.transfer))
                {
                    let (_, _, context) = self.paste.take().unwrap();
                    let changed =
                        if let Some(text) = event.payload.as_ref().and_then(|p| p.text_value()) {
                            apply(&context, text)?
                        } else {
                            false
                        };
                    client
                        .complete(
                            m.transfer,
                            if changed {
                                Status::Ok
                            } else {
                                Status::Cancelled
                            },
                        )
                        .map_err(native)?;
                    return Ok(changed);
                }
            }
            Kind::Cancelled | Kind::Finished => {
                if event.source && self.cut.as_ref().is_some_and(|(r, _)| *r == m.request) {
                    self.cut = None;
                }
                if self
                    .paste
                    .as_ref()
                    .is_some_and(|(_, id, _)| *id == Some(m.transfer))
                {
                    self.paste = None;
                }
            }
            _ => {}
        }
        Ok(false)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Snapshot(u64);
    impl Context for Snapshot {
        fn selected_text(&self) -> Option<&str> {
            Some("selection")
        }
        fn writable(&self) -> bool {
            true
        }
    }
    fn event(kind: Kind, request: u64, id: u64, source: bool, payload: Option<Payload>) -> Event {
        Event {
            message: dunit_transfer::Message {
                kind,
                request,
                transfer: id,
                arg0: 0,
                arg1: 0,
            },
            offer: None,
            payload,
            source,
        }
    }
    #[test]
    fn completion_uses_transfer_identity_and_applies_the_whole_payload_once() {
        let mut session = Session {
            cut: None,
            paste: Some((7, Some(21), Snapshot(3))),
        };
        let mut client = Client::new(1, gui_protocol_v1::wire::FEATURE_DATA_TRANSFER);
        let mut calls = 0;
        session
            .event(
                &mut client,
                &event(Kind::Cancelled, 7, 101, false, None),
                |_, _| panic!("unrelated cancel"),
            )
            .unwrap();
        assert!(session.paste.is_some());
        let data = event(
            Kind::Data,
            7,
            21,
            false,
            Some(Payload::text("я👩‍💻\\nwhole").unwrap()),
        );
        assert!(session
            .event(&mut client, &data, |context, text| {
                calls += 1;
                assert_eq!(context.0, 3);
                assert_eq!(text, "я👩‍💻\\nwhole");
                Ok(true)
            })
            .unwrap());
        assert_eq!(calls, 1);
        assert!(session.paste.is_none());
    }
    #[test]
    fn cut_waits_for_durable_ack_and_rejected_stale_paste_never_creates_a_prefix() {
        let mut session = Session {
            cut: Some((4, Snapshot(5))),
            paste: Some((7, Some(21), Snapshot(3))),
        };
        let mut client = Client::new(1, gui_protocol_v1::wire::FEATURE_DATA_TRANSFER);
        session
            .event(
                &mut client,
                &event(Kind::Cancelled, 4, 100, false, None),
                |_, _| panic!("foreign cancellation"),
            )
            .unwrap();
        assert!(session.cut.is_some());
        assert!(session
            .event(
                &mut client,
                &event(Kind::Result, 4, 11, true, None),
                |context, text| {
                    assert_eq!(context.0, 5);
                    assert_eq!(text, "");
                    Ok(true)
                }
            )
            .unwrap());
        let data = event(
            Kind::Data,
            7,
            21,
            false,
            Some(Payload::text("whole").unwrap()),
        );
        assert!(!session.event(&mut client, &data, |_, _| Ok(false)).unwrap());
        assert!(session.paste.is_none());
    }
}
