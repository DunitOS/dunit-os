//! Validated borrowed BMP24/BMP32 layout; alpha is ignored like native scanout.
#[derive(Clone, Copy)]
pub struct BmpInfo {
    pub data_offset: usize,
    pub width: usize,
    pub height: usize,
    pub bpp: u16,
    pub top_down: bool,
    pub row_stride: usize,
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
    ]))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
        *data.get(offset + 2)?,
        *data.get(offset + 3)?,
    ]))
}

fn read_i32(data: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes([
        *data.get(offset)?,
        *data.get(offset + 1)?,
        *data.get(offset + 2)?,
        *data.get(offset + 3)?,
    ]))
}

pub fn parse_bmp(data: &[u8]) -> Option<BmpInfo> {
    if data.len() < 54 || data.get(0) != Some(&b'B') || data.get(1) != Some(&b'M') {
        return None;
    }

    let data_offset = read_u32(data, 10)? as usize;
    let dib_size = read_u32(data, 14)?;
    if dib_size < 40 {
        return None;
    }

    let header_end = 14usize.checked_add(dib_size as usize)?;
    if header_end > data.len() || data_offset < header_end {
        return None;
    }

    let width = read_i32(data, 18)?;
    let height = read_i32(data, 22)?;
    let planes = read_u16(data, 26)?;
    let bpp = read_u16(data, 28)?;
    let compression = read_u32(data, 30)?;
    if width <= 0 || height == 0 || planes != 1 || compression != 0 {
        return None;
    }
    if bpp != 24 && bpp != 32 {
        return None;
    }

    let abs_height = height.unsigned_abs() as usize;
    let width = width as usize;
    let bytes_per_pixel = (bpp / 8) as usize;
    let row_stride = width.checked_mul(bytes_per_pixel)?.checked_add(3)? & !3;
    let pixel_bytes = row_stride.checked_mul(abs_height)?;
    if data_offset.checked_add(pixel_bytes)? > data.len() {
        return None;
    }

    Some(BmpInfo {
        data_offset,
        width,
        height: abs_height,
        bpp,
        top_down: height < 0,
        row_stride,
    })
}

const PAD_X: u32 = 16;
const PAD_Y: u32 = 4;
const MAX_UPSCALE: usize = 12;
const MIN_USEFUL_HEIGHT_NUM: u32 = 2;
const MIN_USEFUL_HEIGHT_DEN: u32 = 3;

#[derive(Clone, Copy)]
pub struct RenderPlan {
    pub x: u32,
    pub y: u32,
    pub width: usize,
    pub height: usize,
    pub src_step: usize,
    pub dst_scale: usize,
}

fn ceil_div(value: usize, by: usize) -> usize {
    if by == 0 {
        return value;
    }
    value / by + usize::from(value % by != 0)
}

pub fn render_plan(
    info: BmpInfo,
    fb_width: u32,
    fb_height: u32,
    cursor: Option<(u32, u32, u32)>,
) -> RenderPlan {
    let has_cursor = cursor.is_some();
    let (cursor_y, char_width, char_height) = cursor.unwrap_or((0, 8, 16));
    let x0 = if has_cursor {
        char_width.saturating_mul(2)
    } else {
        PAD_X
    };
    let prompt_y = cursor_y.saturating_add(PAD_Y);
    let min_useful_h = fb_height.saturating_mul(MIN_USEFUL_HEIGHT_NUM) / MIN_USEFUL_HEIGHT_DEN;
    let y0 = if has_cursor && fb_height.saturating_sub(prompt_y) >= min_useful_h {
        prompt_y
    } else {
        PAD_Y
    };
    let max_w = fb_width.saturating_sub(x0 + PAD_X).max(1) as usize;
    let max_h = fb_height.saturating_sub(y0 + char_height + PAD_Y).max(1) as usize;
    let src_step = ceil_div(info.width, max_w)
        .max(ceil_div(info.height, max_h))
        .max(1);
    let width = (info.width / src_step).max(1);
    let height = (info.height / src_step).max(1);
    let max_scale = (max_w / width).min(max_h / height).max(1);
    let target_width = (max_w * 2 / 3).max(width);
    let dst_scale = ceil_div(target_width, width)
        .max(1)
        .min(max_scale)
        .min(MAX_UPSCALE);

    RenderPlan {
        x: x0,
        y: y0,
        width,
        height,
        src_step,
        dst_scale,
    }
}

