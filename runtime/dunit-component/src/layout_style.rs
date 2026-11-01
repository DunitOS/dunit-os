//! Layout-affecting DSS is resolved after interaction states, before measurement.
use crate::{Result, UiError};
use dunit_style::{Style, Value};
use dunit_ui::{Align, Attrs, Sizing};
pub(crate) fn sizing(value: &Value) -> Option<Sizing> {
    if let Some(n) = value.as_number() {
        return (n.is_finite() && (0. ..=65536.).contains(&n)).then_some(Sizing::Px(n));
    }
    let s = value.as_keyword()?;
    match s {
        "auto" => Some(Sizing::Auto),
        "grow" | "fill" => Some(Sizing::Grow(1.)),
        _ => {
            let n = Value::parse_word(s.strip_suffix('%')?).as_number()?;
            (n.is_finite() && (0. ..=100.).contains(&n)).then_some(Sizing::Pct(n / 100.))
        }
    }
}
fn align(s: &str) -> Option<Align> {
    Some(match s {
        "start" => Align::Start,
        "end" => Align::End,
        "center" => Align::Center,
        "stretch" => Align::Stretch,
        "space-between" => Align::SpaceBetween,
        "space-around" => Align::SpaceAround,
        _ => return None,
    })
}
pub(crate) fn apply(attrs: &mut Attrs, style: &Style) -> Result<()> {
    for (key, target) in [("width", &mut attrs.width), ("height", &mut attrs.height)] {
        if let Some(v) = style.get(key) {
            *target = sizing(v).ok_or(UiError::InvalidLayout)?;
        }
    }
    for (key, target) in [
        ("min-width", &mut attrs.min_w),
        ("min-height", &mut attrs.min_h),
        ("max-width", &mut attrs.max_w),
        ("max-height", &mut attrs.max_h),
    ] {
        if let Some(v) = style.get(key) {
            *target = if v.as_keyword() == Some("none") {
                None
            } else {
                Some(
                    v.as_number()
                        .filter(|n| n.is_finite() && (0. ..=65536.).contains(n))
                        .ok_or(UiError::InvalidLayout)?,
                )
            };
        }
    }
    if attrs.min_w.zip(attrs.max_w).is_some_and(|(a, b)| a > b)
        || attrs.min_h.zip(attrs.max_h).is_some_and(|(a, b)| a > b)
    {
        return Err(UiError::InvalidLayout);
    }
    for (key, target) in [("flex-grow", &mut attrs.grow), ("gap", &mut attrs.spacing)] {
        if let Some(v) = style.get(key) {
            *target = v
                .as_number()
                .filter(|n| n.is_finite() && (0. ..=65536.).contains(n))
                .ok_or(UiError::InvalidLayout)?;
        }
    }
    // Explicit zero disables the implicit flex weight of typed/DSS grow sizing
    // as well. Keep Grow(0) rather than Auto so cross-axis fill is preserved.
    if style.number("flex-grow") == Some(0.) {
        if matches!(attrs.width, Sizing::Grow(_)) {
            attrs.width = Sizing::Grow(0.);
        }
        if matches!(attrs.height, Sizing::Grow(_)) {
            attrs.height = Sizing::Grow(0.);
        }
    }
    for (key, target) in [
        ("justify-content", &mut attrs.main_align),
        ("align-items", &mut attrs.cross_align),
    ] {
        if let Some(v) = style.get(key) {
            *target = align(v.as_keyword().ok_or(UiError::InvalidLayout)?)
                .ok_or(UiError::InvalidLayout)?;
        }
    }
    if let Some(v) = style.get("grid-columns") {
        let n = v
            .as_number()
            .filter(|n| n.is_finite() && *n >= 1. && *n <= 4096. && *n as u32 as f32 == *n)
            .ok_or(UiError::InvalidLayout)?;
        attrs.columns = n as u32;
    }
    Ok(())
}
