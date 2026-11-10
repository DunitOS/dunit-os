//! Copy bounded foreign descriptions into real owned SDK Views.
use super::*;
pub(super) unsafe fn text(pointer: *const u8, length: usize) -> dunit::Result<String> {
    if length > DUNIT_UI_MAX_TEXT_BYTES as usize || (length > 0 && pointer.is_null()) {
        return Err(invalid());
    }
    if length == 0 {
        return Ok(String::new());
    }
    core::str::from_utf8(core::slice::from_raw_parts(pointer, length))
        .map(String::from)
        .map_err(|_| invalid())
}
#[cfg(test)]
pub(super) unsafe fn convert(nodes: &[DunitUiNodeV1]) -> dunit::Result<View<u64>> {
    convert_decorated(nodes, core::ptr::null_mut(), None)
}
#[cfg(test)]
pub(super) unsafe fn convert_decorated(
    nodes: &[DunitUiNodeV1],
    user: *mut c_void,
    decorate: Option<DunitUiDecorateV1>,
) -> dunit::Result<View<u64>> {
    convert_impl(
        nodes,
        user,
        decorate,
        None,
        None,
        None,
        |id| id,
        None,
        None,
        None,
        None,
    )
}
#[cfg(test)]
pub(super) unsafe fn convert_runtime(
    nodes: &[DunitUiNodeV1],
    user: *mut c_void,
    decorate: Option<DunitUiDecorateV1>,
    text: Option<&DunitUiTextV1>,
) -> dunit::Result<View<Message>> {
    convert_app(nodes, user, decorate, text, None)
}
#[cfg(test)]
pub(super) unsafe fn convert_app(
    nodes: &[DunitUiNodeV1],
    user: *mut c_void,
    decorate: Option<DunitUiDecorateV1>,
    text: Option<&DunitUiTextV1>,
    numbers: Option<&DunitUiNumbersV1>,
) -> dunit::Result<View<Message>> {
    convert_extended(nodes, user, decorate, text, numbers, None)
}
pub(super) unsafe fn convert_extended(
    nodes: &[DunitUiNodeV1],
    user: *mut c_void,
    decorate: Option<DunitUiDecorateV1>,
    text: Option<&DunitUiTextV1>,
    numbers: Option<&DunitUiNumbersV1>,
    collections: Option<&DunitUiCollectionsV1>,
) -> dunit::Result<View<Message>> {
    convert_impl(
        nodes,
        user,
        decorate,
        text,
        numbers,
        collections,
        Message::Action,
        Some(Message::Edit),
        Some(Message::Number),
        Some(Message::Collection),
        Some(Message::Rename),
    )
}
unsafe fn convert_impl<M: Clone + 'static>(
    nodes: &[DunitUiNodeV1],
    user: *mut c_void,
    decorate: Option<DunitUiDecorateV1>,
    inputs: Option<&DunitUiTextV1>,
    numbers: Option<&DunitUiNumbersV1>,
    collections: Option<&DunitUiCollectionsV1>,
    action: fn(u64) -> M,
    edit: Option<fn(u64, String) -> M>,
    numeric: Option<fn(u64, i64) -> M>,
    collection: Option<fn(u64, crate::collection::State) -> M>,
    renamed: Option<fn(u64, RenameEvent) -> M>,
) -> dunit::Result<View<M>> {
    if nodes.is_empty() || nodes.len() > MAX_VIEW_NODES || nodes[0].parent != DUNIT_UI_ROOT {
        return Err(invalid());
    }
    let mut children: Vec<Vec<View<M>>> = vec![Vec::new(); nodes.len()];
    let mut keys: Vec<Option<String>> = nodes
        .iter()
        .map(|node| text(node.key, node.key_len).map(Some))
        .collect::<dunit::Result<_>>()?;
    {
        let mut names = alloc::collections::BTreeSet::new();
        for key in &keys {
            let key = key.as_deref().ok_or_else(invalid)?;
            if key.is_empty() || !names.insert(key) {
                return Err(invalid());
            }
        }
    }
    let mut extras: Vec<Option<alloc::boxed::Box<crate::popup::Extra<M>>>> =
        (0..nodes.len()).map(|_| None).collect();
    let mut depths = vec![0; nodes.len()];
    for (index, node) in nodes.iter().enumerate().skip(1) {
        let parent = node.parent as usize;
        if parent >= index
            || !crate::popup::parent_allowed(
                node.kind & !DUNIT_UI_DISABLED,
                nodes[parent].kind & !DUNIT_UI_DISABLED,
            )
        {
            return Err(invalid());
        }
        depths[index] = depths[parent] + 1;
        if depths[index] >= MAX_VIEW_DEPTH {
            return Err(invalid());
        }
    }
    let mut root = None;
    for index in (0..nodes.len()).rev() {
        let node = &nodes[index];
        let kind = node.kind & !DUNIT_UI_DISABLED;
        let disabled = node.kind & DUNIT_UI_DISABLED != 0;
        if disabled
            && !matches!(
                kind,
                DUNIT_UI_BUTTON
                    | DUNIT_UI_CHECKBOX
                    | DUNIT_UI_CHECKBOX_CHECKED
                    | DUNIT_UI_SWITCH
                    | DUNIT_UI_SWITCH_CHECKED
                    | DUNIT_UI_TOGGLE_BUTTON
                    | DUNIT_UI_TOGGLE_BUTTON_CHECKED
                    | DUNIT_UI_RADIO
                    | DUNIT_UI_RADIO_CHECKED
                    | DUNIT_UI_INPUT
                    | DUNIT_UI_TEXTAREA
                    | DUNIT_UI_SLIDER
                    | DUNIT_UI_VERTICAL_SLIDER
                    | DUNIT_UI_SPINBOX
                    | DUNIT_UI_PROGRESS
                    | DUNIT_UI_BUSY
                    | DUNIT_UI_MENU
                    | DUNIT_UI_SELECT
                    | DUNIT_UI_MENU_ITEM
                    | DUNIT_UI_SUBMENU
                    | DUNIT_UI_OPTION
                    | DUNIT_UI_OPTION_SELECTED
                    | DUNIT_UI_TAB
                    | DUNIT_UI_TAB_SELECTED
                    | DUNIT_UI_LIST
                    | DUNIT_UI_TREE
                    | DUNIT_UI_TABLE
            )
        {
            return Err(invalid());
        }
        let key = keys[index].take().ok_or_else(invalid)?;
        let mut extra = extras[index].take().map(|boxed| *boxed).unwrap_or_default();
        extra.items.reverse();
        extra.options.reverse();
        let mut nested = core::mem::take(&mut children[index]);
        nested.reverse();
        if crate::popup::descriptor(kind) {
            if index == 0 {
                return Err(invalid());
            }
            crate::popup::bind(
                node,
                key,
                extra,
                crate::popup::extra(&mut extras, node.parent as usize),
                action,
                nested,
            )?;
            continue;
        }
        let mut view = match kind {
            DUNIT_UI_SCROLL
            | DUNIT_UI_SCROLL_HORIZONTAL
            | DUNIT_UI_SCROLL_BOTH
            | DUNIT_UI_TABS
            | DUNIT_UI_TOOLBAR
            | DUNIT_UI_TOOLBAR_SEPARATOR
            | DUNIT_UI_STATUS_BAR
            | DUNIT_UI_DIALOG
            | DUNIT_UI_MESSAGE_BOX
            | DUNIT_UI_MODAL => crate::workspace::view(node, key, nested, &mut extra)?,
            DUNIT_UI_LIST | DUNIT_UI_TREE | DUNIT_UI_TABLE => crate::collection::view(
                node,
                index as u32,
                key,
                user,
                collections.ok_or_else(invalid)?,
                collection.ok_or_else(invalid)?,
                renamed.ok_or_else(invalid)?,
            )?,
            DUNIT_UI_TEXT => View::text(key, text(node.text, node.text_len)?),
            DUNIT_UI_BUTTON => {
                View::button(key, text(node.text, node.text_len)?, action(node.message))
            }
            DUNIT_UI_CHECKBOX | DUNIT_UI_CHECKBOX_CHECKED => View::checkbox(
                key,
                text(node.text, node.text_len)?,
                kind == DUNIT_UI_CHECKBOX_CHECKED,
                action(node.message),
            ),
            DUNIT_UI_SWITCH | DUNIT_UI_SWITCH_CHECKED => View::switch(
                key,
                text(node.text, node.text_len)?,
                kind == DUNIT_UI_SWITCH_CHECKED,
                action(node.message),
            ),
            DUNIT_UI_TOGGLE_BUTTON | DUNIT_UI_TOGGLE_BUTTON_CHECKED => View::toggle_button(
                key,
                text(node.text, node.text_len)?,
                kind == DUNIT_UI_TOGGLE_BUTTON_CHECKED,
                action(node.message),
            ),
            DUNIT_UI_RADIO | DUNIT_UI_RADIO_CHECKED => {
                let parent = nodes.get(node.parent as usize).ok_or_else(invalid)?;
                if !matches!(
                    parent.kind,
                    DUNIT_UI_RADIO_GROUP_COLUMN | DUNIT_UI_RADIO_GROUP_ROW
                ) {
                    return Err(invalid());
                }
                View::radio(
                    key,
                    text(node.text, node.text_len)?,
                    keys[node.parent as usize]
                        .as_ref()
                        .ok_or_else(invalid)?
                        .clone(),
                    kind == DUNIT_UI_RADIO_CHECKED,
                    action(node.message),
                )
            }
            DUNIT_UI_COLUMN | DUNIT_UI_RADIO_GROUP_COLUMN => View::column(key, nested),
            DUNIT_UI_ROW | DUNIT_UI_RADIO_GROUP_ROW => View::row(key, nested),
            DUNIT_UI_STACK => View::stack(key, nested),
            DUNIT_UI_GRID => View::grid(key, nested),
            DUNIT_UI_MENU_BAR => View::menu_bar(key, nested),
            DUNIT_UI_MENU => View::menu(
                key,
                text(node.text, node.text_len)?,
                core::mem::take(&mut extra.items),
            ),
            DUNIT_UI_SELECT => crate::popup::select(
                key,
                text(node.text, node.text_len)?,
                core::mem::take(&mut extra.options),
                action,
            )?,
            DUNIT_UI_INPUT | DUNIT_UI_TEXTAREA => crate::text::view(
                node,
                index as u32,
                key,
                text(node.text, node.text_len)?,
                user,
                inputs.ok_or_else(invalid)?,
                action,
                edit.ok_or_else(invalid)?,
            )?,
            DUNIT_UI_SLIDER
            | DUNIT_UI_VERTICAL_SLIDER
            | DUNIT_UI_SPINBOX
            | DUNIT_UI_PROGRESS
            | DUNIT_UI_BUSY
            | DUNIT_UI_SPLITTER
            | DUNIT_UI_VERTICAL_SPLITTER => crate::number::view(
                node,
                index as u32,
                key,
                user,
                numbers.ok_or_else(invalid)?,
                numeric.ok_or_else(invalid)?,
                nested,
            )?,
            _ => return Err(invalid()),
        }
        .disabled(disabled);
        if let Some(items) = extra.context {
            view = view.context_menu(items);
        }
        if let Some(tooltip) = extra.tooltip {
            view = view.tooltip(tooltip);
        }
        if let Some(mnemonic) = extra.mnemonic {
            view = view.menu_mnemonic(mnemonic);
        }
        if let Some(decorate) = decorate {
            let mut appearance = DunitUiAppearanceV1 {
                classes: core::ptr::null(),
                classes_len: 0,
            };
            let result = decorate(user, index as u32, &mut appearance);
            if result != 0 {
                return Err(if result < 0 {
                    dunit::Error(result as isize)
                } else {
                    invalid()
                });
            }
            for class in text(appearance.classes, appearance.classes_len)?.split_ascii_whitespace()
            {
                view = view.class(class);
            }
        }
        if index == 0 {
            root = Some(view);
        } else {
            children[node.parent as usize].push(view);
        }
    }
    let root = root.ok_or_else(invalid)?;
    root.validate().map_err(|_| invalid())?;
    Ok(root)
}
