//! Idle scrollbar visibility. Geometry and reserved gutters never disappear.
use super::Renderer;
use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
};
use dunit_render::ScrollArea;
use dunit_style::Style;
use dunit_ui::Tree;

#[derive(Clone, Default)]
pub(super) struct Visibility {
    last: u64,
    delay: u64,
    visible: bool,
    pending: bool,
}
impl Renderer {
    /// Wake scrollbars on the route from the target through its ancestors.
    /// `now` uses the same monotonic milliseconds as `render_at`.
    pub fn scroll_activity_at(&mut self, target: Option<&str>, now: u64) -> bool {
        self.scroll_clock = Some(now);
        let Some(snapshot) = &self.previous else {
            return false;
        };
        let mut current = target.and_then(|key| snapshot.tree.by_name(key));
        let mut changed = false;
        while let Some(id) = current {
            if let Some(key) = snapshot.tree.node(id).name.as_deref() {
                if let Some(state) = self.scroll_visibility.get_mut(key) {
                    changed |= !state.visible;
                    state.last = now;
                    state.visible = true;
                    state.pending = false;
                }
            }
            current = snapshot.tree.node(id).parent;
        }
        changed
    }
    /// The next one-shot idle deadline. A held thumb never schedules hiding.
    pub fn scroll_hide_deadline_ms(&self) -> Option<u64> {
        self.scroll_clock?;
        self.scroll_visibility
            .iter()
            .filter_map(|(key, state)| {
                (state.visible
                    && state.delay > 0
                    && !self.scroll_capture.as_ref().is_some_and(|c| &c.0 == key))
                .then(|| state.last.saturating_add(state.delay))
            })
            .min()
    }
    pub fn scrollbars_visible(&self, key: &str) -> Option<bool> {
        let snapshot = self.previous.as_ref()?;
        snapshot.areas[snapshot.tree.by_name(key)?.0 as usize]
            .as_ref()
            .map(|a| a.bars_visible)
    }
    pub(super) fn scroll_activity(&mut self, key: &str) {
        if let Some(state) = self.scroll_visibility.get_mut(key) {
            state.pending = true;
        }
    }
    pub(super) fn prepare_scroll_visibility<M>(
        &mut self,
        tree: &Tree,
        styles: &[Style],
        areas: &mut [Option<ScrollArea>],
        views: &[&crate::View<M>],
        now: Option<u64>,
    ) {
        self.scroll_clock = now;
        let Some(now) = now else {
            self.scroll_visibility.clear();
            return;
        };
        let focused = views
            .iter()
            .position(|v| v.interaction.focused && !v.interaction.disabled);
        let changed_focus = focused.is_some_and(|index| {
            self.previous
                .as_ref()
                .and_then(|p| p.focused_key.as_deref())
                != Some(views[index].key.as_str())
        });
        let mut focus_route = BTreeSet::new();
        if changed_focus {
            let mut current = focused.map(|index| dunit_ui::NodeId(index as u32));
            while let Some(id) = current {
                focus_route.insert(id);
                current = tree.node(id).parent;
            }
        }
        let mut next = BTreeMap::new();
        for node in tree.iter() {
            let index = node.id.0 as usize;
            let Some(area) = &mut areas[index] else {
                continue;
            };
            if area.track.is_none() && area.horizontal_track.is_none() {
                continue;
            }
            let key = node.name.as_deref().unwrap_or("");
            let delay = styles[index]
                .get("scrollbar-hide-delay")
                .and_then(|v| v.as_duration())
                .filter(|n| n.is_finite() && *n > 0.)
                .map_or(0, |n| (n as u64).max(1));
            let old_area = self.previous.as_ref().and_then(|p| {
                p.tree
                    .by_name(key)
                    .filter(|&id| p.tree.node(id).kind == node.kind)
                    .and_then(|id| p.areas[id.0 as usize])
            });
            let moved = old_area.is_none_or(|old| {
                old.viewport != area.viewport
                    || old.thumb != area.thumb
                    || old.horizontal_thumb != area.horizontal_thumb
            });
            let captured = self.scroll_capture.as_ref().is_some_and(|c| c.0 == key);
            let mut state = self.scroll_visibility.remove(key).unwrap_or(Visibility {
                last: now,
                visible: true,
                ..Default::default()
            });
            if state.pending
                || moved
                || captured
                || state.delay != delay
                || focus_route.contains(&node.id)
            {
                state.last = now;
            }
            state.pending = false;
            state.delay = delay;
            state.visible = delay == 0 || captured || now < state.last.saturating_add(delay);
            area.bars_visible = state.visible;
            next.insert(String::from(key), state);
        }
        self.scroll_visibility = next;
    }
}
