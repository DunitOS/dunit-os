//! Retained desktop content path. Scene changes use the existing full painter;
//! stable opaque, unscaled, unobscured content patches use committed CPU images.
use super::*;
use crate::damage_plan::{self as plan, Rect, Surface, Update};

#[derive(PartialEq, Eq)]
struct VisualKey {
    pid: u32,
    app: u8,
    alive: bool,
    minimized: bool,
    ws: usize,
    has_image: bool,
    content: (i32, i32, i32, i32),
    source: (i32, i32),
    alpha: i32,
    theme: Theme,
    effects: Effects,
    title_h: i32,
    border: i32,
    lights: bool,
    format: u32,
}
#[derive(PartialEq, Eq)]
struct Key {
    epoch: u64,
    shell: u64,
    shell_hover: u64,
    title_hover: Option<(u32, u8)>,
    ws: usize,
    focus: Option<usize>,
    z: Vec<usize>,
    wins: Vec<VisualKey>,
    theme: Theme,
    layout: Layout,
    effects: Effects,
    widgets: (bool, u64, u32, u64),
    notif_sig: u64,
}
pub struct Cache {
    previous: Option<Key>,
    present: Vec<u32>,
    announced: bool,
    pub(super) cursor_rect: Option<(i32, i32, usize, usize)>,
    pub(super) cursor_under: Vec<u32>,
    cursor_pos: (i32, i32),
    cursor_visible: bool,
    cursor_ready: bool,
}
pub(super) enum Composition {
    Full,
    Partial,
    Idle,
}

pub struct Context<'a> {
    pub wins: &'a [Win],
    pub z: &'a [usize],
    pub focused: Option<usize>,
    pub workspace: usize,
    pub mouse: (i32, i32),
    pub logical_mouse: (i32, i32),
    pub cursor_visible: bool,
    pub now: u64,
    pub duration: u32,
    pub epoch: u64,
    pub theme: &'a Theme,
    pub layout: &'a Layout,
    pub skin: &'a dunit_style::shell::Visuals,
    pub effects: &'a Effects,
    pub shell: &'a shell_runtime::Shell,
    pub window_effects: &'a window_effects::Effects,
    pub blocked: bool,
    pub trace: bool,
    pub widgets: (bool, u64, u32, u64),
    pub notif_sig: u64,
    pub notif_blockers: &'a [Rect],
}

fn to_rect(r: (i32, i32, usize, usize)) -> Rect {
    Rect {
        x: r.0,
        y: r.1,
        w: r.2 as u32,
        h: r.3 as u32,
    }
}

fn rect_contains(outer: Rect, inner: Rect) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x as i64 + inner.w as i64 <= outer.x as i64 + outer.w as i64
        && inner.y as i64 + inner.h as i64 <= outer.y as i64 + outer.h as i64
}

fn present_rect(
    back: &[u32],
    bw: usize,
    r: Rect,
    scratch: &mut Vec<u32>,
    performance: &mut crate::performance::Profiler,
) -> bool {
    if r.w == 0 || r.h == 0 || r.x < 0 || r.y < 0 {
        return true;
    }
    scratch.clear();
    for y in r.y as usize..r.y as usize + r.h as usize {
        scratch.extend_from_slice(
            &back[y * bw + r.x as usize..y * bw + r.x as usize + r.w as usize],
        );
    }
    let bytes = unsafe {
        core::slice::from_raw_parts(scratch.as_ptr() as *const u8, scratch.len() * 4)
    };
    let perf_present = performance.stamp();
    let res = libdunit::fb_present(bytes, r.w, r.h, r.x as u32, r.y as u32);
    performance.present(perf_present, (r.w as u64) * (r.h as u64));
    res == 0
}

