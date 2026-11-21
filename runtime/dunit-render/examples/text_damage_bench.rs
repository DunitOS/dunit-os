//! Time prepared text painting through a small damage rectangle.
use dunit_render::{paint_clipped_with_prepared_inputs, CaretAffinity, Surface, TextInput};
use dunit_style::{cascade::NodeStyle, parse, Cascade};
use dunit_text::{Font, GlyphCache};
use dunit_ui::{layout, parse as parse_tree, Rect};
use std::{hint::black_box, time::Instant};

fn main() {
    const WIDTH: usize = 760;
    const HEIGHT: usize = 640;
    const FRAMES: usize = 500;
    let full = Rect {
        x: 0.,
        y: 0.,
        w: WIDTH as f32,
        h: HEIGHT as f32,
    };
    let damage = Rect {
        x: 570.,
        y: 493.,
        w: 54.,
        h: 27.,
    };
    let tree = parse_tree("Column { Input width=740 height=620 }").unwrap();
    let layout = layout(&tree, full.w, full.h);
    let mut cascade = Cascade::new();
    cascade.push(parse("Input { font-size: 16; color: #ffffff; selection-color: #334455; caret-width: 2; padding: 4; }").unwrap());
    let style = cascade.resolve(&NodeStyle::element("Input"));
    let default_style = dunit_style::Style::default();
    let font =
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap();
    let value =
        "alpha beta e\u{301} gamma delta epsilon Ж sigma phi quick brown fox ".repeat(3) + "\n";
    let value = value.repeat(100);
    let text = TextInput {
        cursor: value.len(),
        anchor: 0,
        value,
        placeholder: String::new(),
        password: false,
        multiline: true,
        soft_wrap: false,
        spinbox: None,
        affinity: CaretAffinity::Downstream,
        focused: true,
        caret_visible: false,
        scroll_x: 16.25,
        scroll_y: 704.5,
    };
    let id = tree.iter().find(|n| n.kind.tag() == "Input").unwrap().id;
    let prepared = text.prepare(&font, &style, layout.rect(id));
    let mut cache = GlyphCache::default();
    let mut pixels = vec![0; WIDTH * HEIGHT];
    let mut paint = |pixels: &mut [u32], clip| {
        paint_clipped_with_prepared_inputs(
            &tree,
            &layout,
            &|node| {
                if node == id {
                    style.clone()
                } else {
                    default_style.clone()
                }
            },
            &font,
            &mut Surface::with_glyph_cache(pixels, WIDTH, HEIGHT, &mut cache),
            clip,
            &|node| (node == id).then_some(&text),
            &|_| None,
            &|_| None,
            &|_| None,
            &|_| None,
            &|node| (node == id).then_some(&prepared),
        )
        .unwrap();
    };
    let mut reference = vec![0; pixels.len()];
    paint(&mut reference, full);
    paint(&mut pixels, damage);
    let mut hash = 0xcbf29ce484222325_u64;
    for (index, &pixel) in pixels.iter().enumerate() {
        let x = (index % WIDTH) as f32;
        let y = (index / WIDTH) as f32;
        let inside =
            x >= damage.x && x < damage.x + damage.w && y >= damage.y && y < damage.y + damage.h;
        assert_eq!(pixel, if inside { reference[index] } else { 0 });
        hash = (hash ^ u64::from(pixel)).wrapping_mul(0x100000001b3);
    }
    assert!(pixels.iter().any(|&pixel| pixel == 0xffffffff));
    for _ in 0..20 {
        paint(&mut pixels, damage);
    }
    let samples: Vec<_> = (0..5)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..FRAMES {
                paint(black_box(&mut pixels), damage);
            }
            start.elapsed().as_micros()
        })
        .collect();
    println!("{{\"frames\":{FRAMES},\"checksum\":\"{hash:016x}\",\"samples_us\":{samples:?}}}");
}
