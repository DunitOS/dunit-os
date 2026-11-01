//! Tooltip hover deadlines use caller-supplied monotonic milliseconds.
use crate::{Application, Component};
impl<C: Component> Application<C> {
    pub fn dismiss_tooltip(&mut self) -> bool {
        let visible = self.overlays.tooltip.visible;
        self.overlays.tooltip = Default::default();
        visible
    }
    pub fn tooltip_candidate(&self) -> Option<(&str, u64)> {
        if self.overlays.tooltip.visible {
            return None;
        }
        self.overlays
            .tooltip
            .owner
            .as_deref()
            .map(|key| (key, self.overlays.tooltip.since))
    }
    pub fn visible_tooltip(&self) -> Option<&str> {
        self.overlays
            .tooltip
            .visible
            .then_some(self.overlays.tooltip.owner.as_deref())
            .flatten()
    }
    pub fn tooltip_pointer(&mut self, target: Option<&str>, now: u64) -> bool {
        let owner = if self.overlays.open.is_none() && !self.overlays.pointer_down {
            target
                .filter(|target| self.in_scope(target))
                .and_then(|target| {
                    self.view
                        .ancestor_with(target, &|v| v.tooltip.is_some() && !v.interaction.disabled)
                })
                .filter(|v| self.in_scope(&v.key))
                .map(|v| v.key.clone())
        } else {
            None
        };
        if owner == self.overlays.tooltip.owner {
            return false;
        }
        let changed = self.overlays.tooltip.visible;
        self.overlays.tooltip = crate::popup::TooltipState {
            owner,
            since: now,
            visible: false,
        };
        changed
    }
    pub fn sample_tooltip(&mut self, now: u64, delay: u64) -> bool {
        let tip = &mut self.overlays.tooltip;
        if self.overlays.open.is_some()
            || tip.visible
            || tip.owner.is_none()
            || now.saturating_sub(tip.since) < delay
        {
            return false;
        }
        tip.visible = true;
        true
    }
}
