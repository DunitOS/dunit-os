//! Group clipping: apply corner coverage once after all descendants paint.
use crate::{
    effects::{coverage, integer},
    PaintResult, Surface,
};
use alloc::{collections::TryReserveError, vec::Vec};
use dunit_ui::Rect;

/// Interpolate two completed pixels in premultiplied space, then return straight
/// ARGB. This also preserves color when one side has zero alpha.
fn mix(before: u32, after: u32, coverage: u32) -> u32 {
    if coverage == 0 {
        return before;
    }
    if coverage == 255 {
        return after;
    }
    let inv = 255 - coverage;
    let a = (before >> 24) * inv + (after >> 24) * coverage;
    if a == 0 {
        return 0;
    }
    let mut pixel = (a / 255) << 24;
    for shift in [0, 8, 16] {
        let channel = (((before >> shift) & 255) * (before >> 24) * inv
            + ((after >> shift) & 255) * (after >> 24) * coverage)
            / a;
        pixel |= channel << shift;
    }
    pixel
}
impl Surface<'_> {
    /// Draw a group inside a rectangular layout clip and apply rounded coverage
    /// to its completed pixels. Only corner pixels need a saved backdrop;
    /// overlapping children cannot accumulate the parent's edge coverage.
    pub fn with_rounded_clip(
        &mut self,
        rect: Rect,
        radius: f32,
        draw: impl FnOnce(&mut Self) -> PaintResult,
    ) -> PaintResult {
        let mut result = Ok(());
        self.with_clip(rect, |surface| {
            let geometry = integer(rect);
            let (x, y, w, h) = geometry;
            let radius = ((radius.max(0.) + 0.5) as i32).min(w.min(h).max(0) / 2);
            let (x0, y0, x1, y1) = surface.clip_rect(rect.x, rect.y, rect.w, rect.h);
            let save = || -> Result<Vec<(usize, u32, u32)>, TryReserveError> {
                let mut saved = Vec::new();
                if radius == 0 {
                    return Ok(saved);
                }
                for py in y0..y1 {
                    if (y + radius..y + h - radius).contains(&(py as i32)) {
                        continue;
                    }
                    let mut px = x0;
                    while px < x1 {
                        if px as i32 >= x + radius && (px as i32) < x + w - radius {
                            px = (x + w - radius).max(0) as usize;
                            continue;
                        }
                        let cov = coverage(px as i32, py as i32, geometry, radius);
                        if cov < 255 {
                            saved.try_reserve(1)?;
                            let index = py * surface.stride + px;
                            saved.push((index, surface.pixels[index], cov));
                        }
                        px += 1;
                    }
                }
                Ok(saved)
            };
            let saved = match save() {
                Ok(saved) => saved,
                Err(error) => {
                    result = Err(error);
                    return;
                }
            };
            result = draw(surface);
            for (index, before, cov) in saved {
                surface.pixels[index] = mix(before, surface.pixels[index], cov);
            }
        });
        result
    }
}
