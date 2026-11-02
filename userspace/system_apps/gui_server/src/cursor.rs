//! Shared opaque software cursor, reapplied after content damage.
/// A classic top-left arrow cursor. 'X' = black outline, '.' = white fill,
/// ' ' = transparent. Hotspot is the top-left corner (0,0).
pub const CURSOR: [&str; 17] = [
    "X                ",
    "XX               ",
    "X.X              ",
    "X..X             ",
    "X...X            ",
    "X....X           ",
    "X.....X          ",
    "X......X         ",
    "X.......X        ",
    "X........X       ",
    "X.....XXXXX      ",
    "X..X..X          ",
    "X.X X..X         ",
    "XX  X..X         ",
    "X    X..X        ",
    "     X..X        ",
    "      XX         ",
];

/// Draw the arrow cursor into the back buffer with its hotspot at (px, py).
pub fn draw_cursor(buf: &mut [u32], bw: usize, bh: usize, px: i32, py: i32) {
    for (ry, row) in CURSOR.iter().enumerate() {
        let dy = py + ry as i32;
        if dy < 0 || dy as usize >= bh {
            continue;
        }
        let drow = dy as usize * bw;
        for (rx, ch) in row.bytes().enumerate() {
            let color = match ch {
                b'X' => 0xFF000000,
                b'.' => 0xFFFFFFFF,
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
    fn stationary_cursor_content_damage_matches_full_composition_without_fallback() {
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
                draw_cursor(&mut back, bw, bh, pointer.0, pointer.1);
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
                draw_cursor(&mut back, bw, bh, pointer.0, pointer.1);
                let mut full = vec![background; bw * bh];
                for y in 0..20 {
                    full[(y + 10) * bw + 10..(y + 10) * bw + 30]
                        .copy_from_slice(&image[y * 20..y * 20 + 20]);
                }
                draw_cursor(&mut full, bw, bh, pointer.0, pointer.1);
                assert_eq!(back, full, "pointer={pointer:?} patch={patch:?}");
            }
        }
    }
}
