//! Cached native transforms for the workspace surface and transient UI layers.
use super::*;
pub const LAUNCHER: u32 = 513;
pub const WORKSPACE: u32 = 514;
pub const NOTIFICATIONS: u32 = 515;
pub fn logical_point(
    rect: (i32, i32, i32, i32),
    x: i32,
    y: i32,
    bw: usize,
    bh: usize,
) -> (i32, i32) {
    if rect.2 <= 0 || rect.3 <= 0 {
        return (-1_000_000, -1_000_000);
    }
    (
        (((x - rect.0) as i64 * bw as i64 / rect.2 as i64).clamp(-1_000_000, 1_000_000)) as i32,
        (((y - rect.1) as i64 * bh as i64 / rect.3 as i64).clamp(-1_000_000, 1_000_000)) as i32,
    )
}
#[derive(Default)]
pub struct Layer {
    before: Vec<u32>,
    source: Vec<u32>,
}
impl Layer {
    pub fn begin(&mut self, back: &[u32]) -> bool {
        if back.len() > 8 * 1024 * 1024 {
            return false;
        }
        self.before.clear();
        self.before.extend_from_slice(back);
        true
    }
    pub fn finish(
        &mut self,
        back: &mut [u32],
        bw: usize,
        bh: usize,
        visual: &shell_motion::Visual,
        a: &mut Vec<u32>,
        b: &mut Vec<u32>,
    ) {
        if visual.blur > 0 {
            if a.len() < back.len() {
                a.resize(back.len(), 0);
                b.resize(back.len(), 0);
            }
            blur_region(
                back,
                bw,
                bh,
                0,
                0,
                bw as i32,
                bh as i32,
                visual.blur,
                1,
                a,
                b,
            );
        }
        self.source.clear();
        self.source.extend_from_slice(back);
        back.copy_from_slice(&self.before);
        let (x, y, w, h) = visual.rect;
        if w <= 0 || h <= 0 {
            return;
        }
        if visual.shadow > 0 {
            draw_soft_shadow(
                back,
                bw,
                bh,
                x,
                y,
                w,
                h,
                visual.radius.unwrap_or(0),
                visual.shadow,
                80,
            );
        }
        for dy in y.max(0)..(y + h).min(bh as i32) {
            for dx in x.max(0)..(x + w).min(bw as i32) {
                let sx = ((dx - x) as i64 * bw as i64 / w as i64) as usize;
                let sy = ((dy - y) as i64 * bh as i64 / h as i64) as usize;
                if let Some(&pixel) = self.source.get(sy * bw + sx) {
                    let pixel = visual.color.map_or(pixel, |color| blend(pixel, color));
                    let coverage = rr_cov(dx, dy, x, y, w, h, visual.radius.unwrap_or(0), RR_ALL);
                    blend_pixel(
                        back,
                        bw,
                        bh,
                        dx,
                        dy,
                        ((visual.opacity * coverage / 255) << 24) | (pixel & 0x00ff_ffff),
                    );
                }
            }
        }
    }
}
pub fn changed(v: &shell_motion::Visual, bw: usize, bh: usize) -> bool {
    v.rect != (0, 0, bw as i32, bh as i32)
        || v.opacity != 255
        || v.color.is_some()
        || v.radius.unwrap_or(0) > 0
        || v.blur > 0
        || v.shadow > 0
}
