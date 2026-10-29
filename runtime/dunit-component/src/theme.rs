//! Pure SDK theme construction; profiles and rules use the same token registry.
use crate::{style, Cascade, Color};

#[derive(Clone)]
pub struct Theme {
    pub tokens: style::Tokens,
    pub cascade: Cascade,
    pub background: Color,
}
impl Theme {
    pub fn parse(profile: &str, base: &str, overlay: &str) -> style::Result<Self> {
        let profile = style::parse_with_tokens(profile, &style::Tokens::default())?;
        let bad = |name: &str| style::StyleError::Unexpected {
            line: 1,
            what: alloc::format!("invalid SDK theme token ${}", name),
        };
        for name in ["window-bg", "surface", "text", "muted", "accent", "line"] {
            profile.tokens.color(name).ok_or_else(|| bad(name))?;
        }
        let background = profile.tokens.color("window-bg").unwrap();
        if background.a != 255 {
            return Err(bad("window-bg"));
        }
        for (name, min, max) in [
            ("font-size", 1., 512.),
            ("font-title", 1., 512.),
            ("space", 0., 1024.),
            ("space-small", 0., 1024.),
            ("radius", 0., 1024.),
            ("border", 0., 64.),
        ] {
            let value = profile.tokens.number(name).ok_or_else(|| bad(name))?;
            if !value.is_finite() || value < min || value > max {
                return Err(bad(name));
            }
        }
        let duration = profile
            .tokens
            .duration("motion-fast")
            .ok_or_else(|| bad("motion-fast"))?;
        if !duration.is_finite() || !(0. ..=60_000.).contains(&duration) {
            return Err(bad("motion-fast"));
        }
        let base = style::parse_with_tokens(base, &profile.tokens)?;
        let overlay = style::parse_with_tokens(overlay, &base.tokens)?;
        for sheet in [&profile, &base, &overlay] {
            for rule in &sheet.rules {
                for declaration in &rule.declarations {
                    let value = &declaration.value;
                    let valid_number = |min, max| {
                        value
                            .as_number()
                            .is_some_and(|v| v.is_finite() && v >= min && v <= max)
                    };
                    let valid = match declaration.property.as_str() {
                        "scrollbar-track"
                        | "scrollbar-thumb"
                        | "color"
                        | "background"
                        | "border-color"
                        | "indicator-background"
                        | "indicator-border-color"
                        | "indicator-mark-color"
                        | "placeholder-color"
                        | "selection-color"
                        | "caret-color" => value.as_color().is_some(),
                        "indicator-width"
                        | "indicator-height"
                        | "indicator-gap"
                        | "indicator-radius"
                        | "indicator-mark-size"
                        | "indicator-inset" => valid_number(0., 1024.),
                        "indicator-border-width" => valid_number(0., 64.),
                        "indicator-position" => valid_number(0., 1.),
                        "caret-width" => valid_number(0., 64.),
                        "scrollbar-width" | "scrollbar-min-thumb" | "scrollbar-radius" => {
                            valid_number(0., 1024.)
                        }
                        "scroll-step" => valid_number(0., 1024.),
                        "caret-blink" => value
                            .as_duration()
                            .is_some_and(|v| v.is_finite() && (0. ..=60_000.).contains(&v)),
                        "background-gradient" => {
                            value.as_keyword() == Some("none")
                                || style::effects::Gradient::parse(value).is_some()
                        }
                        "shadow" => {
                            value.as_keyword() == Some("none")
                                || style::effects::Shadow::parse(value).is_some()
                        }
                        "backdrop-blur" => valid_number(0., 64.),
                        "transition" => crate::motion::declarations(value).is_some(),
                        "clip" => matches!(value.as_keyword(), Some("rect" | "rounded")),
                        "font-size" => valid_number(1., 512.),
                        "radius" => valid_number(0., 1024.),
                        "border-width" => valid_number(0., 64.),
                        "padding" => {
                            let words = match value {
                                style::Value::List(words) => &words[..],
                                value => core::slice::from_ref(value),
                            };
                            (1..=4).contains(&words.len())
                                && words.iter().all(|v| {
                                    v.as_number().is_some_and(|v| {
                                        v.is_finite() && (0. ..=1024.).contains(&v)
                                    })
                                })
                        }
                        _ => true,
                    };
                    if !valid {
                        return Err(style::StyleError::Unexpected {
                            line: 1,
                            what: alloc::format!(
                                "invalid SDK style property {}",
                                declaration.property
                            ),
                        });
                    }
                }
            }
        }
        let tokens = profile.tokens.clone();
        let mut cascade = Cascade::new();
        cascade.push(profile).push(base).push(overlay);
        Ok(Self {
            tokens,
            cascade,
            background,
        })
    }
}
