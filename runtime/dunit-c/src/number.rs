//! Integer ranges and events go directly to the SDK's existing control views.
use super::*;
pub(super) unsafe fn view<M: Clone + 'static>(
    node: &DunitUiNodeV1,
    index: u32,
    key: String,
    user: *mut c_void,
    callbacks: &DunitUiNumbersV1,
    changed: fn(u64, i64) -> M,
) -> dunit::Result<View<M>> {
    let mut config = DunitUiNumberV1 {
        label: key.as_ptr(),
        label_len: key.len(),
        min: 0,
        max: 100,
        step: 1,
        page_step: 10,
        value: 0,
        flags: 0,
        reserved: 0,
    };
    if let Some(configure) = callbacks.configure {
        let result = configure(user, index, &mut config);
        if result != 0 {
            return Err(if result < 0 {
                dunit::Error(result as isize)
            } else {
                invalid()
            });
        }
    }
    let kind = node.kind & !DUNIT_UI_DISABLED;
    let allowed = match kind {
        DUNIT_UI_SPINBOX => DUNIT_UI_NUMBER_READ_ONLY,
        DUNIT_UI_BUSY => DUNIT_UI_NUMBER_RUNNING,
        _ => 0,
    };
    if config.reserved != 0 || config.flags & !allowed != 0 {
        return Err(invalid());
    }
    let label = tree::text(config.label, config.label_len)?;
    if kind == DUNIT_UI_PROGRESS {
        return Ok(View::progress(
            key,
            label,
            config.value,
            ProgressOptions::new(config.min, config.max),
        ));
    }
    if kind == DUNIT_UI_BUSY {
        return Ok(View::busy(
            key,
            label,
            config.flags & DUNIT_UI_NUMBER_RUNNING != 0,
        ));
    }
    if callbacks.changed.is_none() {
        return Err(invalid());
    }
    let options = SliderOptions {
        min: config.min,
        max: config.max,
        step: config.step,
        page_step: config.page_step,
    };
    let view = match kind {
        DUNIT_UI_SLIDER => View::slider(key, label, config.value, options, |value| value),
        DUNIT_UI_VERTICAL_SLIDER => {
            View::vertical_slider(key, label, config.value, options, |value| value)
        }
        DUNIT_UI_SPINBOX => View::spinbox(
            key,
            label,
            config.value,
            SpinboxOptions {
                range: options,
                read_only: config.flags & DUNIT_UI_NUMBER_READ_ONLY != 0,
            },
            |value| value,
        ),
        _ => return Err(invalid()),
    };
    let id = node.message;
    Ok(view.map_messages(move |value| changed(id, value)))
}
