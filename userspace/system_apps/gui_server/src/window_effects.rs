//! Detached, bounded window snapshots and native property tracks. Client shared
//! buffers are read only while capturing; animation owns the resulting pixels.
use super::*;
use animation::{Descriptor, Easing, Edge as MotionEdge, Effect, GenieMap, Rect, Timeline};

const MAX_SNAPSHOT_PIXELS: usize = 16 * 1024 * 1024;
const MAX_SCALARS: usize = 768;
struct Snapshot {
    pid: u32,
    workspace: usize,
    reported: bool,
    source: Rect,
    target: Rect,
    pixels: Vec<u32>,
    descriptor: Descriptor,
    timeline: Timeline,
}
impl Snapshot {
    fn fraction(&self, now: u64) -> u32 {
        self.timeline.sample(now)
    }
    fn redirect(&mut self, now: u64, minimizing: bool, duration: u32) {
        // Descriptor validation bounds duration before redirecting a live clock.
        let _ = self.timeline.retarget(now, duration, minimizing);
    }
}

struct Scalar {
    pid: u32,
    property: lua_policy::Property,
    start: u64,
    duration: u32,
    from: i64,
    to: i64,
    easing: Easing,
}
impl Scalar {
    fn finished(&self, now: u64) -> bool {
        now.saturating_sub(self.start) >= self.duration as u64
    }
    fn value(&self, now: u64) -> i64 {
        if self.finished(now) {
            return self.to;
        }
        let t =
            (now.saturating_sub(self.start) * animation::UNIT as u64 / self.duration as u64) as u32;
        let t = self.easing.sample(t) as i64;
        if self.property == lua_policy::Property::Color {
            let mut color = 0;
            for shift in [0, 8, 16, 24] {
                let a = (self.from >> shift) & 255;
                let b = (self.to >> shift) & 255;
                color |= (a + (b - a) * t / animation::UNIT as i64).clamp(0, 255) << shift;
            }
            color
        } else {
            self.from + (self.to - self.from) * t / animation::UNIT as i64
        }
    }
}
fn geometry_property(property: &lua_policy::Property) -> bool {
    use lua_policy::Property::*;
    matches!(
        property,
        X | Y | Width | Height | Scale | TranslationX | TranslationY
    )
}
fn valid_endpoints(property: &lua_policy::Property, from: i64, to: i64) -> bool {
    use lua_policy::Property::*;
    let (min, max) = match property {
        Opacity => (0, animation::UNIT as i64),
        Scale => (0, 16 * animation::UNIT as i64),
        X | Y | TranslationX | TranslationY => (-16384, 16384),
        Width | Height => (1, 16384),
        Blur | Shadow | CornerRadius => (0, 1024),
        Color => (0, u32::MAX as i64),
    };
    (min..=max).contains(&from) && (min..=max).contains(&to)
}

