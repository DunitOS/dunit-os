#![no_std]
#![no_main]

extern crate alloc;

use dunit::display::Display;

const IMAGE_W: usize = 16;
const IMAGE_H: usize = 16;
const SCALE: u32 = 10;

const TRANSPARENT: u32 = 0xffff_ffff;
const BLACK: u32 = 0x0012_1720;
const OUTLINE: u32 = 0x0026_3442;
const GREEN: u32 = 0x004f_d184;
const GREEN_DARK: u32 = 0x0024_8f5a;
const TEA: u32 = 0x00d9_f99d;
const WHITE: u32 = 0x00f5_faf3;
const AMBER: u32 = 0x00f2_c14e;

const IMAGE: [u32; IMAGE_W * IMAGE_H] = [
    TRANSPARENT, TRANSPARENT, TRANSPARENT, TRANSPARENT, OUTLINE, OUTLINE, OUTLINE, OUTLINE, OUTLINE, OUTLINE, OUTLINE, OUTLINE, TRANSPARENT, TRANSPARENT, TRANSPARENT, TRANSPARENT,
    TRANSPARENT, TRANSPARENT, OUTLINE, OUTLINE, GREEN_DARK, GREEN_DARK, GREEN_DARK, GREEN_DARK, GREEN_DARK, GREEN_DARK, GREEN_DARK, GREEN_DARK, OUTLINE, OUTLINE, TRANSPARENT, TRANSPARENT,
    TRANSPARENT, OUTLINE, GREEN_DARK, GREEN, GREEN, GREEN, GREEN, GREEN, GREEN, GREEN, GREEN, GREEN, GREEN_DARK, GREEN_DARK, OUTLINE, TRANSPARENT,
    OUTLINE, GREEN_DARK, GREEN, GREEN, GREEN, GREEN, TEA, TEA, TEA, TEA, GREEN, GREEN, GREEN, GREEN_DARK, GREEN_DARK, OUTLINE,
    OUTLINE, GREEN_DARK, GREEN, GREEN, TEA, TEA, WHITE, WHITE, WHITE, WHITE, TEA, TEA, GREEN, GREEN, GREEN_DARK, OUTLINE,
    OUTLINE, GREEN_DARK, GREEN, TEA, WHITE, WHITE, WHITE, AMBER, AMBER, WHITE, WHITE, WHITE, TEA, GREEN, GREEN_DARK, OUTLINE,
    OUTLINE, GREEN_DARK, GREEN, TEA, WHITE, WHITE, AMBER, BLACK, BLACK, AMBER, WHITE, WHITE, TEA, GREEN, GREEN_DARK, OUTLINE,
    OUTLINE, GREEN_DARK, GREEN, TEA, WHITE, AMBER, BLACK, WHITE, WHITE, BLACK, AMBER, WHITE, TEA, GREEN, GREEN_DARK, OUTLINE,
    OUTLINE, GREEN_DARK, GREEN, TEA, WHITE, AMBER, BLACK, WHITE, WHITE, BLACK, AMBER, WHITE, TEA, GREEN, GREEN_DARK, OUTLINE,
    OUTLINE, GREEN_DARK, GREEN, TEA, WHITE, WHITE, AMBER, BLACK, BLACK, AMBER, WHITE, WHITE, TEA, GREEN, GREEN_DARK, OUTLINE,
    OUTLINE, GREEN_DARK, GREEN, TEA, WHITE, WHITE, WHITE, AMBER, AMBER, WHITE, WHITE, WHITE, TEA, GREEN, GREEN_DARK, OUTLINE,
    OUTLINE, GREEN_DARK, GREEN, GREEN, TEA, TEA, WHITE, WHITE, WHITE, WHITE, TEA, TEA, GREEN, GREEN, GREEN_DARK, OUTLINE,
    OUTLINE, GREEN_DARK, GREEN_DARK, GREEN, GREEN, GREEN, TEA, TEA, TEA, TEA, GREEN, GREEN, GREEN, GREEN_DARK, GREEN_DARK, OUTLINE,
    TRANSPARENT, OUTLINE, GREEN_DARK, GREEN_DARK, GREEN, GREEN, GREEN, GREEN, GREEN, GREEN, GREEN, GREEN, GREEN_DARK, GREEN_DARK, OUTLINE, TRANSPARENT,
    TRANSPARENT, TRANSPARENT, OUTLINE, OUTLINE, GREEN_DARK, GREEN_DARK, GREEN_DARK, GREEN_DARK, GREEN_DARK, GREEN_DARK, GREEN_DARK, GREEN_DARK, OUTLINE, OUTLINE, TRANSPARENT, TRANSPARENT,
    TRANSPARENT, TRANSPARENT, TRANSPARENT, TRANSPARENT, OUTLINE, OUTLINE, OUTLINE, OUTLINE, OUTLINE, OUTLINE, OUTLINE, OUTLINE, TRANSPARENT, TRANSPARENT, TRANSPARENT, TRANSPARENT,
];

fn draw_image(display: &mut Display, x: u32, y: u32) -> dunit::Result<()> {
    let width = IMAGE_W as u32 * SCALE + 48;
    let height = IMAGE_H as u32 * SCALE + 48;
    let origin_x = x.saturating_sub(24);
    let origin_y = y.saturating_sub(24);
    let mut pixels = alloc::vec![0x0008_1014; (width * height) as usize];
    let inner_x = x.saturating_sub(16) - origin_x;
    let inner_y = y.saturating_sub(16) - origin_y;
    for row in inner_y..inner_y + IMAGE_H as u32 * SCALE + 32 {
        let offset = (row * width + inner_x) as usize;
        pixels[offset..offset + (IMAGE_W as u32 * SCALE + 32) as usize].fill(0x0018_252d);
    }
    for row in 0..IMAGE_H {
        for col in 0..IMAGE_W {
            let color = IMAGE[row * IMAGE_W + col];
            if color != TRANSPARENT {
                for dy in 0..SCALE {
                    let offset = ((y - origin_y + row as u32 * SCALE + dy) * width
                        + x - origin_x + col as u32 * SCALE) as usize;
                    pixels[offset..offset + SCALE as usize].fill(color);
                }
            }
        }
    }
    display.present(&pixels, width, height, origin_x, origin_y)
}

#[dunit::main]
fn main() -> dunit::Result<()> {
    let code = run()?;
    dunit::process::exit(code)
}

fn run() -> dunit::Result<i32> {
    let mut display = match Display::acquire() {
        Ok(display) => display,
        Err(_) => {
            dunit::println!("image_demo: display is busy (a compositor owns it)")?;
            return Ok(1);
        }
    };
    let fb = match display.dimensions() {
        Ok(fb) => fb,
        Err(_) => {
            dunit::println!("image_demo: framebuffer unavailable")?;
            return Ok(1);
        }
    };
    let image_w = IMAGE_W as u32 * SCALE;
    let image_h = IMAGE_H as u32 * SCALE;
    let x = fb.width.saturating_sub(image_w) / 2;
    let y = fb.height.saturating_sub(image_h) / 2;
    draw_image(&mut display, x, y)?;
    dunit::println!("image_demo: rendered embedded 16x16 bitmap")?;
    Ok(0)
}
