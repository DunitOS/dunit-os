//! Native component property tracks; no script or stylesheet evaluation occurs
//! while sampling a frame.
use crate::{
    animation::{Easing, UNIT},
    lua_policy::{Animation, Property},
};
use alloc::vec::Vec;
type Rect = (i32, i32, i32, i32);
#[derive(Clone)]
struct Track {
    background: bool,
    target: u32,
    property: Property,
    start: u64,
    duration: u32,
    from: i64,
    to: i64,
    easing: Easing,
}
impl Track {
    fn value(&self, now: u64) -> i64 {
        let t = if self.duration == 0 {
            UNIT
        } else {
            (now.saturating_sub(self.start) * UNIT as u64 / self.duration as u64).min(UNIT as u64)
                as u32
        };
        let t = self.easing.sample(t) as i64;
        if self.property == Property::Color {
            let mut out = 0;
            for shift in [0, 8, 16, 24] {
                let a = (self.from >> shift) & 255;
                let b = (self.to >> shift) & 255;
                out |= (a + (b - a) * t / UNIT as i64).clamp(0, 255) << shift;
            }
            out
        } else {
            self.from + (self.to - self.from) * t / UNIT as i64
        }
    }
}
#[derive(Clone, Default)]
pub struct Tracks {
    tracks: Vec<Track>,
}
pub struct Visual {
    pub background: Option<u32>,
    pub rect: Rect,
    pub opacity: u32,
    pub color: Option<u32>,
    pub radius: Option<i32>,
    pub shadow: i32,
    pub blur: i32,
}
impl Tracks {
    pub fn animating(&self, now:u64)->bool {
        self.tracks.iter().any(|t|now.saturating_sub(t.start)<t.duration as u64)
    }
    pub fn submit(&mut self, a: &Animation, now: u64) -> Result<(), &'static str> {
        let property = a
            .property
            .clone()
            .ok_or("component animation requires a scalar property")?;
        let easing = Easing::new(a.curve).map_err(|_| "invalid component curve")?;
        let index = self
            .tracks
            .iter()
            .position(|t| !t.background && t.target == a.target && t.property == property);
        if index.is_none() && self.tracks.len() >= 768 {
            return Err("component animation budget exceeded");
        }
        let track = Track {
            background: false,
            target: a.target,
            property,
            start: now,
            duration: a.duration_ms,
            from: a.from,
            to: a.to,
            easing,
        };
        if let Some(i) = index {
            self.tracks[i] = track;
        } else {
            self.tracks.push(track);
        }
        Ok(())
    }
    pub fn since(&self, target: u32, property: Property, start: u64) -> bool {
        self.tracks
            .iter()
            .any(|t| t.target == target && t.property == property && t.start >= start)
    }
    pub fn active(&self, target: u32, now: u64) -> bool {
        self.tracks
            .iter()
            .any(|t| t.target == target && now.saturating_sub(t.start) < t.duration as u64)
    }
    pub fn has(&self, target: u32, property: Property) -> bool {
        self.tracks
            .iter()
            .any(|t| !t.background && t.target == target && t.property == property)
    }
    pub fn transition(
        &mut self,
        target: u32,
        base: Rect,
        old: &dunit_style::Style,
        next: &dunit_style::Style,
        transitions: &[dunit_style::Transition],
        now: u64,
        foreground: u32,
    ) {
        let current = self.sample(target, base, now);
        let rgba = |style: &dunit_style::Style, key: &str, fallback: u32| {
            style
                .color(key)
                .map(|c| {
                    ((c.a as u32) << 24) | ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32
                })
                .unwrap_or(fallback) as i64
        };
        for transition in transitions {
            let background = transition.property == "background";
            let (property, from, to) = match transition.property.as_str() {
                "background" => (
                    Property::Color,
                    current
                        .background
                        .map(|c| c as i64)
                        .unwrap_or_else(|| rgba(old, "background", 0)),
                    rgba(next, "background", 0),
                ),
                "color" => (
                    Property::Color,
                    current
                        .color
                        .map(|c| c as i64)
                        .unwrap_or_else(|| rgba(old, "color", foreground)),
                    rgba(next, "color", foreground),
                ),
                "opacity" => (
                    Property::Opacity,
                    (current.opacity as i64) * UNIT as i64 / 255,
                    (next.number("opacity").unwrap_or(255.0).clamp(0.0, 255.0) as i64)
                        * UNIT as i64
                        / 255,
                ),
                "corner-radius" => (
                    Property::CornerRadius,
                    current
                        .radius
                        .unwrap_or_else(|| old.number("corner-radius").unwrap_or(0.0) as i32)
                        as i64,
                    next.number("corner-radius")
                        .unwrap_or(0.0)
                        .clamp(0.0, 1024.0) as i64,
                ),
                _ => continue,
            };
            let mut curve = [0; 33];
            for (i, value) in curve.iter_mut().enumerate() {
                *value = (transition.easing.apply(i as f32 / 32.0) * UNIT as f32 + 0.5) as i32;
            }
            let animation = Animation {
                window_id: 0,
                target,
                property: Some(property.clone()),
                effect: crate::lua_policy::Effect::Fade,
                duration_ms: transition.duration_ms.clamp(0.0, 2000.0) as u32,
                from,
                to,
                curve,
            };
            if background {
                let Ok(easing) = Easing::new(curve) else {
                    continue;
                };
                let index = self
                    .tracks
                    .iter()
                    .position(|t| t.target == target && t.background);
                let track = Track {
                    background: true,
                    target,
                    property,
                    start: now,
                    duration: animation.duration_ms,
                    from,
                    to,
                    easing,
                };
                if let Some(i) = index {
                    self.tracks[i] = track;
                } else if self.tracks.len() < 768 {
                    self.tracks.push(track);
                }
            } else {
                let _ = self.submit(&animation, now);
            }
        }
    }
    pub fn sample(&self, target: u32, base: Rect, now: u64) -> Visual {
        let mut v = Visual {
            background: None,
            rect: base,
            opacity: 255,
            color: None,
            radius: None,
            shadow: 0,
            blur: 0,
        };
        let mut dx = 0;
        let mut dy = 0;
        let mut scale = UNIT as i64;
        for t in self.tracks.iter().filter(|t| t.target == target) {
            let value = t.value(now);
            if t.background {
                v.background = Some(value as u32);
                continue;
            }
            match t.property {
                Property::Opacity => v.opacity = (value * 255 / UNIT as i64).clamp(0, 255) as u32,
                Property::X => v.rect.0 = value.clamp(-16384, 16384) as i32,
                Property::Y => v.rect.1 = value.clamp(-16384, 16384) as i32,
                Property::Width => v.rect.2 = value.clamp(1, 16384) as i32,
                Property::Height => v.rect.3 = value.clamp(1, 16384) as i32,
                Property::TranslationX => dx = value.clamp(-16384, 16384) as i32,
                Property::TranslationY => dy = value.clamp(-16384, 16384) as i32,
                Property::Scale => scale = value.clamp(0, 16 * UNIT as i64),
                Property::Color => v.color = Some(value as u32),
                Property::CornerRadius => v.radius = Some(value.clamp(0, 1024) as i32),
                Property::Shadow => v.shadow = value.clamp(0, 64) as i32,
                Property::Blur => v.blur = value.clamp(0, 32) as i32,
            }
        }
        v.rect.0 += dx;
        v.rect.1 += dy;
        v.rect.2 = (v.rect.2 as i64 * scale / UNIT as i64).clamp(0, 16384) as i32;
        v.rect.3 = (v.rect.3 as i64 * scale / UNIT as i64).clamp(0, 16384) as i32;
        v
    }
}
