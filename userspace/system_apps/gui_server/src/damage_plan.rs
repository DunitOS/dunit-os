//! Pure conservative content-damage planner. Fallback means full composition,
//! never a partial image with stale occlusion, transparency or overlays.
extern crate alloc;
use alloc::vec::Vec;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}
impl Rect {
    pub fn intersects(self, b: Self) -> bool {
        self.w > 0
            && self.h > 0
            && b.w > 0
            && b.h > 0
            && self.x as i64 + (self.w as i64) > b.x as i64
            && b.x as i64 + (b.w as i64) > self.x as i64
            && self.y as i64 + (self.h as i64) > b.y as i64
            && b.y as i64 + (b.h as i64) > self.y as i64
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub content: Rect,
    pub cover: Rect,
    pub width: u32,
    pub height: u32,
    pub opaque: bool,
}
pub struct Update {
    pub surface: usize,
    pub rect: Rect,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Copy {
    pub surface: usize,
    pub x: u32,
    pub y: u32,
    pub destination: Rect,
}
pub struct Plan {
    pub copies: Vec<Copy>,
    pub pixels: u64,
}

pub fn plan(
    surfaces: &[Surface],
    updates: &[Update],
    blockers: &[Rect],
    width: u32,
    height: u32,
) -> Option<Plan> {
    let mut copies = Vec::new();
    let mut pixels = 0;
    for u in updates {
        let s = surfaces.get(u.surface)?;
        let r = u.rect;
        if !s.opaque
            || s.content.w != s.width
            || s.content.h != s.height
            || r.x < 0
            || r.y < 0
            || r.w == 0
            || r.h == 0
            || r.x as u64 + r.w as u64 > s.width as u64
            || r.y as u64 + r.h as u64 > s.height as u64
        {
            return None;
        }
        let left = s.content.x as i64 + r.x as i64;
        let top = s.content.y as i64 + r.y as i64;
        let x = left.max(0).min(width as i64);
        let y = top.max(0).min(height as i64);
        let right = (left + r.w as i64).max(0).min(width as i64);
        let bottom = (top + r.h as i64).max(0).min(height as i64);
        if right <= x || bottom <= y {
            continue;
        }
        let d = Rect {
            x: x as i32,
            y: y as i32,
            w: (right - x) as u32,
            h: (bottom - y) as u32,
        };
        if blockers.iter().any(|b| b.intersects(d))
            || surfaces[u.surface + 1..]
                .iter()
                .any(|s| s.cover.intersects(d))
        {
            return None;
        }
        copies.push(Copy {
            surface: u.surface,
            x: (r.x as i64 + x - left) as u32,
            y: (r.y as i64 + y - top) as u32,
            destination: d,
        });
        pixels += d.w as u64 * d.h as u64;
    }
    Some(Plan { copies, pixels })
}

pub fn apply(
    copy: Copy,
    source: &[u32],
    sw: usize,
    sh: usize,
    back: &mut [u32],
    bw: usize,
    bh: usize,
) -> bool {
    let d = copy.destination;
    if source.len() != sw.saturating_mul(sh)
        || back.len() != bw.saturating_mul(bh)
        || d.x < 0
        || d.y < 0
        || d.x as u64 + d.w as u64 > bw as u64
        || d.y as u64 + d.h as u64 > bh as u64
        || copy.x as u64 + d.w as u64 > sw as u64
        || copy.y as u64 + d.h as u64 > sh as u64
    {
        return false;
    }
    for row in 0..d.h as usize {
        let src = (copy.y as usize + row) * sw + copy.x as usize;
        let dst = (d.y as usize + row) * bw + d.x as usize;
        // Match the full painter's XRGB semantics: the unused source byte must
        // not become framebuffer alpha or affect subsequent overlay blending.
        for (target, pixel) in back[dst..dst + d.w as usize]
            .iter_mut()
            .zip(&source[src..src + d.w as usize])
        {
            *target = 0xff00_0000 | pixel;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn surface() -> Surface {
        Surface {
            content: Rect {
                x: 10,
                y: 10,
                w: 20,
                h: 20,
            },
            cover: Rect {
                x: 8,
                y: 2,
                w: 24,
                h: 30,
            },
            width: 20,
            height: 20,
            opaque: true,
        }
    }
    #[test]
    fn row_updates_only_declared_pixels_and_matches_full_redraw() {
        let s = surface();
        let mut image = alloc::vec![1;400];
        let mut back = alloc::vec![0;1600];
        apply(
            Copy {
                surface: 0,
                x: 0,
                y: 0,
                destination: s.content,
            },
            &image,
            20,
            20,
            &mut back,
            40,
            40,
        );
        let mut full = back.clone();
        image[40..60].fill(9);
        let p = plan(
            &[s],
            &[Update {
                surface: 0,
                rect: Rect {
                    x: 0,
                    y: 2,
                    w: 20,
                    h: 1,
                },
            }],
            &[],
            40,
            40,
        )
        .unwrap();
        assert_eq!(p.pixels, 20);
        assert!(apply(p.copies[0], &image, 20, 20, &mut back, 40, 40));
        // Independent full XRGB redraw, including the ignored alpha byte.
        for y in 0..20 {
            for x in 0..20 {
                full[(10 + y) * 40 + 10 + x] = 0xff00_0000 | image[y * 20 + x];
            }
        }
        assert_eq!(back, full);
    }
    #[test]
    fn overlap_transparency_scaling_cursor_and_shell_force_full() {
        let s = surface();
        let u = [Update {
            surface: 0,
            rect: Rect {
                x: 0,
                y: 2,
                w: 20,
                h: 1,
            },
        }];
        let mut upper = s;
        upper.cover = Rect {
            x: 20,
            y: 12,
            w: 10,
            h: 10,
        };
        assert!(plan(&[s, upper], &u, &[], 40, 40).is_none());
        assert!(plan(&[s], &u, &[upper.cover], 40, 40).is_none());
        assert!(plan(&[Surface { opaque: false, ..s }], &u, &[], 40, 40).is_none());
        assert!(plan(&[Surface { width: 40, ..s }], &u, &[], 40, 40).is_none());
    }
    #[test]
    fn offscreen_clipping_adjusts_source_coordinates() {
        let s = Surface {
            content: Rect {
                x: -10,
                y: -5,
                w: 20,
                h: 20,
            },
            ..surface()
        };
        let p = plan(
            &[s],
            &[Update {
                surface: 0,
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 20,
                    h: 20,
                },
            }],
            &[],
            40,
            40,
        )
        .unwrap();
        assert_eq!((p.copies[0].x, p.copies[0].y), (10, 5));
        assert_eq!(p.pixels, 150);
        let mut back = alloc::vec![0;1600];
        let src: Vec<u32> = (0..400).collect();
        assert!(apply(p.copies[0], &src, 20, 20, &mut back, 40, 40));
        assert_eq!(back[0], 0xff00_006e);
    }
    #[test]
    fn malformed_source_or_damage_never_writes() {
        let s = surface();
        assert!(plan(
            &[s],
            &[Update {
                surface: 0,
                rect: Rect {
                    x: 19,
                    y: 0,
                    w: 2,
                    h: 1
                }
            }],
            &[],
            40,
            40
        )
        .is_none());
        let mut back = alloc::vec![7;1600];
        assert!(!apply(
            Copy {
                surface: 0,
                x: 0,
                y: 0,
                destination: s.content
            },
            &[],
            20,
            20,
            &mut back,
            40,
            40
        ));
        assert!(back.iter().all(|p| *p == 7));
    }
}
