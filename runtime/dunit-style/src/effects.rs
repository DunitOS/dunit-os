//! Typed, bounded effect values shared by validation and painting.
use crate::{Color, Value};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub x: f32,
    pub y: f32,
    pub spread: f32,
    pub color: Color,
}
impl Shadow {
    /// DSS: `shadow: <x> <y> <spread> <color>`; spread is a layered falloff,
    /// not a Gaussian blur radius. `none` is handled by the caller.
    pub fn parse(value: &Value) -> Option<Self> {
        let [x, y, spread, color] = value.as_list()? else {
            return None;
        };
        let (x, y, spread) = (x.as_number()?, y.as_number()?, spread.as_number()?);
        let shadow = Self {
            x,
            y,
            spread,
            color: color.as_color()?,
        };
        shadow.is_valid().then_some(shadow)
    }
    /// Bounds shared by DSS decoding and direct surface callers.
    pub fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.spread.is_finite()
            && self.x.abs() <= 1024.
            && self.y.abs() <= 1024.
            && (0. ..=64.).contains(&self.spread)
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    pub horizontal: bool,
    pub start: Color,
    pub end: Color,
}
impl Gradient {
    /// DSS: `background-gradient: vertical|horizontal <start> <end>`.
    pub fn parse(value: &Value) -> Option<Self> {
        let [axis, start, end] = value.as_list()? else {
            return None;
        };
        let horizontal = match axis.as_keyword()? {
            "horizontal" => true,
            "vertical" => false,
            _ => return None,
        };
        Some(Self {
            horizontal,
            start: start.as_color()?,
            end: end.as_color()?,
        })
    }
}
