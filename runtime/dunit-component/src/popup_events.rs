//! Input capture, menu traversal and controlled selection, independent of IPC.
use crate::popup::{PopupItem, Trigger};
use crate::{
    Application, Component, Key, MenuEntry, Navigation, OpenPopup, PointerEvent as P,
    PopupCommand as K, PopupHit, Result, View,
};
use alloc::{string::String, vec::Vec};
impl<C: Component> Application<C> {
    pub fn cancel_popup_press(&mut self) {
        if let Some(p) = &mut self.overlays.open {
            p.pressed = None;
        }
    }
    pub fn popup_state(&self) -> Option<&OpenPopup> {
        self.overlays.open.as_ref()
    }
    pub fn accessibility_expanded(&self, key: &str) -> Option<bool> {
        self.view.find(key)?.accessibility_has_popup()?;
        Some(self.overlays.open.as_ref().is_some_and(|p| p.owner == key))
    }
    pub fn popup_items(&self, prefix: &[usize]) -> Option<Vec<PopupItem>> {
        let p = self.overlays.open.as_ref()?;
        self.view.find(&p.owner)?.popup_items(p.context, prefix)
    }
    pub fn close_popup(&mut self) -> bool {
        let Some(p) = self.overlays.open.take() else {
            return false;
        };
        if let Some(key) = p.restore {
            self.set_focus(&key);
        }
        true
    }
    pub(crate) fn reconcile_overlays(&mut self, new: &View<C::Message>) {
        self.overlays.bar_focus.retain(|bar, key| {
            new.find(bar).is_some_and(|v| {
                v.menu_bar
                    && v.children
                        .iter()
                        .any(|c| &c.key == key && !c.interaction.disabled)
            })
        });
        if let Some(open) = &self.overlays.open {
            let same = self
                .view
                .find(&open.owner)
                .zip(new.find(&open.owner))
                .is_some_and(|(a, b)| {
                    if b.interaction.disabled || a.kind != b.kind {
                        return false;
                    }
                    fn shape<M>(
                        a: &View<M>,
                        b: &View<M>,
                        context: bool,
                        prefix: &mut Vec<usize>,
                    ) -> bool {
                        let (Some(x), Some(y)) = (
                            a.popup_items(context, prefix),
                            b.popup_items(context, prefix),
                        ) else {
                            return false;
                        };
                        if x != y {
                            return false;
                        }
                        for (i, item) in x.iter().enumerate() {
                            if item.submenu {
                                prefix.push(i);
                                let same = shape(a, b, context, prefix);
                                prefix.pop();
                                if !same {
                                    return false;
                                }
                            }
                        }
                        true
                    }
                    shape(a, b, open.context, &mut Vec::new())
                });
            if !same {
                self.close_popup();
                self.overlays.suppress_up = true;
            }
        }
        if let Some(key) = self.overlays.tooltip.owner.as_deref() {
            let same = self
                .view
                .find(key)
                .zip(new.find(key))
                .is_some_and(|(a, b)| {
                    a.kind == b.kind && a.tooltip == b.tooltip && !b.interaction.disabled
                });
            if !same {
                self.dismiss_tooltip();
            }
        }
    }
    pub fn open_context_menu(&mut self, target: &str, point: Option<(f32, f32)>) -> bool {
        if point.is_some_and(|(x, y)| !x.is_finite() || !y.is_finite()) {
            return false;
        }
        let owner = self
            .view
            .ancestor_with(target, &|v| {
                v.context_menu.is_some() && !v.interaction.disabled
            })
            .map(|v| v.key.clone());
        owner.is_some_and(|key| self.begin_popup(&key, true, point, false))
    }
    fn begin_popup(
        &mut self,
        key: &str,
        context: bool,
        point: Option<(f32, f32)>,
        last: bool,
    ) -> bool {
        let Some(owner) = self.view.find(key).filter(|v| !v.interaction.disabled) else {
            return false;
        };
        let Some(items) = owner.popup_items(context, &[]) else {
            return false;
        };
        let selected = items.iter().position(|i| i.selected && i.enabled);
        let active = selected.or_else(|| {
            if last {
                items.iter().rposition(|i| i.enabled)
            } else {
                items.iter().position(|i| i.enabled)
            }
        });
        self.close_popup();
        self.dismiss_tooltip();
        let restore = if context {
            self.focused_key().map(String::from)
        } else {
            Some(key.into())
        };
        if !context {
            self.set_focus(key);
        } else {
            self.view.visit_mut(&mut |v| {
                v.interaction.pressed = false;
                if let Some(i) = &mut v.input {
                    i.dragging = false;
                }
            });
        }
        self.overlays.open = Some(OpenPopup {
            owner: key.into(),
            context,
            path: active.into_iter().collect(),
            point,
            pressed: None,
            restore,
        });
        self.overlays.suppress_up = false;
        true
    }
    fn bar_siblings(&self, key: &str) -> Vec<String> {
        self.view
            .ancestor_with(key, &|v| v.menu_bar)
            .map_or_else(Vec::new, |bar| {
                bar.children
                    .iter()
                    .filter(|v| v.popup.is_some() && !v.interaction.disabled)
                    .map(|v| v.key.clone())
                    .collect()
            })
    }
    fn switch_bar(&mut self, forward: bool) -> bool {
        let key = self
            .overlays
            .open
            .as_ref()
            .map(|p| p.owner.as_str())
            .or_else(|| self.focused_key());
        let Some(key) = key else {
            return false;
        };
        let keys = self.bar_siblings(key);
        let Some(index) = keys.iter().position(|k| k == key) else {
            return false;
        };
        let next = if forward {
            (index + 1) % keys.len()
        } else {
            (index + keys.len() - 1) % keys.len()
        };
        let key = keys[next].clone();
        if self.overlays.open.is_some() {
            self.begin_popup(&key, false, None, false)
        } else {
            self.set_focus(&key);
            true
        }
    }
    fn change_active(&mut self, command: K) -> bool {
        let Some(open) = self.overlays.open.as_ref() else {
            return false;
        };
        let mut prefix = open.path.clone();
        let old = prefix.pop();
        let Some(items) = self.popup_items(&prefix) else {
            return false;
        };
        let choices: Vec<_> = items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.enabled)
            .map(|(i, _)| i)
            .collect();
        if choices.is_empty() {
            return true;
        }
        let position = old.and_then(|i| choices.iter().position(|v| *v == i));
        let index = match command {
            K::Home => 0,
            K::End => choices.len() - 1,
            K::Up => position.map_or(choices.len() - 1, |i| {
                (i + choices.len() - 1) % choices.len()
            }),
            _ => position.map_or(0, |i| (i + 1) % choices.len()),
        };
        prefix.push(choices[index]);
        let p = self.overlays.open.as_mut().unwrap();
        p.path = prefix;
        p.pressed = None;
        true
    }
}
impl<C: Component> Application<C>
where
    C::Message: Clone,
{
    pub fn popup_command(&mut self, command: K) -> Result<bool> {
        self.dismiss_tooltip();
        if command == K::MenuBar {
            if self.close_popup() {
                return Ok(true);
            }
            let mut nodes = Vec::new();
            self.view.flatten(&mut nodes);
            let key = nodes
                .into_iter()
                .find(|v| v.menu_bar && !v.interaction.disabled)
                .and_then(|b| b.children.iter().find(|v| !v.interaction.disabled))
                .map(|v| v.key.clone());
            if let Some(key) = key {
                self.commit_spinbox()?;
                self.set_focus(&key);
                return Ok(true);
            }
            return Ok(false);
        }
        if self.overlays.open.is_none() {
            let key = self.focused_key().map(String::from);
            if let Some(key) = key {
                if self.view.find(&key).is_some_and(|v| v.popup.is_some()) {
                    return Ok(match command {
                        K::Up => self.begin_popup(&key, false, None, true),
                        K::Down | K::Activate => self.begin_popup(&key, false, None, false),
                        K::Left => self.switch_bar(false),
                        K::Right => self.switch_bar(true),
                        _ => false,
                    });
                }
            }
            return Ok(false);
        }
        match command {
            K::Up | K::Down | K::Home | K::End => {
                self.change_active(command);
            }
            K::Escape => {
                if self.overlays.open.as_ref().unwrap().path.len() > 1 {
                    let p = self.overlays.open.as_mut().unwrap();
                    p.path.pop();
                    p.pressed = None;
                } else {
                    self.close_popup();
                }
            }
            K::Left => {
                if self.overlays.open.as_ref().unwrap().path.len() > 1 {
                    let p = self.overlays.open.as_mut().unwrap();
                    p.path.pop();
                    p.pressed = None;
                } else {
                    self.switch_bar(false);
                }
            }
            K::Right => {
                if !self.enter_submenu() {
                    self.switch_bar(true);
                }
            }
            K::Activate => {
                self.activate_popup()?;
            }
            K::MenuBar => {}
        }
        Ok(true)
    }
    fn enter_submenu(&mut self) -> bool {
        let Some(p) = self.overlays.open.as_ref() else {
            return false;
        };
        let Some(items) = self.popup_items(&p.path) else {
            return false;
        };
        let first = items.iter().position(|i| i.enabled).unwrap_or(usize::MAX);
        self.overlays.open.as_mut().unwrap().path.push(first);
        true
    }
    fn activate_popup(&mut self) -> Result<()> {
        if self.enter_submenu() {
            return Ok(());
        }
        let Some(p) = self.overlays.open.as_ref() else {
            return Ok(());
        };
        let Some(&index) = p.path.last() else {
            return Ok(());
        };
        let prefix = &p.path[..p.path.len() - 1];
        let Some(item) = self
            .popup_items(prefix)
            .and_then(|items| items.get(index).cloned())
            .filter(|i| i.enabled && !i.submenu)
        else {
            return Ok(());
        };
        let owner = self.view.find(&p.owner).unwrap();
        let message = if !p.context {
            match owner.popup.as_ref().map(|p| &p.trigger) {
                Some(Trigger::Select {
                    selected,
                    on_change,
                    ..
                }) => {
                    if selected.as_deref() == Some(&item.key) {
                        None
                    } else {
                        Some(on_change(item.key.clone()))
                    }
                }
                _ => owner.menu_entry(false, &p.path).and_then(|i| {
                    if let MenuEntry::Action(m) = &i.entry {
                        Some(m.clone())
                    } else {
                        None
                    }
                }),
            }
        } else {
            owner.menu_entry(true, &p.path).and_then(|i| {
                if let MenuEntry::Action(m) = &i.entry {
                    Some(m.clone())
                } else {
                    None
                }
            })
        };
        self.close_popup();
        if let Some(message) = message {
            self.update(message)?;
        }
        Ok(())
    }
    pub(crate) fn popup_navigation(&mut self, direction: Navigation) -> Result<bool> {
        self.popup_command(match direction {
            Navigation::Up => K::Up,
            Navigation::Down => K::Down,
            Navigation::Left => K::Left,
            Navigation::Right => K::Right,
        })
    }
    pub(crate) fn popup_key(&mut self, key: Key) -> Result<bool> {
        if key.modifiers == 4 {
            let mut nodes = Vec::new();
            self.view.flatten(&mut nodes);
            let owner = nodes
                .into_iter()
                .find(|v| {
                    !v.interaction.disabled
                        && v.popup
                            .as_ref()
                            .is_some_and(|p| p.mnemonic == Some(key.character.to_ascii_lowercase()))
                })
                .map(|v| v.key.clone());
            if let Some(key) = owner {
                self.commit_spinbox()?;
                return Ok(self.begin_popup(&key, false, None, false));
            }
        }
        if self.overlays.open.is_some() {
            if key.character == '\t' && key.modifiers & !1 == 0 {
                self.close_popup();
                return Ok(false);
            }
            if key.modifiers == 0 {
                match key.character {
                    '\u{1b}' => return self.popup_command(K::Escape),
                    '\n' | '\r' | ' ' => return self.popup_command(K::Activate),
                    c if !c.is_control() => {
                        let p = self.overlays.open.as_ref().unwrap();
                        let mut prefix = p.path.clone();
                        let current = prefix.pop();
                        let items = self.popup_items(&prefix).unwrap_or_default();
                        let matches: Vec<_> = items
                            .iter()
                            .enumerate()
                            .filter(|(_, i)| {
                                i.enabled
                                    && (i.mnemonic == Some(c.to_ascii_lowercase())
                                        || i.label
                                            .chars()
                                            .next()
                                            .is_some_and(|x| x.to_lowercase().eq(c.to_lowercase())))
                            })
                            .map(|(n, _)| n)
                            .collect();
                        if let Some(&index) = matches
                            .iter()
                            .find(|&&i| current.is_none_or(|old| i > old))
                            .or(matches.first())
                        {
                            let mnemonic = items[index].mnemonic.is_some();
                            prefix.push(index);
                            self.overlays.open.as_mut().unwrap().path = prefix;
                            if mnemonic {
                                self.activate_popup()?;
                            }
                        }
                    }
                    _ => {}
                }
            }
            return Ok(true);
        }
        if key.modifiers == 0 && matches!(key.character, ' ' | '\n' | '\r') {
            if let Some(key) = self.focused_key().map(String::from) {
                if self.view.find(&key).is_some_and(|v| v.popup.is_some()) {
                    return Ok(self.begin_popup(&key, false, None, false));
                }
            }
        }
        Ok(false)
    }
    pub fn popup_pointer(
        &mut self,
        event: P,
        hit: Option<PopupHit>,
        target: Option<&str>,
    ) -> Result<bool> {
        match event {
            P::Down => {
                self.overlays.pointer_down = true;
                self.dismiss_tooltip();
            }
            P::Up | P::Leave => self.overlays.pointer_down = false,
            P::Move => {}
        }
        if self.overlays.open.is_none() {
            if self.overlays.suppress_up {
                if event == P::Down {
                    self.overlays.suppress_up = false;
                } else if event == P::Up {
                    self.overlays.suppress_up = false;
                    return Ok(true);
                }
            }
            return Ok(false);
        }
        let owner = self.overlays.open.as_ref().unwrap().owner.clone();
        if event == P::Move && hit.is_none() {
            if let Some(key) = target {
                if key != owner && self.bar_siblings(&owner).iter().any(|k| k == key) {
                    self.begin_popup(key, false, None, false);
                    return Ok(true);
                }
            }
        }
        match hit {
            Some(PopupHit::Item {
                owner: hit_owner,
                path,
            }) if hit_owner == owner => {
                let mut prefix = path.clone();
                let index = prefix.pop();
                let item =
                    index.and_then(|i| self.popup_items(&prefix).and_then(|v| v.get(i).cloned()));
                let enabled = item.as_ref().is_some_and(|i| i.enabled);
                match event {
                    P::Down => {
                        let p = self.overlays.open.as_mut().unwrap();
                        p.pressed = enabled.then_some(path.clone());
                        if enabled {
                            p.path = path;
                        }
                    }
                    P::Move => {
                        let p = self.overlays.open.as_mut().unwrap();
                        if p.pressed.as_ref() != Some(&path) {
                            p.pressed = None;
                        }
                        if enabled
                            && (!p.path.starts_with(&path)
                                || (p.path.len() == path.len()
                                    && item.as_ref().is_some_and(|i| i.submenu)))
                        {
                            p.path = path;
                            if item.unwrap().submenu {
                                self.enter_submenu();
                            }
                        }
                    }
                    P::Up => {
                        let pressed = self.overlays.open.as_mut().unwrap().pressed.take();
                        if enabled && pressed.as_ref() == Some(&path) {
                            self.overlays.open.as_mut().unwrap().path = path;
                            self.activate_popup()?;
                        }
                    }
                    P::Leave => {
                        self.overlays.open.as_mut().unwrap().pressed = None;
                    }
                }
            }
            hit => {
                let p = self.overlays.open.as_mut().unwrap();
                p.pressed = None;
                if event == P::Down && hit.is_none() {
                    self.close_popup();
                    self.overlays.suppress_up = true;
                }
            }
        }
        Ok(true)
    }
    pub(crate) fn trigger_popup(&mut self, key: &str) -> bool {
        if self.overlays.open.as_ref().is_some_and(|p| p.owner == key) {
            self.close_popup()
        } else {
            self.begin_popup(key, false, None, false)
        }
    }
}
