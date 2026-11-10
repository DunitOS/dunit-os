//! Descriptor nodes become the SDK's menu/choice/tooltip bindings, never painters.
use super::*;
use alloc::boxed::Box;
pub(super) struct Choice {
    pub option: SelectOption,
    pub selected: bool,
    pub message: u64,
}
pub(super) struct Extra<M> {
    pub items: Vec<MenuItem<M>>,
    pub options: Vec<Choice>,
    pub context: Option<Vec<MenuItem<M>>>,
    pub tooltip: Option<String>,
    pub mnemonic: Option<char>,
    pub pages: Vec<(Tab<M>, bool)>,
    pub actions: Vec<DialogAction<M>>,
    pub default: Option<String>,
    pub cancel: Option<M>,
    pub body: Option<String>,
}
impl<M> Default for Extra<M> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            options: Vec::new(),
            context: None,
            tooltip: None,
            mnemonic: None,
            pages: Vec::new(),
            actions: Vec::new(),
            default: None,
            cancel: None,
            body: None,
        }
    }
}
pub(super) fn extra<M>(entries: &mut [Option<Box<Extra<M>>>], index: usize) -> &mut Extra<M> {
    entries[index].get_or_insert_with(|| Box::new(Extra::default()))
}
pub(super) fn descriptor(kind: u32) -> bool {
    matches!(
        kind,
        DUNIT_UI_TAB
            | DUNIT_UI_TAB_SELECTED
            | DUNIT_UI_DIALOG_ACTION
            | DUNIT_UI_DIALOG_DEFAULT
            | DUNIT_UI_DIALOG_CANCEL
            | DUNIT_UI_MESSAGE_BODY
            | DUNIT_UI_MENU_ITEM
            | DUNIT_UI_MENU_SEPARATOR
            | DUNIT_UI_SUBMENU
            | DUNIT_UI_OPTION
            | DUNIT_UI_OPTION_SELECTED
            | DUNIT_UI_CONTEXT_MENU
            | DUNIT_UI_TOOLTIP
            | DUNIT_UI_MNEMONIC
    )
}
pub(super) fn parent_allowed(kind: u32, parent: u32) -> bool {
    match kind {
        DUNIT_UI_TAB | DUNIT_UI_TAB_SELECTED => parent == DUNIT_UI_TABS,
        DUNIT_UI_DIALOG_ACTION | DUNIT_UI_DIALOG_DEFAULT | DUNIT_UI_DIALOG_CANCEL => {
            matches!(parent, DUNIT_UI_DIALOG | DUNIT_UI_MESSAGE_BOX)
        }
        DUNIT_UI_MESSAGE_BODY => parent == DUNIT_UI_MESSAGE_BOX,
        DUNIT_UI_MENU_ITEM | DUNIT_UI_MENU_SEPARATOR | DUNIT_UI_SUBMENU => matches!(
            parent,
            DUNIT_UI_MENU | DUNIT_UI_SUBMENU | DUNIT_UI_CONTEXT_MENU
        ),
        DUNIT_UI_OPTION | DUNIT_UI_OPTION_SELECTED => parent == DUNIT_UI_SELECT,
        DUNIT_UI_CONTEXT_MENU | DUNIT_UI_TOOLTIP => !descriptor(parent),
        DUNIT_UI_MNEMONIC => matches!(
            parent,
            DUNIT_UI_MENU | DUNIT_UI_SELECT | DUNIT_UI_MENU_ITEM | DUNIT_UI_SUBMENU
        ),
        _ => {
            crate::workspace::container(parent)
                || matches!(
                    parent,
                    DUNIT_UI_COLUMN
                        | DUNIT_UI_ROW
                        | DUNIT_UI_STACK
                        | DUNIT_UI_GRID
                        | DUNIT_UI_MENU_BAR
                        | DUNIT_UI_RADIO_GROUP_COLUMN
                        | DUNIT_UI_RADIO_GROUP_ROW
                )
        }
    }
}
pub(super) unsafe fn bind<M: Clone>(
    node: &DunitUiNodeV1,
    key: String,
    mut own: Extra<M>,
    parent: &mut Extra<M>,
    action: fn(u64) -> M,
    mut children: Vec<View<M>>,
) -> dunit::Result<()> {
    let kind = node.kind & !DUNIT_UI_DISABLED;
    let disabled = node.kind & DUNIT_UI_DISABLED != 0;
    match kind {
        DUNIT_UI_TAB | DUNIT_UI_TAB_SELECTED => {
            if children.len() != 1 {
                return Err(invalid());
            }
            parent.pages.push((
                Tab::new(
                    key,
                    tree::text(node.text, node.text_len)?,
                    action(node.message),
                    children.remove(0),
                )
                .disabled(disabled),
                kind == DUNIT_UI_TAB_SELECTED,
            ));
        }
        DUNIT_UI_DIALOG_ACTION => parent.actions.push(DialogAction::new(
            key,
            tree::text(node.text, node.text_len)?,
            action(node.message),
        )),
        DUNIT_UI_DIALOG_DEFAULT => {
            if parent
                .default
                .replace(tree::text(node.text, node.text_len)?)
                .is_some()
            {
                return Err(invalid());
            }
        }
        DUNIT_UI_DIALOG_CANCEL => {
            if parent.cancel.replace(action(node.message)).is_some() {
                return Err(invalid());
            }
        }
        DUNIT_UI_MESSAGE_BODY => {
            if parent
                .body
                .replace(tree::text(node.text, node.text_len)?)
                .is_some()
            {
                return Err(invalid());
            }
        }
        DUNIT_UI_MENU_ITEM | DUNIT_UI_SUBMENU | DUNIT_UI_MENU_SEPARATOR => {
            let mut item = match kind {
                DUNIT_UI_MENU_ITEM => MenuItem::action(
                    key,
                    tree::text(node.text, node.text_len)?,
                    action(node.message),
                ),
                DUNIT_UI_SUBMENU => {
                    MenuItem::submenu(key, tree::text(node.text, node.text_len)?, own.items)
                }
                _ => MenuItem::separator(key),
            }
            .disabled(disabled);
            if let Some(mnemonic) = own.mnemonic {
                item = item.mnemonic(mnemonic);
            }
            parent.items.push(item);
        }
        DUNIT_UI_OPTION | DUNIT_UI_OPTION_SELECTED => parent.options.push(Choice {
            option: SelectOption::new(key, tree::text(node.text, node.text_len)?)
                .disabled(disabled),
            selected: kind == DUNIT_UI_OPTION_SELECTED,
            message: node.message,
        }),
        DUNIT_UI_CONTEXT_MENU => {
            if parent
                .context
                .replace(core::mem::take(&mut own.items))
                .is_some()
            {
                return Err(invalid());
            }
        }
        DUNIT_UI_TOOLTIP => {
            if parent
                .tooltip
                .replace(tree::text(node.text, node.text_len)?)
                .is_some()
            {
                return Err(invalid());
            }
        }
        DUNIT_UI_MNEMONIC => {
            let text = tree::text(node.text, node.text_len)?;
            let mut chars = text.chars();
            let mnemonic = chars.next().ok_or_else(invalid)?;
            if chars.next().is_some() || parent.mnemonic.replace(mnemonic).is_some() {
                return Err(invalid());
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}
pub(super) fn select<M: Clone + 'static>(
    key: String,
    label: String,
    choices: Vec<Choice>,
    action: fn(u64) -> M,
) -> dunit::Result<View<M>> {
    let mut selected = None;
    let mut options = Vec::new();
    let mut messages = Vec::new();
    for choice in choices {
        if choice.selected && selected.replace(choice.option.key.clone()).is_some() {
            return Err(invalid());
        }
        messages.push((choice.option.key.clone(), choice.message));
        options.push(choice.option);
    }
    Ok(
        View::select(key, label, selected.as_deref(), options, |key| key).map_messages(
            move |key| {
                // The SDK only dispatches keys from its validated option list.
                action(
                    messages
                        .iter()
                        .find(|(candidate, _)| *candidate == key)
                        .expect("SDK selection must reference a validated option")
                        .1,
                )
            },
        ),
    )
}