impl Cache {
    pub fn new() -> Self {
        Self {
            previous: None,
            present: Vec::new(),
            announced: false,
            cursor_rect: None,
            cursor_under: Vec::new(),
            cursor_pos: (0, 0),
            cursor_visible: false,
            cursor_ready: false,
        }
    }
    pub fn mark_cursor(&mut self, mx: i32, my: i32, visible: bool) {
        self.cursor_pos = (mx, my);
        self.cursor_visible = visible;
        self.cursor_ready = true;
    }
    pub fn compose(
        &mut self,
        c: Context<'_>,
        clients: &[ClientState],
        back: &mut [u32],
        bw: usize,
        bh: usize,
        performance: &mut crate::performance::Profiler,
    ) -> Composition {
        let mut stable =
            !c.blocked && c.shell.reusable(c.now) && !c.window_effects.animating(c.now);
        let mut wins = Vec::new();
        let mut surfaces = Vec::new();
        let mut updates = Vec::new();
        let mut title_hover = None;
        for w in c.wins {
            let (v, t, fx) = c.window_effects.visual(w, c.now, c.theme, c.effects);
            let alpha = (animation::progress(c.now, w.born, c.duration, false)
                * c.window_effects.alpha(w.pid, c.now)
                / 255
                * w.opacity as u32
                / 255) as i32;
            if w.motion.is_some() || alpha != w.opacity as i32 {
                stable = false;
            }
            if title_hover.is_none()
                && w.alive
                && !w.minimized
                && w.ws == c.workspace
                && v.traffic_lights
            {
                for role in 0..3u8 {
                    if in_rect(v.control_rect(role as i32, c.skin), c.logical_mouse.0, c.logical_mouse.1) {
                        title_hover = Some((w.pid, role));
                        break;
                    }
                }
            }
            wins.push(VisualKey {
                pid: w.pid,
                app: w.app,
                alive: w.alive,
                minimized: w.minimized,
                ws: w.ws,
                has_image: !w.buf_ptr.is_null(),
                content: (v.cx, v.cy, v.sw, v.sh),
                source: (w.surface_w, w.surface_h),
                alpha,
                theme: t,
                effects: fx,
                title_h: v.title_h,
                border: v.border,
                lights: v.traffic_lights,
                format: v.format,
            });
        }
        let key = Key {
            epoch: c.epoch,
            shell: c.shell.revision,
            shell_hover: c.shell.hover_token(c.mouse.0, c.mouse.1),
            title_hover,
            ws: c.workspace,
            focus: c.focused,
            z: c.z.to_vec(),
            wins,
            theme: *c.theme,
            layout: *c.layout,
            effects: *c.effects,
            widgets: c.widgets,
            notif_sig: c.notif_sig,
        };
        for &wi in c.z {
            let w = &c.wins[wi];
            if !w.alive || w.minimized || w.ws != c.workspace {
                continue;
            }
            let v = &key.wins[wi];
            let margin = v
                .effects
                .shadow
                .max(0)
                .saturating_add(v.effects.blur_radius.max(0));
            let x = v.content.0 - v.border;
            let y = v.content.1 - v.title_h - v.border;
            let ww = v.content.2 + 2 * v.border;
            let hh = v.content.3 + v.title_h + 2 * v.border;
            let surface = Surface {
                content: Rect {
                    x: v.content.0,
                    y: v.content.1,
                    w: v.content.2.max(0) as u32,
                    h: v.content.3.max(0) as u32,
                },
                cover: Rect {
                    x: x - margin,
                    y: y - margin,
                    w: (ww + 2 * margin).max(0) as u32,
                    h: (hh + 2 * margin).max(0) as u32,
                },
                width: w.surface_w.max(0) as u32,
                height: w.surface_h.max(0) as u32,
                opaque: w.format == FORMAT_XRGB8888 && v.alpha == 255 && !w.buf_ptr.is_null(),
            };
            if let Some(client) = clients.iter().find(|client| client.pid == w.pid) {
                if client.buffers.frame.is_some_and(|f| f.damage.continuous) {
                    stable = false;
                }
                if let Some(r) = client.dirty {
                    updates.push(Update {
                        surface: surfaces.len(),
                        rect: Rect {
                            x: r.x,
                            y: r.y,
                            w: r.w,
                            h: r.h,
                        },
                    });
                }
            }
            surfaces.push(surface);
        }
        if stable && !self.announced {
            libdunit::println("[DAMAGE] scene reusable");
            self.announced = true;
        }
        let same = stable && self.previous.as_ref() == Some(&key);
        self.previous = if stable { Some(key) } else { None };
        if !same || !self.cursor_ready {
            self.cursor_ready = false;
            return Composition::Full;
        }

        let new_cursor = if c.cursor_visible {
            crate::cursor::bounds(c.mouse.0, c.mouse.1, &c.shell.cursor, bw, bh)
        } else {
            None
        };
        let cursor_changed = new_cursor != self.cursor_rect
            || c.cursor_visible != self.cursor_visible
            || (c.cursor_visible && c.mouse != self.cursor_pos);

        if updates.is_empty() {
            if !cursor_changed {
                return Composition::Idle;
            }
            let old_cursor = self.cursor_rect;
            if let Some(old) = old_cursor {
                crate::cursor::restore_under(back, bw, old, &self.cursor_under);
            }
            if let Some(new) = new_cursor {
                crate::cursor::save_under(back, bw, new, &mut self.cursor_under);
                draw_cursor(
                    back,
                    bw,
                    bh,
                    c.mouse.0,
                    c.mouse.1,
                    &c.shell.cursor,
                    c.shell.visuals.cursor_colors(),
                );
            } else {
                self.cursor_under.clear();
            }
            let ok = match (old_cursor.map(to_rect), new_cursor.map(to_rect)) {
                (Some(a), Some(b)) => {
                    let x0 = a.x.min(b.x);
                    let y0 = a.y.min(b.y);
                    let x1 = (a.x + a.w as i32).max(b.x + b.w as i32);
                    let y1 = (a.y + a.h as i32).max(b.y + b.h as i32);
                    let union = Rect {
                        x: x0,
                        y: y0,
                        w: (x1 - x0) as u32,
                        h: (y1 - y0) as u32,
                    };
                    if union.w * union.h <= (a.w * a.h + b.w * b.h) * 2 {
                        present_rect(back, bw, union, &mut self.present, performance)
                    } else {
                        present_rect(back, bw, a, &mut self.present, performance)
                            && present_rect(back, bw, b, &mut self.present, performance)
                    }
                }
                (Some(r), None) | (None, Some(r)) => {
                    present_rect(back, bw, r, &mut self.present, performance)
                }
                (None, None) => true,
            };
            if !ok {
                self.cursor_ready = false;
                return Composition::Full;
            }
            self.cursor_rect = new_cursor;
            self.cursor_pos = c.mouse;
            self.cursor_visible = c.cursor_visible;
            return Composition::Partial;
        }

        let mut blockers = c.shell.damage_blockers(c.now, c.effects);
        blockers.extend_from_slice(c.notif_blockers);
        let Some(p) = plan::plan(&surfaces, &updates, &blockers, bw as u32, bh as u32) else {
            self.cursor_ready = false;
            return Composition::Full;
        };
        let visible: Vec<_> = c
            .z
            .iter()
            .copied()
            .filter(|&wi| {
                let w = &c.wins[wi];
                w.alive && !w.minimized && w.ws == c.workspace
            })
            .collect();
        let old_cursor = self.cursor_rect;
        if let Some(old) = old_cursor {
            crate::cursor::restore_under(back, bw, old, &self.cursor_under);
        }
        for copy in &p.copies {
            let w = &c.wins[visible[copy.surface]];
            let count = (w.surface_w as usize).saturating_mul(w.surface_h as usize);
            if w.buf_ptr.is_null() || count * 4 > w.buf_size {
                self.cursor_ready = false;
                return Composition::Full;
            }
            let pixels = unsafe { core::slice::from_raw_parts(w.buf_ptr as *const u32, count) };
            if !plan::apply(
                *copy,
                pixels,
                w.surface_w as usize,
                w.surface_h as usize,
                back,
                bw,
                bh,
            ) {
                self.cursor_ready = false;
                return Composition::Full;
            }
        }
        if c.cursor_visible {
            if let Some(new) = new_cursor {
                crate::cursor::save_under(back, bw, new, &mut self.cursor_under);
            } else {
                self.cursor_under.clear();
            }
            draw_cursor(
                back,
                bw,
                bh,
                c.mouse.0,
                c.mouse.1,
                &c.shell.cursor,
                c.shell.visuals.cursor_colors(),
            );
        } else {
            self.cursor_under.clear();
        }
        for copy in &p.copies {
            if !present_rect(back, bw, copy.destination, &mut self.present, performance) {
                self.cursor_ready = false;
                return Composition::Full;
            }
        }
        if cursor_changed {
            for r in [old_cursor.map(to_rect), new_cursor.map(to_rect)]
                .into_iter()
                .flatten()
            {
                if !p.copies.iter().any(|cp| rect_contains(cp.destination, r))
                    && !present_rect(back, bw, r, &mut self.present, performance)
                {
                    self.cursor_ready = false;
                    return Composition::Full;
                }
            }
        }
        self.cursor_rect = new_cursor;
        self.cursor_pos = c.mouse;
        self.cursor_visible = c.cursor_visible;
        if p.pixels > 0 && (c.trace || performance.collecting()) {
            let texture: u64 = clients.iter().map(|c| c.buffers.copied).sum();
            performance.partial_present(texture);
            if c.trace {
                libdunit::println(&alloc::format!(
                    "[DAMAGE] partial composed={} presented={} texture={} screen={}",
                    p.pixels,
                    p.pixels,
                    texture,
                    bw * bh
                ));
                if p.pixels < (bw * bh) as u64 && texture > 0 && texture <= p.pixels {
                    libdunit::println("[DAMAGE] partial frame OK");
                }
            }
        }
        if p.pixels > 0 || cursor_changed {
            Composition::Partial
        } else {
            Composition::Idle
        }
    }
}
