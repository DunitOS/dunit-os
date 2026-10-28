//! Integer raster kernels shared by SDK surfaces and desktop decorations.
/// Linear RGB interpolation between `a` and `b`; `t` in 0..=255 (0 = a, 255 = b).
#[inline]
pub fn lerp_color(a: u32, b: u32, t: u32) -> u32 {
    let t = t.min(255);
    let inv = 255 - t;
    let r = (((a >> 16) & 0xFF) * inv + ((b >> 16) & 0xFF) * t) / 255;
    let g = (((a >> 8) & 0xFF) * inv + ((b >> 8) & 0xFF) * t) / 255;
    let bl = ((a & 0xFF) * inv + (b & 0xFF) * t) / 255;
    (r << 16) | (g << 8) | bl
}

// Rounded-corner selection bitmask for `fill_rrect*` (which corners get the arc).
pub const RR_TL: u8 = 1;
pub const RR_TR: u8 = 2;
pub const RR_BL: u8 = 4;
pub const RR_BR: u8 = 8;
pub const RR_ALL: u8 = RR_TL | RR_TR | RR_BL | RR_BR;
pub const RR_TOP: u8 = RR_TL | RR_TR;

/// Coverage 0..255 of pixel (px,py) inside rounded-rect [x,y,w,h] radius `r`.
/// Straight edges and non-selected corners return 255; a selected corner is
/// 4x4-supersampled in 1/8-px units for a cheap anti-aliased arc (integer only).
pub fn rr_cov(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32, r: i32, corners: u8) -> u32 {
    if r <= 0 {
        return 255;
    }
    let (cx, cy, bit);
    if px < x + r && py < y + r {
        cx = x + r;
        cy = y + r;
        bit = RR_TL;
    } else if px >= x + w - r && py < y + r {
        cx = x + w - r;
        cy = y + r;
        bit = RR_TR;
    } else if px < x + r && py >= y + h - r {
        cx = x + r;
        cy = y + h - r;
        bit = RR_BL;
    } else if px >= x + w - r && py >= y + h - r {
        cx = x + w - r;
        cy = y + h - r;
        bit = RR_BR;
    } else {
        return 255;
    }
    if corners & bit == 0 {
        return 255;
    }
    let r8 = (r * 8) as i64;
    let r2 = r8 * r8;
    let mut inside = 0u32;
    let mut sy = 0;
    while sy < 4 {
        let dy = (py * 8 + (sy * 2 + 1) - cy * 8) as i64;
        let mut sx = 0;
        while sx < 4 {
            let dx = (px * 8 + (sx * 2 + 1) - cx * 8) as i64;
            if dx * dx + dy * dy <= r2 {
                inside += 1;
            }
            sx += 1;
        }
        sy += 1;
    }
    inside * 255 / 16
}
