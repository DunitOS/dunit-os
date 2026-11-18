#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use dunit::display::{Dimensions, Display};
use dunit::io::Read;
use dunit::time::{self, Duration};
mod bmp;
use bmp::{parse_bmp, render_plan, BmpInfo};

const DEFAULT_PATH: &str = "/assets/images/logo.bmp";
const MAX_BMP_SIZE: usize = 8 * 1024 * 1024;
const READ_CHUNK_SIZE: usize = 4096;
#[derive(Clone, Copy)]
struct DrawArea {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

fn draw_bmp(
    display: &mut Display,
    data: &[u8],
    info: BmpInfo,
    fb: &Dimensions,
) -> dunit::Result<DrawArea> {
    let cursor = dunit::terminal::cursor()
        .ok()
        .map(|c| (c.y, c.char_width, c.char_height));
    let plan = render_plan(info, fb.width, fb.height, cursor);
    let image_w = (plan.width * plan.dst_scale) as u32;
    let image_h = (plan.height * plan.dst_scale) as u32;
    let area = DrawArea {
        x: plan.x.saturating_sub(2),
        y: plan.y.saturating_sub(2),
        width: image_w + 4,
        height: image_h + 4,
    };

    let count = (area.width as usize)
        .checked_mul(area.height as usize)
        .filter(|count| *count <= 64 * 1024 * 1024 / 4)
        .ok_or(dunit::Error(dunit::errno::EINVAL))?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(count)
        .map_err(|_| dunit::Error(dunit::errno::ENOMEM))?;
    pixels.resize(count, 0x0008_1014);
    bmp::raster(
        &data,
        info,
        plan,
        area.x,
        area.y,
        area.width as usize,
        &mut pixels,
    )
    .ok_or(dunit::Error(dunit::errno::EINVAL))?;
    display.present(&pixels, area.width, area.height, area.x, area.y)?;

    Ok(area)
}

fn drain_stale_keys() -> dunit::Result<()> {
    let mut quiet_ticks = 0usize;
    while quiet_ticks < 8 {
        if dunit::terminal::read_scancode().is_some() {
            quiet_ticks = 0;
        } else {
            quiet_ticks += 1;
            time::sleep(Duration::from_millis(10))?;
        }
    }
    Ok(())
}

fn wait_for_key() -> dunit::Result<()> {
    drain_stale_keys()?;
    loop {
        if let Some(scancode) = dunit::terminal::read_scancode() {
            if (scancode & 0x80) == 0 {
                break;
            }
        }
        time::sleep(Duration::from_millis(20))?;
    }
    Ok(())
}

fn is_no_wait_arg(arg: &str) -> bool {
    arg.as_bytes() == b"--no-wait"
}

fn parse_args(args: &[alloc::string::String]) -> (&str, bool) {
    let mut path = DEFAULT_PATH;
    let mut wait = true;
    for arg in args.iter().skip(1) {
        if is_no_wait_arg(arg) {
            wait = false;
        } else {
            path = arg;
        }
    }
    (path, wait)
}

fn read_file(path: &str) -> Result<Vec<u8>, i32> {
    let mut file = dunit::fs::File::open(path).map_err(|_| 1)?;
    let mut data = Vec::new();
    let mut chunk = [0; READ_CHUNK_SIZE];
    loop {
        // Preserve the existing strict size boundary: a file exactly at the
        // limit also fails, rather than probing beyond the application budget.
        if data.len() >= MAX_BMP_SIZE {
            return Err(2);
        }
        let end = chunk.len().min(MAX_BMP_SIZE - data.len());
        let count = file.read(&mut chunk[..end]).map_err(|_| 3)?;
        if count == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..count]);
    }
    file.close().map_err(|_| 4)?;
    Ok(data)
}

#[dunit::main]
fn main() -> dunit::Result<()> {
    let args = dunit::args::args()?;
    let (path, wait) = parse_args(&args);
    let code = run(path, wait)?;
    dunit::process::exit(code)
}

fn run(path: &str, wait: bool) -> dunit::Result<i32> {
    let data = match read_file(path) {
        Ok(data) => data,
        Err(code) => {
            dunit::println!("bmp_viewer: read failed")?;
            return Ok(code);
        }
    };
    let info = match parse_bmp(&data) {
        Some(info) => info,
        None => {
            dunit::println!("bmp_viewer: unsupported BMP")?;
            return Ok(5);
        }
    };
    let mut display = match Display::acquire() {
        Ok(display) => display,
        Err(_) => {
            dunit::println!("bmp_viewer: display is busy (a compositor owns it)")?;
            return Ok(6);
        }
    };
    let fb = match display.dimensions() {
        Ok(fb) => fb,
        Err(_) => {
            dunit::println!("bmp_viewer: framebuffer unavailable")?;
            return Ok(6);
        }
    };
    let area = draw_bmp(&mut display, &data, info, &fb)?;
    if wait {
        dunit::println!("bmp_viewer: rendered BMP, press any key to close")?;
        wait_for_key()?;
        display.draw_rect(area.x, area.y, area.width, area.height, 0x0000_0000)?;
    } else {
        dunit::println!("bmp_viewer: rendered BMP")?;
    }
    Ok(0)
}
