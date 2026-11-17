//! Session policy owns both the in-memory mirror and writes to the store.
use super::*;

pub(super) struct Session {
    enabled: bool,
    entries: Vec<settings::WinGeom>,
}

impl Session {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            entries: if enabled {
                settings::load_session()
            } else {
                Vec::new()
            },
        }
    }

    pub(super) fn set_restore(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        *self = Self::new(enabled);
    }

    pub(super) fn entries(&self) -> &[settings::WinGeom] {
        &self.entries
    }
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn lookup(&self, id: &str) -> Option<settings::WinGeom> {
        self.entries
            .iter()
            .find(|entry| entry.id.as_str() == id)
            .copied()
    }

    pub(super) fn remember(&mut self, geometry: settings::WinGeom) {
        if !self.enabled {
            return;
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.id.as_str() == geometry.id.as_str())
        {
            *entry = geometry;
        } else if self.entries.len() < settings::MAX_SESSION {
            self.entries.push(geometry);
        }
    }

    pub(super) fn save(&self) -> bool {
        self.enabled && settings::save_session(&self.entries)
    }

    pub(super) fn persist(&self) {
        if self.enabled && !self.save() {
            libdunit::println("gui_server: session save failed");
        }
    }
}
