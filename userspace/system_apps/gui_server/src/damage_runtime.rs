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
    ws: usize,
    focus: Option<usize>,
    mouse: (i32, i32),
    z: Vec<usize>,
    wins: Vec<VisualKey>,
    theme: Theme,
    layout: Layout,
    effects: Effects,
    widgets: (bool, u64, u32, u64),
}
pub struct Cache {
    previous: Option<Key>,
    present: Vec<u32>,
    announced: bool,
}
pub struct Context<'a> {
    pub wins: &'a [Win],
    pub z: &'a [usize],
    pub focused: Option<usize>,
    pub workspace: usize,
    pub mouse: (i32, i32),
    pub now: u64,
    pub duration: u32,
    pub epoch: u64,
    pub theme: &'a Theme,
    pub layout: &'a Layout,
    pub effects: &'a Effects,
    pub shell: &'a shell_runtime::Shell,
    pub window_effects: &'a window_effects::Effects,
    pub blocked: bool,
    pub widgets: (bool, u64, u32, u64),
}
impl Cache {
    pub fn invalidate(&mut self) {
        self.previous = None;
    }
    pub fn new() -> Self {
        Self {
            previous: None,
            present: Vec::new(),
            announced: false,
        }
    }
    pub fn compose(
        &mut self,
        c: Context<'_>,
        clients: &[ClientState],
        back: &mut [u32],
        bw: usize,
        bh: usize,
    ) -> bool {
        let mut stable =
            !c.blocked && c.shell.reusable(c.now) && !c.window_effects.animating(c.now);
        let mut wins = Vec::new();
        let mut surfaces = Vec::new();
        let mut updates = Vec::new();
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
            ws: c.workspace,
            focus: c.focused,
            mouse: c.mouse,
            z: c.z.to_vec(),
            wins,
            theme: *c.theme,
            layout: *c.layout,
            effects: *c.effects,
            widgets: c.widgets,
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
        if !same {
            return false;
        }
        let mut blockers = c.shell.damage_blockers(c.now, c.effects);
        blockers.push(Rect {
            x: c.mouse.0,
            y: c.mouse.1,
            w: CURSOR.iter().map(|s| s.len()).max().unwrap_or(0) as u32,
            h: CURSOR.len() as u32,
        });
        let Some(p) = plan::plan(&surfaces, &updates, &blockers, bw as u32, bh as u32) else {
            return false;
        };
        let visible: Vec<_> =
            c.z.iter()
                .copied()
                .filter(|&wi| {
                    let w = &c.wins[wi];
                    w.alive && !w.minimized && w.ws == c.workspace
                })
                .collect();
        for copy in &p.copies {
            let w = &c.wins[visible[copy.surface]];
            let count = (w.surface_w as usize).saturating_mul(w.surface_h as usize);
            if w.buf_ptr.is_null() || count * 4 > w.buf_size {
                return false;
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
                return false;
            }
        }
        for copy in &p.copies {
            let r = copy.destination;
            self.present.clear();
            for y in r.y as usize..r.y as usize + r.h as usize {
                self.present.extend_from_slice(
                    &back[y * bw + r.x as usize..y * bw + r.x as usize + r.w as usize],
                );
            }
            let bytes = unsafe {
                core::slice::from_raw_parts(
                    self.present.as_ptr() as *const u8,
                    self.present.len() * 4,
                )
            };
            if libdunit::fb_present(bytes, r.w, r.h, r.x as u32, r.y as u32) != 0 {
                return false;
            }
        }
        if p.pixels > 0 {
            let texture: u64 = clients.iter().map(|c| c.buffers.copied).sum();
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
        true
    }
}