impl BmpInfo {
    pub fn pixel(&self, data: &[u8], x: usize, y: usize) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let file_y = if self.top_down {
            y
        } else {
            self.height - 1 - y
        };
        let offset = self
            .data_offset
            .checked_add(file_y.checked_mul(self.row_stride)?)?
            .checked_add(x.checked_mul((self.bpp / 8) as usize)?)?;
        let b = *data.get(offset)? as u32;
        let g = *data.get(offset.checked_add(1)?)? as u32;
        let r = *data.get(offset.checked_add(2)?)? as u32;
        Some((r << 16) | (g << 8) | b)
    }
}

/// Paint the scaled image over the caller's border/background allocation.
pub fn raster(
    data: &[u8],
    image: BmpInfo,
    plan: RenderPlan,
    origin_x: u32,
    origin_y: u32,
    stride: usize,
    output: &mut [u32],
) -> Option<()> {
    let left = plan.x.checked_sub(origin_x)? as usize;
    let top = plan.y.checked_sub(origin_y)? as usize;
    let image_width = plan.width.checked_mul(plan.dst_scale)?;
    let image_height = plan.height.checked_mul(plan.dst_scale)?;
    if stride == 0
        || left.checked_add(image_width)? > stride
        || top.checked_add(image_height)?.checked_mul(stride)? > output.len()
    {
        return None;
    }
    for y in 0..plan.height {
        for x in 0..plan.width {
            let color = image.pixel(
                data,
                x.checked_mul(plan.src_step)?,
                y.checked_mul(plan.src_step)?,
            )?;
            for row in 0..plan.dst_scale {
                let offset = (top + y * plan.dst_scale + row) * stride + left + x * plan.dst_scale;
                output[offset..offset + plan.dst_scale].fill(color);
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(bpp: u16, top_down: bool) -> Vec<u8> {
        let stride = if bpp == 24 { 8 } else { 8 };
        let mut bytes = vec![0; 54 + stride * 2];
        bytes[..2].copy_from_slice(b"BM");
        bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&2i32.to_le_bytes());
        bytes[22..26].copy_from_slice(&(if top_down { -2i32 } else { 2i32 }).to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&bpp.to_le_bytes());
        // Logical rows: red/green, blue/white. Nonzero row padding and alpha
        // distinguish color decoding from accidental adjacent-byte reads.
        for (y, row) in [[0xff0000u32, 0x00ff00], [0x0000ff, 0xffffff]]
            .iter()
            .enumerate()
        {
            let file_y = if top_down { y } else { 1 - y };
            for (x, color) in row.iter().enumerate() {
                let offset = 54 + file_y * stride + x * (bpp / 8) as usize;
                bytes[offset..offset + 3].copy_from_slice(&[
                    *color as u8,
                    (*color >> 8) as u8,
                    (*color >> 16) as u8,
                ]);
                if bpp == 32 {
                    bytes[offset + 3] = 0x5a;
                }
            }
            if bpp == 24 {
                bytes[54 + file_y * stride + 6..54 + file_y * stride + 8].fill(0xa5);
            }
        }
        bytes
    }

    #[test]
    fn padded_bmp24_and_bmp32_have_identical_logical_pixels_in_both_directions() {
        for bpp in [24, 32] {
            for top_down in [false, true] {
                let bytes = fixture(bpp, top_down);
                let image = parse_bmp(&bytes).unwrap();
                assert_eq!((image.width, image.height, image.row_stride), (2, 2, 8));
                let pixels: Vec<_> = (0..2)
                    .flat_map(|y| (0..2).map(move |x| (x, y)))
                    .map(|(x, y)| image.pixel(&bytes, x, y).unwrap())
                    .collect();
                assert_eq!(pixels, [0xff0000, 0x00ff00, 0x0000ff, 0xffffff]);
                assert_eq!(image.pixel(&bytes, 2, 0), None);
            }
        }
    }

    #[test]
    fn bulk_raster_preserves_border_scaling_sampling_and_rgb_pixels() {
        for bpp in [24, 32] {
            for top_down in [false, true] {
                let data = fixture(bpp, top_down);
                let image = parse_bmp(&data).unwrap();
                let plan = RenderPlan {
                    x: 18,
                    y: 6,
                    width: 2,
                    height: 2,
                    src_step: 1,
                    dst_scale: 2,
                };
                let background = 0x081014;
                let mut output = vec![background; 64];
                raster(&data, image, plan, 16, 4, 8, &mut output).unwrap();
                let expected = [
                    [background; 8],
                    [background; 8],
                    [
                        background, background, 0xff0000, 0xff0000, 0x00ff00, 0x00ff00, background,
                        background,
                    ],
                    [
                        background, background, 0xff0000, 0xff0000, 0x00ff00, 0x00ff00, background,
                        background,
                    ],
                    [
                        background, background, 0x0000ff, 0x0000ff, 0xffffff, 0xffffff, background,
                        background,
                    ],
                    [
                        background, background, 0x0000ff, 0x0000ff, 0xffffff, 0xffffff, background,
                        background,
                    ],
                    [background; 8],
                    [background; 8],
                ];
                assert_eq!(output, expected.concat());
                let sampled = RenderPlan {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    src_step: 2,
                    dst_scale: 1,
                };
                let mut output = [0];
                raster(&data, image, sampled, 0, 0, 1, &mut output).unwrap();
                assert_eq!(output, [0xff0000]);
                assert!(raster(&data, image, plan, 16, 4, 1, &mut output).is_none());
            }
        }
    }

    #[test]
    fn malformed_headers_dimensions_and_truncated_pixels_are_rejected() {
        let original = fixture(24, false);
        for length in 0..original.len() {
            assert!(
                parse_bmp(&original[..length]).is_none(),
                "truncation {length}"
            );
        }
        for (offset, replacement) in [
            (0, vec![0]),
            (10, 53u32.to_le_bytes().to_vec()),
            (14, u32::MAX.to_le_bytes().to_vec()),
            (18, 0i32.to_le_bytes().to_vec()),
            (18, (-1i32).to_le_bytes().to_vec()),
            (22, 0i32.to_le_bytes().to_vec()),
            (22, i32::MIN.to_le_bytes().to_vec()),
            (26, 2u16.to_le_bytes().to_vec()),
            (28, 16u16.to_le_bytes().to_vec()),
            (30, 1u32.to_le_bytes().to_vec()),
        ] {
            let mut bytes = original.clone();
            bytes[offset..offset + replacement.len()].copy_from_slice(&replacement);
            assert!(parse_bmp(&bytes).is_none(), "invalid field at {offset}");
        }
    }

    #[test]
    fn fitting_preserves_cursor_placement_scale_cap_and_downsample() {
        let bytes = fixture(24, true);
        let mut image = parse_bmp(&bytes).unwrap();
        let below_prompt = render_plan(image, 1024, 768, Some((100, 8, 16)));
        assert_eq!(
            (
                below_prompt.x,
                below_prompt.y,
                below_prompt.src_step,
                below_prompt.dst_scale
            ),
            (16, 104, 1, 12)
        );
        let full_height = render_plan(image, 1024, 768, Some((700, 8, 16)));
        assert_eq!((full_height.x, full_height.y), (16, 4));
        image.width = 4000;
        image.height = 2000;
        let reduced = render_plan(image, 1024, 768, None);
        assert_eq!(
            (
                reduced.width,
                reduced.height,
                reduced.src_step,
                reduced.dst_scale
            ),
            (800, 400, 5, 1)
        );
        assert!(reduced.width * reduced.dst_scale <= 992);
        assert!(reduced.height * reduced.dst_scale <= 744);
    }
}
