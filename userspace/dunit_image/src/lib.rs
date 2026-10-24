//! Shared checked VFS image loading; SVG rendering stays entirely in userspace.
#![no_std]
extern crate alloc;
use alloc::{format, vec::Vec};

pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}
#[derive(Debug, Clone, Copy)]
pub enum Error {
    Missing,
    Io,
    Limit,
    Format,
    Svg(dunit_svg::Error),
}

fn read(path: &str, cap: usize) -> Result<Vec<u8>, Error> {
    let fd = libdunit::open(path, libdunit::OPEN_READ);
    if fd < 0 {
        return Err(if fd == -2 { Error::Missing } else { Error::Io });
    }
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    let result = loop {
        let n = libdunit::read(fd as usize, &mut chunk);
        if n < 0 {
            break Err(Error::Io);
        }
        if n == 0 {
            break Ok(data);
        }
        if data.len() + n as usize > cap {
            break Err(Error::Limit);
        }
        data.extend_from_slice(&chunk[..n as usize]);
    };
    libdunit::close(fd as usize);
    result
}

/// SVG is rasterized at the requested dimensions. Legacy raw RGBA stays 32×32.
pub fn load(path: &str, width: u32, height: u32) -> Result<Image, Error> {
    if path.ends_with(".svg") {
        let data = read(path, 1024 * 1024)?;
        let text = core::str::from_utf8(&data).map_err(|_| Error::Format)?;
        let pixels = dunit_svg::render(text, width, height).map_err(Error::Svg)?;
        Ok(Image {
            width: width as usize,
            height: height as usize,
            pixels,
        })
    } else if path.ends_with(".rgba") {
        let data = read(path, 32 * 32 * 4)?;
        if data.len() != 32 * 32 * 4 {
            return Err(Error::Format);
        }
        let pixels = data
            .chunks_exact(4)
            .map(|c| {
                ((c[3] as u32) << 24) | ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
            })
            .collect();
        Ok(Image {
            width: 32,
            height: 32,
            pixels,
        })
    } else {
        Err(Error::Format)
    }
}

/// A present but invalid SVG is reported, never silently replaced by a bitmap.
pub fn icon(directory: &str, stem: &str, side: u32) -> Result<Image, Error> {
    if stem.is_empty() || stem.contains(['/', '\\']) || stem == ".." {
        return Err(Error::Format);
    }
    let path = format!("{}{}.svg", directory, stem);
    match load(&path, side, side) {
        Ok(image) => Ok(image),
        Err(Error::Missing) => load(&format!("{}{}.rgba", directory, stem), side, side),
        Err(error) => Err(error),
    }
}

impl Image {
    pub fn rgba(&self) -> Vec<u8> {
        self.pixels
            .iter()
            .flat_map(|&p| [(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8])
            .collect()
    }
}
