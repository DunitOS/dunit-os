//! Benchmark repeated paint of a long label through a narrow damage clip.
use dunit_render::{paint_clipped, Surface};
use dunit_style::{cascade::NodeStyle, parse, Cascade};
use dunit_text::{Font, GlyphCache};
use dunit_ui::{layout, parse as parse_tree, Rect};
use std::{hint::black_box, time::Instant};

fn main() {
    const WIDTH: usize = 240;
    const HEIGHT: usize = 36;
    const PAINTS: usize = 1000;
    let label = "long label Åp fJ e\u{301} Ж with mixed glyphs ".repeat(16);
    let tree = parse_tree(&format!(
        r#"Column {{ Text width=240 height=36 "{label}" }}"#
    ))
    .unwrap();
    let layout = layout(&tree, WIDTH as f32, HEIGHT as f32);
    let font =
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap();
    let mut cascade = Cascade::new();
    cascade.push(parse("Text { font-size: 16; color: #ffffff; }").unwrap());
    let style = |id| cascade.resolve(&NodeStyle::element(tree.node(id).kind.tag()));
    let clip = Rect {
        x: 11.,
        y: 0.,
        w: 57.,
        h: HEIGHT as f32,
    };
    let full = Rect {
        x: 0.,
        y: 0.,
        w: WIDTH as f32,
        h: HEIGHT as f32,
    };
    let mut pixels = vec![0; WIDTH * HEIGHT];
    let mut reference = pixels.clone();
    let mut cache = GlyphCache::default();
    let mut draw = |pixels: &mut [u32], rect| {
        let mut surface = Surface::with_glyph_cache(pixels, WIDTH, HEIGHT, &mut cache);
        paint_clipped(&tree, &layout, &style, &font, &mut surface, rect).unwrap();
    };
    draw(&mut reference, full);
    draw(&mut pixels, clip);
    let mut checksum = 0xcbf29ce484222325_u64;
    assert!(pixels.iter().any(|&pixel| pixel != 0));
    for (index, &pixel) in pixels.iter().enumerate() {
        let x = index % WIDTH;
        let expected = if (11..68).contains(&x) {
            reference[index]
        } else {
            0
        };
        assert_eq!(pixel, expected);
        checksum = (checksum ^ u64::from(pixel)).wrapping_mul(0x100000001b3);
    }
    for _ in 0..20 {
        draw(&mut pixels, clip);
    }
    let samples: Vec<_> = (0..5)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..PAINTS {
                draw(black_box(&mut pixels), clip);
            }
            start.elapsed().as_micros()
        })
        .collect();
    println!("{{\"paints\":{PAINTS},\"checksum\":\"{checksum:016x}\",\"samples_us\":{samples:?}}}");
}
