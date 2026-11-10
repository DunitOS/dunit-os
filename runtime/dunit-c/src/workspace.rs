//! Structural descriptions compose the SDK workspace widgets and their messages.
use super::*;
pub(super) fn container(kind: u32) -> bool {
    matches!(
        kind,
        DUNIT_UI_SCROLL
            | DUNIT_UI_SCROLL_HORIZONTAL
            | DUNIT_UI_SCROLL_BOTH
            | DUNIT_UI_TAB
            | DUNIT_UI_TAB_SELECTED
            | DUNIT_UI_TOOLBAR
            | DUNIT_UI_DIALOG
            | DUNIT_UI_MODAL
            | DUNIT_UI_SPLITTER
            | DUNIT_UI_VERTICAL_SPLITTER
    )
}
pub(super) unsafe fn view<M: Clone + 'static>(
    node: &DunitUiNodeV1,
    key: String,
    mut children: Vec<View<M>>,
    extra: &mut crate::popup::Extra<M>,
) -> dunit::Result<View<M>> {
    let label = tree::text(node.text, node.text_len)?;
    Ok(match node.kind & !DUNIT_UI_DISABLED {
        DUNIT_UI_SCROLL | DUNIT_UI_SCROLL_HORIZONTAL | DUNIT_UI_SCROLL_BOTH => {
            View::scroll(key, label, children).scroll_axes(match node.kind {
                DUNIT_UI_SCROLL_HORIZONTAL => ScrollAxes::Horizontal,
                DUNIT_UI_SCROLL_BOTH => ScrollAxes::Both,
                _ => ScrollAxes::Vertical,
            })
        }
        DUNIT_UI_TOOLBAR => View::toolbar(key, label, children),
        DUNIT_UI_TOOLBAR_SEPARATOR => View::toolbar_separator(key),
        DUNIT_UI_STATUS_BAR => View::status_bar(key.clone(), key, label),
        DUNIT_UI_TABS => {
            extra.pages.reverse();
            let mut selected = None;
            let mut pages = Vec::new();
            for (page, active) in core::mem::take(&mut extra.pages) {
                if active && selected.replace(page.key.clone()).is_some() {
                    return Err(invalid());
                }
                pages.push(page);
            }
            View::tabs(key, label, selected.as_deref().ok_or_else(invalid)?, pages)
        }
        DUNIT_UI_DIALOG | DUNIT_UI_MESSAGE_BOX => {
            extra.actions.reverse();
            let actions = core::mem::take(&mut extra.actions);
            let default = extra.default.as_deref();
            let cancel = extra.cancel.take();
            if node.kind == DUNIT_UI_MESSAGE_BOX {
                View::message_box(
                    key,
                    label,
                    extra.body.take().ok_or_else(invalid)?,
                    actions,
                    default,
                    cancel,
                )
            } else {
                if children.len() != 1 {
                    return Err(invalid());
                }
                View::dialog(key, label, children.remove(0), actions, default, cancel)
            }
        }
        DUNIT_UI_MODAL => {
            if !(1..=2).contains(&children.len()) {
                return Err(invalid());
            }
            let dialog = if children.len() == 2 {
                children.pop()
            } else {
                None
            };
            View::modal(key, children.remove(0), dialog)
        }
        _ => return Err(invalid()),
    })
}