pub struct PaintStyle<'a> {
    pub theme: &'a Theme,
    pub layout: &'a Layout,
    pub effects: &'a super::Effects,
    pub font: Option<&'a Font>,
    pub apps: &'a Applications,
}
/// The caller applies these positions and sends a client configure for changed
/// dimensions. Taking a commit drains its geometry tracks exactly once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryCommit {
    pub pid: u32,
    pub cx: i32,
    pub cy: i32,
    pub width: i32,
    pub height: i32,
}
pub type Geometry = (i32, i32, i32, i32);
pub struct Effects {
    snapshots: Vec<Snapshot>,
    alpha: Vec<Scalar>,
}
impl Effects {
    pub fn animating(&self, now:u64)->bool {
        self.snapshots.iter().any(|s|!s.timeline.finished(now))
            ||self.alpha.iter().any(|s|now.saturating_sub(s.start)<s.duration as u64)
    }
    pub fn new() -> Self {
        Self {
            snapshots: Vec::new(),
            alpha: Vec::new(),
        }
    }
    pub fn prune(&mut self, wins: &[Win], now: u64) {
        self.snapshots.retain(|s| {
            !s.timeline.finished(now)
                || (s.timeline.minimizing()
                    && wins
                        .iter()
                        .any(|w| w.pid == s.pid && w.alive && w.minimized))
        });
        self.alpha
            .retain(|a| wins.iter().any(|w| w.pid == a.pid && w.alive));
    }
    pub fn clear(&mut self) {
        self.snapshots.clear();
        self.alpha.clear();
    }
    pub fn alpha(&self, pid: u32, now: u64) -> u32 {
        self.alpha
            .iter()
            .find(|a| a.pid == pid && a.property == lua_policy::Property::Opacity)
            .map_or(255, |a| {
                (a.value(now) * 255 / animation::UNIT as i64).clamp(0, 255) as u32
            })
    }
    pub fn scalar(
        &mut self,
        pid: u32,
        property: lua_policy::Property,
        now: u64,
        duration: u32,
        from: i64,
        to: i64,
        easing: Easing,
    ) -> bool {
        if duration > animation::MAX_DURATION_MS || !valid_endpoints(&property, from, to) {
            return false;
        }
        let existing = self
            .alpha
            .iter()
            .position(|a| a.pid == pid && a.property == property);
        if let Some(index) = existing {
            self.alpha.remove(index);
        } else if self.alpha.len() >= MAX_SCALARS {
            return false;
        }
        self.alpha.push(Scalar {
            pid,
            property,
            start: now,
            duration,
            from,
            to,
            easing,
        });
        true
    }
    fn geometry(&self, w: &Win, now: u64) -> Win {
        let mut v = w.clone();
        let mut tx = 0;
        let mut ty = 0;
        let mut scale = animation::UNIT as i64;
        use lua_policy::Property::*;
        for scalar in self.alpha.iter().filter(|s| s.pid == w.pid) {
            let value = scalar.value(now);
            match scalar.property {
                X => v.cx = value.clamp(-16384, 16384) as i32,
                Y => v.cy = value.clamp(-16384, 16384) as i32,
                TranslationX => tx = value.clamp(-16384, 16384) as i32,
                TranslationY => ty = value.clamp(-16384, 16384) as i32,
                Width => v.sw = value.clamp(1, 16384) as i32,
                Height => v.sh = value.clamp(1, 16384) as i32,
                Scale => scale = value.clamp(0, 16 * animation::UNIT as i64),
                Opacity | Blur | Shadow | CornerRadius | Color => {}
            }
        }
        v.cx = v.cx.saturating_add(tx);
        v.cy = v.cy.saturating_add(ty);
        v.sw = (v.sw as i64 * scale / animation::UNIT as i64).clamp(1, 16384) as i32;
        v.sh = (v.sh as i64 * scale / animation::UNIT as i64).clamp(1, 16384) as i32;
        v
    }
    pub fn input_geometry(&self, w: &Win, now: u64) -> Win {
        self.geometry(w, now)
    }
    pub fn cancel_geometry(&mut self, pid: u32) {
        self.alpha
            .retain(|s| s.pid != pid || !geometry_property(&s.property));
    }
    pub fn take_geometry_commits(&mut self, wins: &[Win], now: u64) -> Vec<GeometryCommit> {
        let mut commits = Vec::new();
        for w in wins.iter().filter(|w| w.alive) {
            let mut found = false;
            let done = self
                .alpha
                .iter()
                .filter(|s| s.pid == w.pid && geometry_property(&s.property))
                .all(|s| {
                    found = true;
                    s.finished(now)
                });
            if found && done {
                let v = self.geometry(w, now);
                commits.push(GeometryCommit {
                    pid: w.pid,
                    cx: v.cx,
                    cy: v.cy,
                    width: v.sw,
                    height: v.sh,
                });
            }
        }
        self.alpha
            .retain(|s| !geometry_property(&s.property) || !commits.iter().any(|c| c.pid == s.pid));
        commits
    }
    pub fn visual(
        &self,
        w: &Win,
        now: u64,
        theme: &Theme,
        fx: &super::Effects,
    ) -> (Win, Theme, super::Effects) {
        let v = self.geometry(w, now);
        let mut t = *theme;
        let mut f = *fx;
        use lua_policy::Property::*;
        for scalar in self.alpha.iter().filter(|s| s.pid == w.pid) {
            let value = scalar.value(now);
            match scalar.property {
                Blur => {
                    f.blur = value > 0;
                    f.blur_radius = value.clamp(0, 32) as i32;
                }
                Shadow => f.shadow = value.clamp(0, 128) as i32,
                CornerRadius => f.corner_radius = value.clamp(0, 1024) as i32,
                Color => {
                    let color = value as u32;
                    t.border_focused = color;
                    t.border_unfocused = color;
                    t.title_focused = color;
                    t.title_unfocused = color;
                }
                Opacity | X | Y | Width | Height | Scale | TranslationX | TranslationY => {}
            }
        }
        (v, t, f)
    }
    /// Returns false when no snapshot exists, letting the host start an opening
    /// effect from the current client surface rather than silently dropping it.
    pub fn configure(
        &mut self,
        pid: u32,
        effect: Effect,
        duration: u32,
        easing: Easing,
        now: u64,
    ) -> bool {
        let Ok(descriptor) = Descriptor::new(effect, duration, easing) else {
            return false;
        };
        if let Some(s) = self.snapshots.iter_mut().find(|s| s.pid == pid) {
            s.descriptor = descriptor;
            s.redirect(now, s.timeline.minimizing(), descriptor.duration_ms());
            true
        } else {
            false
        }
    }
    pub fn covers(&self, pid: u32, now: u64) -> bool {
        self.snapshots
            .iter()
            .any(|s| s.pid == pid && !s.timeline.finished(now))
    }
    /// Closed clients may already have left the live window list when their Lua
    /// response arrives. Their detached snapshot still supplies both endpoints.
    pub fn snapshot_geometry(&self, pid: u32) -> Option<(Geometry, Geometry)> {
        self.snapshots.iter().find(|s| s.pid == pid).map(|s| {
            (
                (s.source.x, s.source.y, s.source.width, s.source.height),
                (s.target.x, s.target.y, s.target.width, s.target.height),
            )
        })
    }
    pub fn restore(&mut self, w: &Win, now: u64, duration: u32) {
        if duration > animation::MAX_DURATION_MS {
            return;
        }
        if let Some(s) = self.snapshots.iter_mut().find(|s| s.pid == w.pid) {
            s.redirect(now, false, duration);
        }
    }
    pub fn minimize(
        &mut self,
        w: &Win,
        back: &[u32],
        bw: usize,
        bh: usize,
        target: (i32, i32, i32, i32),
        now: u64,
        duration: u32,
        style: &PaintStyle,
    ) {
        self.start(
            w,
            back,
            bw,
            bh,
            target,
            now,
            duration,
            Effect::Scale,
            Easing::ease_out(),
            true,
            style,
        );
    }
    pub fn start(
        &mut self,
        w: &Win,
        _back: &[u32],
        _bw: usize,
        _bh: usize,
        target: (i32, i32, i32, i32),
        now: u64,
        duration: u32,
        effect: Effect,
        easing: Easing,
        minimizing: bool,
        style: &PaintStyle,
    ) {
        let Ok(descriptor) = Descriptor::new(effect, duration, easing) else {
            return;
        };
        if let Some(s) = self.snapshots.iter_mut().find(|s| s.pid == w.pid) {
            s.descriptor = descriptor;
            s.redirect(now, minimizing, duration);
            return;
        }
        if duration == 0 {
            return;
        }
        let (x, y, width, height) = w.outer();
        let source = Rect {
            x,
            y,
            width,
            height,
        };
        let target = Rect {
            x: target.0,
            y: target.1,
            width: target.2.max(1),
            height: target.3.max(1),
        };
        if !source.valid() || !target.valid() {
            return;
        }
        let count = width as usize * height as usize;
        if count > MAX_SNAPSHOT_PIXELS
            || self.snapshots.iter().map(|s| s.pixels.len()).sum::<usize>() + count
                > MAX_SNAPSHOT_PIXELS
        {
            return;
        }
        let mut pixels = alloc::vec![0; count];
        let mut local = w.clone();
        local.cx = w.border;
        local.cy = w.title_h + w.border;
        let mut backing = Vec::new();
        let mut logged = true;
        let mut blur_a = Vec::new();
        let mut blur_b = Vec::new();
        let alpha = (self.alpha(w.pid, now) * w.opacity as u32 / 255) as i32;
        let mut snapshot_effects = *style.effects;
        snapshot_effects.blur = false;
        window_painter::paint(
            &mut pixels,
            width as usize,
            height as usize,
            &local,
            true,
            alpha,
            style.theme,
            style.layout,
            &snapshot_effects,
            style.font,
            style.apps,
            i32::MIN,
            i32::MIN,
            &mut backing,
            &mut logged,
            &mut blur_a,
            &mut blur_b,
        );
        let Ok(timeline) = Timeline::new(now, duration, minimizing) else {
            return;
        };
        self.snapshots.push(Snapshot {
            pid: w.pid,
            reported: false,
            workspace: w.ws,
            source,
            target,
            pixels,
            descriptor,
            timeline,
        });
    }
    pub fn paint(&mut self, back: &mut [u32], bw: usize, bh: usize, now: u64, workspace: usize) {
        // Prune decides whether a completed endpoint belongs to a minimized
        // live window. Paint never discards a restoration snapshot prematurely.
        for s in &mut self.snapshots {
            if s.workspace != workspace || s.timeline.finished(now) {
                continue;
            }
            let fraction = s.fraction(now);
            let Ok(frame) = s.descriptor.frame(s.source, s.target, fraction) else {
                continue;
            };
            let genie = if let Effect::Genie(edge) = s.descriptor.effect {
                GenieMap::new(
                    s.source,
                    s.target,
                    edge,
                    s.descriptor
                        .easing
                        .sample(fraction)
                        .clamp(0, animation::UNIT as i32) as u32,
                )
                .ok()
            } else {
                None
            };
            let mut painted = 0usize;
            let r = genie.as_ref().map(|g| g.bounds()).unwrap_or(frame.rect);
            for y in r.y.max(0)..(r.y + r.height).min(bh as i32) {
                for x in r.x.max(0)..(r.x + r.width).min(bw as i32) {
                    let coord = if let Some(g) = &genie {
                        g.inverse(x, y)
                    } else {
                        Some((
                            ((x - r.x) as i64 * s.source.width as i64 / r.width as i64) as usize,
                            ((y - r.y) as i64 * s.source.height as i64 / r.height as i64) as usize,
                        ))
                    };
                    if let Some((sx, sy)) = coord {
                        if let Some(&pixel) = s.pixels.get(sy * s.source.width as usize + sx) {
                            painted += 1;
                            blend_pixel(
                                back,
                                bw,
                                bh,
                                x,
                                y,
                                ((frame.opacity as u32 * (pixel >> 24) / 255) << 24)
                                    | (pixel & 0x00ff_ffff),
                            );
                        }
                    }
                }
            }
            if genie.is_some() && painted > 0 && !s.reported {
                report_genie(s.pid, painted);
                s.reported = true;
            }
        }
    }
}
pub fn target_edge(source: (i32, i32, i32, i32), target: (i32, i32, i32, i32)) -> MotionEdge {
    let dx = target.0 as i64 + target.2 as i64 / 2 - source.0 as i64 - source.2 as i64 / 2;
    let dy = target.1 as i64 + target.3 as i64 / 2 - source.1 as i64 - source.3 as i64 / 2;
    if dx.abs() > dy.abs() {
        if dx < 0 {
            MotionEdge::Left
        } else {
            MotionEdge::Right
        }
    } else if dy < 0 {
        MotionEdge::Top
    } else {
        MotionEdge::Bottom
    }
}

pub fn preset(
    name: &str,
    source: (i32, i32, i32, i32),
    target: (i32, i32, i32, i32),
) -> (Effect, Easing) {
    match name {
        "fade" => (Effect::Fade, Easing::linear()),
        "slide" => (Effect::Slide, Easing::ease_out()),
        "spring" => (Effect::Spring, Easing::spring()),
        "genie" => (
            Effect::Genie(target_edge(source, target)),
            Easing::ease_out(),
        ),
        _ => (Effect::Scale, Easing::ease_out()),
    }
}
