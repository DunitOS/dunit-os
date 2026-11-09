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
fn radio_group(kind: u32) -> bool {
    matches!(kind, DUNIT_UI_RADIO_GROUP_COLUMN | DUNIT_UI_RADIO_GROUP_ROW)
}
pub(super) unsafe fn convert(nodes: &[DunitUiNodeV1]) -> dunit::Result<View<u64>> {
    convert_decorated(nodes, core::ptr::null_mut(), None)
}
pub(super) unsafe fn convert_decorated(
    nodes: &[DunitUiNodeV1],
    user: *mut c_void,
    decorate: Option<DunitUiDecorateV1>,
) -> dunit::Result<View<u64>> {
    if nodes.is_empty() || nodes.len() > MAX_VIEW_NODES || nodes[0].parent != DUNIT_UI_ROOT {
        return Err(invalid());
    }
    let mut children: Vec<Vec<View<u64>>> = vec![Vec::new(); nodes.len()];
    let mut depths = vec![0; nodes.len()];
    for (index, node) in nodes.iter().enumerate().skip(1) {
        let parent = node.parent as usize;
        if parent >= index
            || !(matches!(
                nodes[parent].kind,
                DUNIT_UI_COLUMN | DUNIT_UI_ROW | DUNIT_UI_STACK | DUNIT_UI_GRID
            ) || radio_group(nodes[parent].kind))
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
            )
        {
            return Err(invalid());
        }
        let key = text(node.key, node.key_len)?;
        let mut nested = core::mem::take(&mut children[index]);
        nested.reverse();
        let mut view = match kind {
            DUNIT_UI_TEXT => View::text(key, text(node.text, node.text_len)?),
            DUNIT_UI_BUTTON => View::button(key, text(node.text, node.text_len)?, node.message),
            DUNIT_UI_CHECKBOX | DUNIT_UI_CHECKBOX_CHECKED => View::checkbox(
                key,
                text(node.text, node.text_len)?,
                kind == DUNIT_UI_CHECKBOX_CHECKED,
                node.message,
            ),
            DUNIT_UI_SWITCH | DUNIT_UI_SWITCH_CHECKED => View::switch(
                key,
                text(node.text, node.text_len)?,
                kind == DUNIT_UI_SWITCH_CHECKED,
                node.message,
            ),
            DUNIT_UI_TOGGLE_BUTTON | DUNIT_UI_TOGGLE_BUTTON_CHECKED => View::toggle_button(
                key,
                text(node.text, node.text_len)?,
                kind == DUNIT_UI_TOGGLE_BUTTON_CHECKED,
                node.message,
            ),
            DUNIT_UI_RADIO | DUNIT_UI_RADIO_CHECKED => {
                let parent = nodes.get(node.parent as usize).ok_or_else(invalid)?;
                if !radio_group(parent.kind) {
                    return Err(invalid());
                }
                View::radio(
                    key,
                    text(node.text, node.text_len)?,
                    text(parent.key, parent.key_len)?,
                    kind == DUNIT_UI_RADIO_CHECKED,
                    node.message,
                )
            }
            DUNIT_UI_COLUMN | DUNIT_UI_RADIO_GROUP_COLUMN => View::column(key, nested),
            DUNIT_UI_ROW | DUNIT_UI_RADIO_GROUP_ROW => View::row(key, nested),
            DUNIT_UI_STACK => View::stack(key, nested),
            DUNIT_UI_GRID => View::grid(key, nested),
            _ => return Err(invalid()),
        }
        .disabled(disabled);
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
