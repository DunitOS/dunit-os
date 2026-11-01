use crate::{Application, Component, PointerEvent as P, Result, SliderCommand};
use alloc::{string::String, vec::Vec};
pub use dunit_widgets::splitter::SplitGeometry;
impl<C: Component> Application<C> {
    pub fn dragging_splitter_key(&self) -> Option<&str> {
        let mut nodes = Vec::new();
        self.view.flatten(&mut nodes);
        nodes
            .into_iter()
            .find(|v| {
                v.split.is_some()
                    && v.interaction.pressed
                    && !v.interaction.disabled
                    && self.in_scope(&v.key)
            })
            .map(|v| v.key.as_str())
    }
    pub fn splitter_command(&mut self, command: SliderCommand) -> Result<bool> {
        let mut message = None;
        let mut consumed = false;
        self.view.visit_mut(&mut |v| {
            if v.interaction.focused && !v.interaction.disabled {
                if let Some(s) = &v.split {
                    consumed = true;
                    let value = s.options.range.command(s.value, command);
                    if value != s.value {
                        message = Some((s.on_change)(value));
                    }
                }
            }
        });
        if let Some(message) = message {
            self.update(message)?;
        }
        Ok(consumed)
    }
    pub fn splitter_pointer(
        &mut self,
        event: P,
        target: Option<&str>,
        position: Option<(&str, f32, SplitGeometry)>,
    ) -> Result<bool>
    where
        C::Message: Clone,
    {
        let capture = if event == P::Down {
            None
        } else {
            self.dragging_splitter_key().map(String::from)
        };
        if event != P::Down && capture.is_none() {
            return Ok(false);
        }
        let selected = capture.as_deref().or(target);
        let Some(key) = selected
            .filter(|key| {
                self.in_scope(key)
                    && self
                        .view
                        .find(key)
                        .is_some_and(|v| v.split.is_some() && !v.interaction.disabled)
            })
            .map(String::from)
        else {
            return Ok(false);
        };
        self.pointer(event, target)?;
        let mut message = None;
        self.view.visit_mut(&mut |v| {
            if v.key != key || v.interaction.disabled {
                return;
            }
            let Some(s) = &mut v.split else {
                return;
            };
            let sample = position.filter(|(sample, _, _)| *sample == key);
            if event == P::Down {
                v.interaction.pressed = true;
                if let Some((_, coordinate, g)) = sample {
                    s.grab = g.grab(coordinate);
                }
            } else if event == P::Move && capture.is_some() {
                v.interaction.pressed = true;
            }
            if event != P::Leave && (event == P::Down || capture.is_some()) {
                if let Some((_, coordinate, g)) = sample {
                    if let Some(value) = g.value(coordinate, s.grab) {
                        let value = s.options.range.snap(value as i128);
                        if value != s.value {
                            message = Some((s.on_change)(value));
                        }
                    }
                }
            }
        });
        if let Some(message) = message {
            self.update(message)?;
        }
        Ok(true)
    }
}
