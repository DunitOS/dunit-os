//! Focus traversal and radio-group selection over the current typed view.
use crate::{Application, Component, Result, View};
use alloc::{collections::BTreeMap, vec::Vec};
use dunit_ui::NodeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigation {
    Left,
    Right,
    Up,
    Down,
}

impl<M: Clone> View<M> {
    pub(crate) fn activation_message(&self) -> Option<M> {
        if self.radio_group.is_some() && self.interaction.checked {
            None
        } else {
            self.activation.clone()
        }
    }
}
impl<C: Component> Application<C> {
    pub(crate) fn tab_focus(&mut self, reverse: bool) -> bool {
        let mut nodes = Vec::new();
        self.view.flatten(&mut nodes);
        // Each group has one Tab stop: focused member, selected enabled member,
        // or first enabled member. Arrow keys move inside the group.
        let mut groups = BTreeMap::new();
        for (index, node) in nodes.iter().enumerate() {
            if node.interaction.disabled {
                continue;
            }
            if let Some(group) = node.radio_group.as_deref() {
                let rank = if node.interaction.focused {
                    2
                } else if node.interaction.checked {
                    1
                } else {
                    0
                };
                let choice = groups.entry(group).or_insert((index, rank));
                if rank > choice.1 {
                    *choice = (index, rank);
                }
            }
        }
        let order: Vec<_> = nodes
            .iter()
            .enumerate()
            .filter(|(index, v)| {
                v.focusable()
                    && !v.interaction.disabled
                    && v.radio_group
                        .as_deref()
                        .map_or(true, |g| groups[g].0 == *index)
            })
            .map(|(i, _)| NodeId(i as u32))
            .collect();
        let current = order
            .iter()
            .copied()
            .find(|id| nodes[id.0 as usize].interaction.focused);
        let next = if reverse {
            dunit_widgets::focus_prev(&order, current)
        } else {
            dunit_widgets::focus_next(&order, current)
        };
        let Some(next) = next else {
            return false;
        };
        let target = nodes[next.0 as usize].key.clone();
        self.set_focus(&target);
        true
    }
    fn set_focus(&mut self, target: &str) {
        self.view.visit_mut(&mut |v| {
            v.interaction.focused = v.focusable() && !v.interaction.disabled && v.key == target;
            v.interaction.pressed = false;
            if let Some(input) = &mut v.input {
                input.dragging = false;
                input.caret_visible = true;
            }
        });
    }
    /// Arrow navigation selects an enabled member of the focused radio group.
    /// Selection is committed only by the message handled by the model.
    pub fn navigate(&mut self, direction: Navigation) -> Result<bool>
    where
        C::Message: Clone,
    {
        let mut nodes = Vec::new();
        self.view.flatten(&mut nodes);
        let Some(focused) = nodes
            .iter()
            .find(|v| v.interaction.focused && !v.interaction.disabled)
        else {
            return Ok(false);
        };
        let Some(group) = focused.radio_group.as_deref() else {
            return Ok(false);
        };
        let order: Vec<_> = nodes
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.interaction.disabled && v.radio_group.as_deref() == Some(group))
            .map(|(i, _)| NodeId(i as u32))
            .collect();
        let current = order
            .iter()
            .copied()
            .find(|id| nodes[id.0 as usize].interaction.focused);
        let next = match direction {
            Navigation::Left | Navigation::Up => dunit_widgets::focus_prev(&order, current),
            Navigation::Right | Navigation::Down => dunit_widgets::focus_next(&order, current),
        };
        let Some(next) = next else {
            return Ok(false);
        };
        let node = nodes[next.0 as usize];
        let target = node.key.clone();
        let message = node.activation_message();
        self.set_focus(&target);
        if let Some(message) = message {
            self.update(message)?;
        }
        Ok(true)
    }
}
