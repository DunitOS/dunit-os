//! Controlled slider messages; range/value stay in the component model.
use crate::{Application, Component, PointerEvent, Result, View};
use alloc::string::String;
pub use dunit_widgets::slider::{SliderCommand, SliderGeometry, SliderOptions, SliderOrientation};
#[derive(Debug, Clone)]
pub(crate) struct SliderBinding<M> {
    pub label: String,
    pub value: i64,
    pub options: SliderOptions,
    pub on_change: fn(i64) -> M,
    pub grab: f32,
}
impl<M> PartialEq for SliderBinding<M> {
    fn eq(&self, other: &Self) -> bool {
        self.label == other.label
            && self.value == other.value
            && self.options == other.options
            && core::ptr::fn_addr_eq(self.on_change, other.on_change)
            && self.grab == other.grab
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliderRange {
    pub orientation: SliderOrientation,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub value: i64,
}
impl<M> View<M> {
    pub fn slider(
        key: impl Into<String>,
        label: impl Into<String>,
        value: i64,
        options: SliderOptions,
        on_change: fn(i64) -> M,
    ) -> Self {
        let mut view = Self::new(
            key,
            dunit_ui::Kind::Element("Slider".into()),
            None,
            alloc::vec::Vec::new(),
        );
        view.slider = Some(SliderBinding {
            label: label.into(),
            value,
            options,
            on_change,
            grab: 0.,
        });
        view
    }
    /// Vertical value slider: minimum at the bottom, maximum at the top.
    pub fn vertical_slider(
        key: impl Into<String>,
        label: impl Into<String>,
        value: i64,
        options: SliderOptions,
        on_change: fn(i64) -> M,
    ) -> Self {
        let mut view = Self::slider(key, label, value, options, on_change);
        view.kind = dunit_ui::Kind::Element("VerticalSlider".into());
        view
    }
    pub fn slider_range(&self) -> Option<SliderRange> {
        self.slider.as_ref().map(|s| SliderRange {
            orientation: SliderOrientation::from_tag(self.kind.tag()),
            min: s.options.min,
            max: s.options.max,
            step: s.options.step,
            value: s.value,
        })
    }
}
impl<C: Component> Application<C> {
    pub fn dragging_slider_key(&self) -> Option<&str> {
        let mut nodes = alloc::vec::Vec::new();
        self.view.flatten(&mut nodes);
        nodes
            .into_iter()
            .find(|v| v.slider.is_some() && v.interaction.pressed && !v.interaction.disabled)
            .map(|v| v.key.as_str())
    }
    pub fn slider_command(&mut self, command: SliderCommand) -> Result<bool> {
        let mut message = None;
        let mut consumed = false;
        self.view.visit_mut(&mut |v| {
            if !v.interaction.focused || v.interaction.disabled {
                return;
            }
            if let Some(s) = &v.slider {
                consumed = true;
                let value = s.options.command(s.value, command);
                if value != s.value {
                    message = Some((s.on_change)(value));
                }
            }
        });
        if let Some(message) = message {
            self.update(message)?;
        }
        Ok(consumed)
    }
    /// Position carries the primary-axis coordinate (x horizontal, y vertical),
    /// as returned by SliderGeometry::coordinate. Geometry is from the last
    /// painted layout. Capture continues outside the
    /// control, until Up/Leave/Escape/focus change/removal/disable/range change.
    pub fn slider_pointer(
        &mut self,
        event: PointerEvent,
        target: Option<&str>,
        position: Option<(&str, f32, SliderGeometry)>,
    ) -> Result<bool>
    where
        C::Message: Clone,
    {
        if event != PointerEvent::Down && self.dragging_input_key().is_some() {
            return Ok(false);
        }
        let capture = if event == PointerEvent::Down {
            None
        } else {
            self.dragging_slider_key().map(String::from)
        };
        let selected = capture.as_deref().or(target);
        let mut is_slider = false;
        self.view.visit_mut(&mut |v| {
            if Some(v.key.as_str()) == selected && v.slider.is_some() && !v.interaction.disabled {
                is_slider = true;
            }
        });
        if !is_slider {
            return Ok(false);
        }
        let key = String::from(selected.unwrap());
        self.pointer(event, target)?;
        let mut message = None;
        self.view.visit_mut(&mut |v| {
            if v.key != key || v.interaction.disabled {
                return;
            }
            let Some(s) = &mut v.slider else {
                return;
            };
            let sample = position.filter(|(sample, _, _)| *sample == key);
            if event == PointerEvent::Down {
                v.interaction.pressed = true;
                if let Some((_, coordinate, geometry)) = sample {
                    s.grab = geometry.grab(coordinate);
                }
            } else if event == PointerEvent::Move && capture.is_some() {
                v.interaction.pressed = true;
            }
            if event != PointerEvent::Leave && (event == PointerEvent::Down || capture.is_some()) {
                if let Some((_, coordinate, geometry)) = sample {
                    if let Some(fraction) = geometry.fraction(coordinate, s.grab) {
                        let value = s.options.at_fraction(fraction);
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
