//! Passive progress views: all values are owned by the application model.
use crate::View;
use alloc::{string::String, vec::Vec};
pub use dunit_widgets::progress::ProgressOptions;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    Determinate {
        value: i64,
        options: ProgressOptions,
    },
    Busy {
        running: bool,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProgressBinding {
    pub label: String,
    pub state: ProgressState,
}
impl<M> View<M> {
    pub fn progress(
        key: impl Into<String>,
        label: impl Into<String>,
        value: i64,
        options: ProgressOptions,
    ) -> Self {
        let mut view = Self::new(
            key,
            dunit_ui::Kind::Element("Progress".into()),
            None,
            Vec::new(),
        );
        view.progress = Some(ProgressBinding {
            label: label.into(),
            state: ProgressState::Determinate { value, options },
        });
        view
    }
    pub fn busy(key: impl Into<String>, label: impl Into<String>, running: bool) -> Self {
        let mut view = Self::new(
            key,
            dunit_ui::Kind::Element("Busy".into()),
            None,
            Vec::new(),
        );
        view.progress = Some(ProgressBinding {
            label: label.into(),
            state: ProgressState::Busy { running },
        });
        view
    }
    pub fn progress_state(&self) -> Option<ProgressState> {
        self.progress.as_ref().map(|p| p.state)
    }
    pub fn accessibility_busy(&self) -> Option<bool> {
        self.progress
            .as_ref()
            .map(|p| matches!(p.state, ProgressState::Busy { running: true }))
    }
}
