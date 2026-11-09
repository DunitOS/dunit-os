//! Configure real SDK editors and map proposed values into the model's message.
use super::*;
pub(super) unsafe fn view<M: Clone + 'static>(
    node: &DunitUiNodeV1,
    index: u32,
    key: String,
    value: String,
    user: *mut c_void,
    text: &DunitUiTextV1,
    action: fn(u64) -> M,
    edit: fn(u64, String) -> M,
) -> dunit::Result<View<M>> {
    let mut config = DunitUiInputV1 {
        label: key.as_ptr(),
        label_len: key.len(),
        placeholder: core::ptr::null(),
        placeholder_len: 0,
        max_chars: InputOptions::default().max_chars,
        flags: 0,
        reserved: 0,
        submit: 0,
    };
    if let Some(configure) = text.configure {
        let result = configure(user, index, &mut config);
        if result != 0 {
            return Err(if result < 0 {
                dunit::Error(result as isize)
            } else {
                invalid()
            });
        }
    }
    let known = DUNIT_UI_INPUT_READ_ONLY
        | DUNIT_UI_INPUT_PASSWORD
        | DUNIT_UI_INPUT_SOFT_WRAP
        | DUNIT_UI_INPUT_SUBMIT;
    let multiline = node.kind & !DUNIT_UI_DISABLED == DUNIT_UI_TEXTAREA;
    if config.flags & !known != 0
        || config.reserved != 0
        || (multiline && config.flags & DUNIT_UI_INPUT_SUBMIT != 0)
    {
        return Err(invalid());
    }
    let label = tree::text(config.label, config.label_len)?;
    let options = InputOptions {
        placeholder: tree::text(config.placeholder, config.placeholder_len)?,
        read_only: config.flags & DUNIT_UI_INPUT_READ_ONLY != 0,
        password: config.flags & DUNIT_UI_INPUT_PASSWORD != 0,
        soft_wrap: config.flags & DUNIT_UI_INPUT_SOFT_WRAP != 0,
        max_chars: config.max_chars,
    };
    let id = node.message;
    let input = if multiline {
        View::text_area(key, label, value, |s| s, options)
    } else {
        View::input(key, label, value, |s| s, options)
    };
    let input = input.map_messages(move |value| edit(id, value));
    Ok(if config.flags & DUNIT_UI_INPUT_SUBMIT != 0 {
        input.on_submit(action(config.submit))
    } else {
        input
    })
}
