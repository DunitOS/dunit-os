//! Bounded shared image decoders for BMP, PNG, and Baseline Sequential JPEG.
pub mod bmp;
pub mod jpeg;
pub mod png;

use crate::{Error, Image};

pub const MAX_IMAGE_FILE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_IMAGE_DIM: usize = 4096;
pub const MAX_IMAGE_PIXELS: usize = 4096 * 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Bmp,
    Png,
    Jpeg,
    Svg,
    Rgba32,
}

pub fn sniff_format(data: &[u8]) -> Option<ImageFormat> {
    if data.starts_with(b"BM") {
        Some(ImageFormat::Bmp)
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(ImageFormat::Png)
    } else if data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        Some(ImageFormat::Jpeg)
    } else if data.len() == 32 * 32 * 4 {
        Some(ImageFormat::Rgba32)
    } else {
        let head = core::str::from_utf8(&data[..data.len().min(512)]).ok()?;
        let trimmed = head.trim_start();
        if trimmed.starts_with("<svg") || trimmed.starts_with("<?xml") {
            Some(ImageFormat::Svg)
        } else {
            None
        }
    }
}

pub fn check_dimensions(width: usize, height: usize) -> Result<usize, Error> {
    if width == 0 || height == 0 || width > MAX_IMAGE_DIM || height > MAX_IMAGE_DIM {
        return Err(Error::Format);
    }
    let count = width.checked_mul(height).ok_or(Error::Limit)?;
    if count > MAX_IMAGE_PIXELS {
        return Err(Error::Limit);
    }
    Ok(count)
}

pub fn decode(data: &[u8]) -> Result<Image, Error> {
    if data.len() > MAX_IMAGE_FILE_BYTES {
        return Err(Error::Limit);
    }
    match sniff_format(data).ok_or(Error::Format)? {
        ImageFormat::Bmp => bmp::decode_bmp(data),
        ImageFormat::Png => png::decode_png(data),
        ImageFormat::Jpeg => jpeg::decode_jpeg(data),
        ImageFormat::Rgba32 => {
            let pixels = data
                .chunks_exact(4)
                .map(|c| {
                    ((c[3] as u32) << 24)
                        | ((c[0] as u32) << 16)
                        | ((c[1] as u32) << 8)
                        | c[2] as u32
                })
                .collect();
            Ok(Image {
                width: 32,
                height: 32,
                pixels,
            })
        }
        ImageFormat::Svg => {
            let text = core::str::from_utf8(data).map_err(|_| Error::Format)?;
            let pixels = dunit_svg::render(text, 256, 256).map_err(Error::Svg)?;
            Ok(Image {
                width: 256,
                height: 256,
                pixels,
            })
        }
    }
}
