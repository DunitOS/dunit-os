//! Bounded software cursor shape, shared by full and partial composition.
extern crate alloc;
use alloc::vec::Vec;

pub struct Cursor {
    width: usize,
    pixels: Vec<u8>,
}
impl Cursor {
    /// X = outline, . = fill, _ = transparent. The asset owns its dimensions;
    /// the bound is a resource budget, not an appearance default.
    pub fn parse(source: &str) -> Result<Self, &'static str> {
        const MAX_SIDE: usize = 128;
        let mut width = None;
        let mut pixels = Vec::new();
        let mut height = 0;
        for row in source.lines() {
            let row = row.strip_suffix('\r').unwrap_or(row);
            if row.is_empty() || row.len() > MAX_SIDE || width.is_some_and(|w| w != row.len()) {
                return Err("cursor rows must be rectangular and bounded");
            }
            width = Some(row.len());
            height += 1;
            if height > MAX_SIDE {
                return Err("cursor height exceeds budget");
            }
            for byte in row.bytes() {
                pixels.push(match byte {
                    b'X' => 1,
                    b'.' => 2,
                    b'_' => 0,
                    _ => return Err("invalid cursor pixel"),
                });
            }
        }
        Ok(Self {
            width: width.ok_or("empty cursor")?,
            pixels,
        })
    }
}

/// Draw the arrow cursor into the back buffer with its hotspot at (px, py).
pub fn draw_cursor(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    px: i32,
    py: i32,
    cursor: &Cursor,
    colors: [u32; 2],
) {
    for (ry, row) in cursor.pixels.chunks(cursor.width).enumerate() {
        let dy = py + ry as i32;
        if dy < 0 || dy as usize >= bh {
            continue;
        }
        let drow = dy as usize * bw;
        for (rx, ch) in row.iter().enumerate() {
            let color = match ch {
                1 => colors[0],
                2 => colors[1],
                _ => continue,
            };
            let dx = px + rx as i32;
            if dx >= 0 && (dx as usize) < bw {
                buf[drow + dx as usize] = color;
            }
        }
    }
}
#[cfg(test)]
#[path = "damage_plan.rs"]
mod damage_plan;
#[cfg(test)]
mod tests {
    use super::*;
    use damage_plan::{Rect, Surface, Update};
    #[test]
    fn malformed_shapes_are_rejected_and_selected_colors_reach_pixels() {
        for bad in ["", "X\nXX", "X?", "X\n\n", &"X".repeat(129)] {
            assert!(Cursor::parse(bad).is_err());
        }
        let cursor = Cursor::parse("X._").unwrap();
        let mut back = vec![7; 3];
        draw_cursor(&mut back, 3, 1, 0, 0, &cursor, [0xff123456, 0xffabcdef]);
        assert_eq!(back, [0xff123456, 0xffabcdef, 7]);
    }
    #[test]
    fn stationary_cursor_content_damage_matches_full_composition_without_fallback() {
        let cursor = Cursor::parse(include_str!("../../../../assets/dwm/cursor.txt")).unwrap();
        for pointer in [(13, 13), (7, 9), (28, 24), (-3, -4), (38, 38)] {
            for patch in [
                Rect {
                    x: 0,
                    y: 0,
                    w: 20,
                    h: 20,
                },
                Rect {
                    x: 0,
                    y: 2,
                    w: 7,
                    h: 5,
                },
                Rect {
                    x: 9,
                    y: 9,
                    w: 11,
                    h: 11,
                },
            ] {
                let (bw, bh) = (40, 40);
                let background = 0xff102030;
                let surface = Surface {
                    content: Rect {
                        x: 10,
                        y: 10,
                        w: 20,
                        h: 20,
                    },
                    cover: Rect {
                        x: 10,
                        y: 10,
                        w: 20,
                        h: 20,
                    },
                    width: 20,
                    height: 20,
                    opaque: true,
                };
                let mut image = vec![0xff405060; 400];
                let mut back = vec![background; bw * bh];
                for y in 0..20 {
                    back[(y + 10) * bw + 10..(y + 10) * bw + 30]
                        .copy_from_slice(&image[y * 20..y * 20 + 20]);
                }
                draw_cursor(
                    &mut back,
                    bw,
                    bh,
                    pointer.0,
                    pointer.1,
                    &cursor,
                    [0xff203040, 0xffa0b0c0],
                );
                for y in patch.y as usize..patch.y as usize + patch.h as usize {
                    for x in patch.x as usize..patch.x as usize + patch.w as usize {
                        image[y * 20 + x] = 0xff90a0b0;
                    }
                }
                let plan = damage_plan::plan(
                    &[surface],
                    &[Update {
                        surface: 0,
                        rect: patch,
                    }],
                    &[],
                    bw as u32,
                    bh as u32,
                )
                .expect("cursor is repainted, not a blocker");
                for copy in plan.copies {
                    assert!(damage_plan::apply(copy, &image, 20, 20, &mut back, bw, bh));
                }
                draw_cursor(
                    &mut back,
                    bw,
                    bh,
                    pointer.0,
                    pointer.1,
                    &cursor,
                    [0xff203040, 0xffa0b0c0],
                );
                let mut full = vec![background; bw * bh];
                for y in 0..20 {
                    full[(y + 10) * bw + 10..(y + 10) * bw + 30]
                        .copy_from_slice(&image[y * 20..y * 20 + 20]);
                }
                draw_cursor(
                    &mut full,
                    bw,
                    bh,
                    pointer.0,
                    pointer.1,
                    &cursor,
                    [0xff203040, 0xffa0b0c0],
                );
                assert_eq!(back, full, "pointer={pointer:?} patch={patch:?}");
            }
        }
    }
}
