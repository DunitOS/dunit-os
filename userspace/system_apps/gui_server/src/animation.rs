//! Native, bounded animation sampling. Scripts submit descriptors once; frames
//! use monotonic milliseconds and fixed-point arithmetic without invoking Lua.

pub const UNIT: u32 = 65_535;
pub const CURVE_SAMPLES: usize = 33;
pub const MAX_DURATION_MS: u32 = 60_000;
const MAX_COORD: i32 = 1_048_576;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Duration,
    Curve,
    Geometry,
}

/// The 33 samples include both endpoints. Overshoot is bounded to one unit in
/// either direction, allowing spring curves without unbounded surface geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Easing {
    samples: [i32; CURVE_SAMPLES],
}

impl Easing {
    pub fn new(samples: [i32; CURVE_SAMPLES]) -> Result<Self, Error> {
        if samples[0] != 0
            || samples[CURVE_SAMPLES - 1] != UNIT as i32
            || samples
                .iter()
                .any(|&v| v < -(UNIT as i32) || v > 2 * UNIT as i32)
        {
            return Err(Error::Curve);
        }
        Ok(Self { samples })
    }
    pub fn linear() -> Self {
        Self::builtin(0)
    }
    pub fn ease_out() -> Self {
        Self::builtin(1)
    }
    /// A critically bounded, damped oscillation; no floating point or runtime
    /// transcendental functions are needed in the compositor.
    pub fn spring() -> Self {
        Self::builtin(2)
    }
    fn builtin(kind: u8) -> Self {
        let mut samples = [0; CURVE_SAMPLES];
        for (i, v) in samples.iter_mut().enumerate() {
            let t = i as i64 * UNIT as i64 / (CURVE_SAMPLES - 1) as i64;
            let u = UNIT as i64;
            *v = match kind {
                1 => (u - (u - t) * (u - t) / u) as i32,
                // Piecewise Hermite-like damped overshoot with exact endpoints.
                2 => {
                    let knots = [0i64, 81_918, 61_439, 66_559, 65_535];
                    let segment = (i / 8).min(3);
                    let local = (i - segment * 8) as i64 * u / 8;
                    let smooth = local * local / u * (3 * u - 2 * local) / u;
                    (knots[segment] + (knots[segment + 1] - knots[segment]) * smooth / u) as i32
                }
                _ => t as i32,
            };
        }
        Self { samples }
    }
    pub fn sample(&self, fraction: u32) -> i32 {
        let p = fraction.min(UNIT) as u64 * (CURVE_SAMPLES - 1) as u64;
        let index = (p / UNIT as u64) as usize;
        if index == CURVE_SAMPLES - 1 {
            return self.samples[index];
        }
        let remainder = (p % UNIT as u64) as i64;
        (self.samples[index] as i64
            + (self.samples[index + 1] - self.samples[index]) as i64 * remainder / UNIT as i64)
            as i32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}
impl Rect {
    pub fn valid(self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= MAX_COORD
            && self.height <= MAX_COORD
            && self.x.abs_diff(0) <= MAX_COORD as u32
            && self.y.abs_diff(0) <= MAX_COORD as u32
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Right,
    Bottom,
    Left,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Fade,
    Slide,
    Scale,
    Spring,
    Genie(Edge),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub effect: Effect,
    duration_ms: u32,
    pub easing: Easing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub rect: Rect,
    pub opacity: u8,
}
impl Descriptor {
    pub fn new(effect: Effect, duration_ms: u32, easing: Easing) -> Result<Self, Error> {
        if duration_ms > MAX_DURATION_MS {
            return Err(Error::Duration);
        }
        Ok(Self {
            effect,
            duration_ms,
            easing,
        })
    }
    pub fn duration_ms(self) -> u32 {
        self.duration_ms
    }
    /// Fraction zero is the original window, one is the minimized endpoint.
    /// Genie additionally uses `GenieMap` to deform its surface within this rect.
    pub fn frame(self, source: Rect, target: Rect, fraction: u32) -> Result<Frame, Error> {
        if !source.valid() || !target.valid() {
            return Err(Error::Geometry);
        }
        let p = self.easing.sample(fraction) as i64;
        let mut rect = source;
        let opacity = match self.effect {
            Effect::Fade => (255 - p * 255 / UNIT as i64).clamp(0, 255) as u8,
            Effect::Slide => {
                rect.x = mix(source.x, target.x, p);
                rect.y = mix(source.y, target.y, p);
                255
            }
            Effect::Scale | Effect::Spring | Effect::Genie(_) => {
                rect = interpolate_rect(source, target, p);
                255
            }
        };
        Ok(Frame { rect, opacity })
    }
}

fn mix(a: i32, b: i32, p: i64) -> i32 {
    (a as i64 + (b as i64 - a as i64) * p / UNIT as i64) as i32
}
fn interpolate_rect(a: Rect, b: Rect, p: i64) -> Rect {
    Rect {
        x: mix(a.x, b.x, p),
        y: mix(a.y, b.y, p),
        width: mix(a.width, b.width, p).max(1),
        height: mix(a.height, b.height, p).max(1),
    }
}

/// A reversible native clock. Reversing samples the current position first and
/// preserves speed, so interrupted minimize/restore operations do not jump.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeline {
    start_ms: u64,
    from: u32,
    to: u32,
    duration_ms: u32,
}
impl Timeline {
    pub fn new(now_ms: u64, duration_ms: u32, minimizing: bool) -> Result<Self, Error> {
        if duration_ms > MAX_DURATION_MS {
            return Err(Error::Duration);
        }
        Ok(Self {
            start_ms: now_ms,
            from: if minimizing { 0 } else { UNIT },
            to: if minimizing { UNIT } else { 0 },
            duration_ms,
        })
    }
    pub fn sample(self, now_ms: u64) -> u32 {
        let elapsed = now_ms.saturating_sub(self.start_ms);
        if self.duration_ms == 0 || elapsed >= self.duration_ms as u64 {
            return self.to;
        }
        (self.from as i64
            + (self.to as i64 - self.from as i64) * elapsed as i64 / self.duration_ms as i64)
            as u32
    }
    pub fn finished(self, now_ms: u64) -> bool {
        self.duration_ms == 0 || now_ms.saturating_sub(self.start_ms) >= self.duration_ms as u64
    }
    pub fn minimizing(self) -> bool {
        self.to == UNIT
    }
    pub fn retarget(
        &mut self,
        now_ms: u64,
        full_duration_ms: u32,
        minimizing: bool,
    ) -> Result<(), Error> {
        if self.minimizing() != minimizing {
            return self.reverse(now_ms, full_duration_ms);
        }
        if full_duration_ms > MAX_DURATION_MS {
            return Err(Error::Duration);
        }
        let current = self.sample(now_ms);
        self.from = current;
        self.start_ms = now_ms;
        self.duration_ms =
            ((full_duration_ms as u64 * self.to.abs_diff(current) as u64 + UNIT as u64 - 1)
                / UNIT as u64) as u32;
        Ok(())
    }
    pub fn reverse(&mut self, now_ms: u64, full_duration_ms: u32) -> Result<(), Error> {
        if full_duration_ms > MAX_DURATION_MS {
            return Err(Error::Duration);
        }
        let current = self.sample(now_ms);
        self.to = if self.to == UNIT { 0 } else { UNIT };
        self.from = current;
        self.start_ms = now_ms;
        self.duration_ms =
            ((full_duration_ms as u64 * self.to.abs_diff(current) as u64 + UNIT as u64 - 1)
                / UNIT as u64) as u32;
        Ok(())
    }
}

/// Inverse scanline warp of a snapshot toward an arbitrary panel icon. The
/// edge-facing rows narrow before the distant rows, producing the genie neck.
/// Destinations are sampled backwards to avoid holes in software compositing.
#[derive(Clone, Copy, Debug)]
pub struct GenieMap {
    source: Rect,
    target: Rect,
    edge: Edge,
    fraction: u32,
    bounds: Rect,
}
impl GenieMap {
    pub fn new(source: Rect, target: Rect, edge: Edge, fraction: u32) -> Result<Self, Error> {
        if !source.valid() || !target.valid() {
            return Err(Error::Geometry);
        }
        let fraction = fraction.min(UNIT);
        let bounds = interpolate_rect(source, target, fraction as i64);
        Ok(Self {
            source,
            target,
            edge,
            fraction,
            bounds,
        })
    }
    pub fn bounds(self) -> Rect {
        self.bounds
    }
    /// Returns snapshot-relative integer pixel coordinates, or None outside the
    /// deformed silhouette. A zero-progress mapping is exactly pixel-identical.
    pub fn inverse(self, x: i32, y: i32) -> Option<(usize, usize)> {
        let r = self.bounds;
        let vertical = matches!(self.edge, Edge::Top | Edge::Bottom);
        let (axis, origin, extent, source_extent) = if vertical {
            (
                y as i64,
                r.y as i64,
                r.height as i64,
                self.source.height as i64,
            )
        } else {
            (
                x as i64,
                r.x as i64,
                r.width as i64,
                self.source.width as i64,
            )
        };
        if axis < origin || axis >= origin + extent {
            return None;
        }
        let along = axis - origin;
        let mut proximity = along * UNIT as i64 / extent;
        if matches!(self.edge, Edge::Top | Edge::Left) {
            proximity = UNIT as i64 - proximity;
        }
        let p = self.fraction as i64;
        // Zero at both temporal endpoints; maximal taper halfway through.
        let bend = 4 * p * (UNIT as i64 - p) / UNIT as i64;
        let taper = bend * proximity / UNIT as i64;
        let (cross_origin, cross_extent, target_origin, target_extent, source_cross, cross) =
            if vertical {
                (
                    r.x,
                    r.width,
                    self.target.x,
                    self.target.width,
                    self.source.width,
                    x,
                )
            } else {
                (
                    r.y,
                    r.height,
                    self.target.y,
                    self.target.height,
                    self.source.height,
                    y,
                )
            };
        let width = mix(cross_extent, target_extent.min(cross_extent), taper).max(1) as i64;
        let center = cross_origin as i64 + cross_extent as i64 / 2;
        let target_center = target_origin as i64 + target_extent as i64 / 2;
        let shifted_center = center + (target_center - center) * taper / UNIT as i64;
        // Clamp to the advertised damage bounds even for remote icon targets.
        let left = (shifted_center - width / 2).clamp(
            cross_origin as i64,
            cross_origin as i64 + cross_extent as i64 - width,
        );
        let cross = cross as i64 - left;
        if cross < 0 || cross >= width {
            return None;
        }
        let a = (along * source_extent / extent) as usize;
        let b = (cross * source_cross as i64 / width) as usize;
        Some(if vertical { (b, a) } else { (a, b) })
    }
}

pub fn progress(now: u64, start: u64, duration_ms: u32, ease_out: bool) -> u32 {
    let elapsed = now.saturating_sub(start);
    let linear = if duration_ms == 0 || elapsed >= duration_ms as u64 {
        255
    } else {
        (elapsed * 255 / duration_ms as u64) as u32
    };
    if ease_out {
        255 - (255 - linear) * (255 - linear) / 255
    } else {
        linear
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const WINDOW: Rect = Rect {
        x: -20,
        y: 10,
        width: 120,
        height: 80,
    };
    const ICON: Rect = Rect {
        x: 200,
        y: 300,
        width: 24,
        height: 24,
    };
    #[test]
    fn descriptors_reject_untrusted_resource_bounds() {
        assert_eq!(
            Descriptor::new(Effect::Fade, MAX_DURATION_MS + 1, Easing::linear()),
            Err(Error::Duration)
        );
        assert_eq!(Easing::new([0; CURVE_SAMPLES]), Err(Error::Curve));
        let mut curve = Easing::linear().samples;
        curve[4] = i32::MAX;
        assert_eq!(Easing::new(curve), Err(Error::Curve));
        assert!(!Rect {
            x: i32::MIN,
            ..WINDOW
        }
        .valid());
        assert!(!Rect { width: 0, ..WINDOW }.valid());
    }
    #[test]
    fn sampled_curves_and_spring_keep_endpoints() {
        for curve in [Easing::linear(), Easing::ease_out(), Easing::spring()] {
            assert_eq!(curve.sample(0), 0);
            assert_eq!(curve.sample(UNIT), UNIT as i32);
            assert_eq!(curve.sample(u32::MAX), UNIT as i32);
        }
        assert!(Easing::spring().sample(UNIT / 4) > UNIT as i32);
    }
    #[test]
    fn reversing_uses_current_position_and_remaining_distance() {
        let mut t = Timeline::new(100, 1000, true).unwrap();
        assert_eq!(t.sample(99), 0);
        let at = t.sample(500);
        t.reverse(500, 1000).unwrap();
        assert_eq!(t.sample(500), at);
        assert!(t.sample(600) < at);
        assert_eq!(t.sample(1000), 0);
        assert!(t.finished(1000));
        t.reverse(1000, 1000).unwrap();
        assert_eq!(t.sample(1000), 0);
        assert_eq!(t.sample(u64::MAX), UNIT);
        assert_eq!(Timeline::new(1, 0, false).unwrap().sample(1), 0);
    }
    #[test]
    fn native_effects_have_expected_endpoints() {
        for effect in [
            Effect::Fade,
            Effect::Slide,
            Effect::Scale,
            Effect::Spring,
            Effect::Genie(Edge::Bottom),
        ] {
            let d = Descriptor::new(effect, 240, Easing::linear()).unwrap();
            assert_eq!(
                d.frame(WINDOW, ICON, 0).unwrap(),
                Frame {
                    rect: WINDOW,
                    opacity: 255
                }
            );
            let end = d.frame(WINDOW, ICON, UNIT).unwrap();
            match effect {
                Effect::Fade => assert_eq!(end.opacity, 0),
                Effect::Slide => assert_eq!((end.rect.x, end.rect.y), (ICON.x, ICON.y)),
                _ => assert_eq!(end.rect, ICON),
            }
        }
    }
    #[test]
    fn genie_inverse_is_identity_then_bounded_for_every_edge() {
        for edge in [Edge::Top, Edge::Right, Edge::Bottom, Edge::Left] {
            let start = GenieMap::new(WINDOW, ICON, edge, 0).unwrap();
            for y in 0..WINDOW.height {
                for x in 0..WINDOW.width {
                    assert_eq!(
                        start.inverse(WINDOW.x + x, WINDOW.y + y),
                        Some((x as usize, y as usize))
                    );
                }
            }
            let middle = GenieMap::new(WINDOW, ICON, edge, UNIT / 2).unwrap();
            let r = middle.bounds();
            let mut holes = 0;
            let mut pixels = 0;
            for y in r.y..r.y + r.height {
                for x in r.x..r.x + r.width {
                    if let Some((sx, sy)) = middle.inverse(x, y) {
                        assert!(sx < WINDOW.width as usize && sy < WINDOW.height as usize);
                        pixels += 1;
                    } else {
                        holes += 1;
                    }
                }
            }
            assert!(
                holes > 0 && pixels > 0,
                "genie must actually deform the silhouette"
            );
            assert_eq!(middle.inverse(r.x - 1, r.y - 1), None);
            let end = GenieMap::new(WINDOW, ICON, edge, UNIT).unwrap();
            assert_eq!(end.bounds(), ICON);
            assert_eq!(end.inverse(ICON.x, ICON.y), Some((0, 0)));
        }
    }
    #[test]
    fn dropped_frames_do_not_extend_duration() {
        assert_eq!(progress(1000, 1000, 140, false), 0);
        assert_eq!(progress(1070, 1000, 140, false), 127);
        assert_eq!(progress(1200, 1000, 140, false), 255);
        assert_eq!(progress(1000, 1000, 0, true), 255);
        assert_eq!(progress(999, 1000, 140, false), 0);
        assert_eq!(progress(u64::MAX, 0, 2000, true), 255);
    }
    #[test]
    fn easing_preserves_endpoints_and_is_monotonic() {
        let mut previous = 0;
        for elapsed in 0..=180 {
            let value = progress(elapsed, 0, 180, true);
            assert!(value >= previous && value <= 255);
            previous = value;
        }
        assert_eq!(progress(0, 0, 180, true), 0);
        assert_eq!(previous, 255);
    }
}
