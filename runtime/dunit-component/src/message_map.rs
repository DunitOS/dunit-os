//! Message composition keeps child views in the parent's actual layout/scope.
use crate::View;
use alloc::{rc::Rc, vec::Vec};
fn menus<M, N>(
    items: Vec<crate::MenuItem<M>>,
    map: &Rc<dyn Fn(M) -> N>,
) -> Vec<crate::MenuItem<N>> {
    items
        .into_iter()
        .map(|i| crate::MenuItem {
            key: i.key,
            label: i.label,
            disabled: i.disabled,
            mnemonic: i.mnemonic,
            entry: match i.entry {
                crate::MenuEntry::Action(m) => crate::MenuEntry::Action(map(m)),
                crate::MenuEntry::Submenu(v) => crate::MenuEntry::Submenu(menus(v, map)),
                crate::MenuEntry::Separator => crate::MenuEntry::Separator,
            },
        })
        .collect()
}
impl<M: 'static> View<M> {
    /// Map a reusable child component's complete message surface, including
    /// callbacks, nested menus, default/cancel actions and collection editors.
    pub fn map_messages<N: 'static>(self, map: impl Fn(M) -> N + 'static) -> View<N> {
        self.map_with(Rc::new(map))
    }
    fn map_with<N: 'static>(self, map: Rc<dyn Fn(M) -> N>) -> View<N> {
        let capture_map = map.clone();
        let bubble_map = map.clone();
        View {
            tab_index: self.tab_index,
            mnemonic: self.mnemonic,
            capture: self
                .capture
                .map(|c| c.map(Rc::new(move |r| r.map(&|m| capture_map(m))))),
            bubble: self
                .bubble
                .map(|c| c.map(Rc::new(move |r| r.map(&|m| bubble_map(m))))),
            key: self.key,
            kind: self.kind,
            attrs: self.attrs,
            text: self.text,
            classes: self.classes,
            interaction: self.interaction,
            radio_group: self.radio_group,
            scroll_label: self.scroll_label,
            scroll_axes: self.scroll_axes,
            tree: self.tree,
            table: self.table,
            workspace: self.workspace,
            progress: self.progress,
            menu_bar: self.menu_bar,
            tooltip: self.tooltip,
            children: self
                .children
                .into_iter()
                .map(|c| c.map_with(map.clone()))
                .collect(),
            bindings: self
                .bindings
                .into_iter()
                .map(|(k, m)| (k, map(m)))
                .collect(),
            activation: self.activation.map(|m| map(m)),
            submit: self.submit.map(|m| map(m)),
            context_menu: self.context_menu.map(|v| menus(v, &map)),
            dialog: self.dialog.map(|d| crate::workspace::DialogBinding {
                label: d.label,
                cancel: d.cancel.map(|m| map(m)),
                default: d.default,
            }),
            popup: self.popup.map(|p| crate::popup::PopupBinding {
                label: p.label,
                mnemonic: p.mnemonic,
                trigger: match p.trigger {
                    crate::popup::Trigger::Menu(v) => crate::popup::Trigger::Menu(menus(v, &map)),
                    crate::popup::Trigger::Select {
                        options,
                        selected,
                        on_change,
                    } => crate::popup::Trigger::Select {
                        options,
                        selected,
                        on_change: on_change.map(map.clone()),
                    },
                },
            }),
            input: self.input.map(|i| crate::input::InputBinding {
                affinity: i.affinity,
                visual_goal: i.visual_goal,
                owner: i.owner,
                label: i.label,
                options: i.options,
                editor: i.editor,
                on_change: i.on_change.map(|c| c.map(map.clone())),
                dragging: i.dragging,
                caret_visible: i.caret_visible,
                caret_revision: i.caret_revision,
            }),
            slider: self.slider.map(|s| crate::slider::SliderBinding {
                label: s.label,
                value: s.value,
                options: s.options,
                grab: s.grab,
                on_change: s.on_change.map(map.clone()),
            }),
            split: self.split.map(|s| crate::workspace::SplitBinding {
                label: s.label,
                value: s.value,
                options: s.options,
                grab: s.grab,
                on_change: s.on_change.map(map.clone()),
            }),
            spinbox: self.spinbox.map(|s| crate::spinbox::SpinboxBinding {
                value: s.value,
                options: s.options,
                pressed: s.pressed,
                on_change: s.on_change.map(map.clone()),
            }),
            list: self.list.map(|l| crate::list::ListBinding {
                label: l.label,
                items: l.items,
                selection: l.selection,
                mode: l.mode,
                on_change: match l.on_change {
                    crate::list::SelectionHandler::List(c) => {
                        crate::list::SelectionHandler::List(c.map(map.clone()))
                    }
                    crate::list::SelectionHandler::Tree(c) => {
                        crate::list::SelectionHandler::Tree(c.map(map.clone()))
                    }
                    crate::list::SelectionHandler::Table(c) => {
                        crate::list::SelectionHandler::Table(c.map(map.clone()))
                    }
                },
            }),
            rename: self.rename.map(|r| crate::rename::RenameBinding {
                state: r.state,
                on_event: r.on_event.map(map.clone()),
            }),
        }
    }
}
