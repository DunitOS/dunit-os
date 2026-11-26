//! Shared checked VFS image loading; SVG/BMP/PNG/JPEG decoding stays in userspace.
#![cfg_attr(not(test), no_std)]
extern crate alloc;
use alloc::{format, vec::Vec};

pub mod codecs;
mod theme;
#[cfg(test)]
mod tests;

pub use codecs::{decode, ImageFormat, MAX_IMAGE_DIM, MAX_IMAGE_FILE_BYTES, MAX_IMAGE_PIXELS};
pub use theme::IconTheme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Missing,
    Io,
    Limit,
    Format,
    Svg(dunit_svg::Error),
}

#[cfg(not(test))]
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

#[cfg(test)]
fn read(path: &str, cap: usize) -> Result<Vec<u8>, Error> {
    let bytes = std::fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::Missing
        } else {
            Error::Io
        }
    })?;
    if bytes.len() > cap {
        return Err(Error::Limit);
    }
    Ok(bytes)
}

/// SVG is rasterized at the requested dimensions. Raster images decode at intrinsic size.
pub fn load(path: &str, width: u32, height: u32) -> Result<Image, Error> {
    if path.ends_with(".svg") {
        let data = read(path, 1024 * 1024)?;
        svg(&data, width, height)
    } else if path.ends_with(".rgba") {
        let data = read(path, 32 * 32 * 4)?;
        if data.len() != 32 * 32 * 4 {
            return Err(Error::Format);
        }
        codecs::decode(&data)
    } else if path.ends_with(".bmp")
        || path.ends_with(".png")
        || path.ends_with(".jpg")
        || path.ends_with(".jpeg")
    {
        load_auto(path)
    } else {
        Err(Error::Format)
    }
}

/// Decode BMP, PNG, JPEG, SVG, or RGBA from a VFS path at natural image dimensions.
pub fn load_auto(path: &str) -> Result<Image, Error> {
    let data = read(path, MAX_IMAGE_FILE_BYTES)?;
    codecs::decode(&data)
}

fn svg(data: &[u8], width: u32, height: u32) -> Result<Image, Error> {
    let text = core::str::from_utf8(data).map_err(|_| Error::Format)?;
    let pixels = dunit_svg::render(text, width, height).map_err(Error::Svg)?;
    Ok(Image {
        width: width as usize,
        height: height as usize,
        pixels,
    })
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
