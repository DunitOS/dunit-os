use dunit_component::{Attrs, Font, Renderer, Sizing, Theme, View};
const DARK: &str = include_str!("../../../assets/dwm/sdk/dark.dss");
const BASE: &str = include_str!("../../../assets/dwm/sdk/base.dss");
fn renderer(css: &str) -> Renderer {
    let theme = Theme::parse(DARK, BASE, css).unwrap();
    Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        theme.cascade,
        theme.background,
    )
}
fn scene(width: f32) -> View<()> {
    View::stack(
        "root",
        vec![
            View::text("back", "Blur background").layout(Attrs {
                width: Sizing::Px(width),
                height: Sizing::Px(100.),
                ..Attrs::default()
            }),
            View::column("glass", vec![View::text("foreground", "Foreground")]).layout(Attrs {
                width: Sizing::Px(160.),
                height: Sizing::Px(80.),
                ..Attrs::default()
            }),
        ],
    )
}
#[test]
fn changed_backdrop_rebuilds_scene_without_blur_feedback_and_noop_stays_free() {
    let css = "Text#back {background-gradient:horizontal #ff8800 #0033ff;}
        Column#glass {background:#ffffff20;radius:24;clip:rounded;backdrop-blur:4;}
        Text#foreground {background:#ffffff40;}";
    let mut retained = renderer(css);
    retained.render(&scene(150.), 240, 140).unwrap();
    let first = retained.pixels().to_vec();
    for width in [80., 180., 150.] {
        assert_eq!(
            retained
                .render(&scene(width), 240, 140)
                .unwrap()
                .damaged_pixels,
            240 * 140
        );
        let mut full = renderer(css);
        full.render(&scene(width), 240, 140).unwrap();
        assert_eq!(retained.pixels(), full.pixels());
        assert_eq!(
            retained
                .render(&scene(width), 240, 140)
                .unwrap()
                .damaged_pixels,
            0
        );
    }
    assert_eq!(retained.pixels(), first);
}
#[test]
fn rounded_parent_mask_applies_to_its_background_and_all_children() {
    let css = "Column#glass {background:#ffffffff;radius:24;clip:rounded;}
        Text#foreground {background:#ffffffff;}";
    // Root padding is 12. The glass corner must preserve the layer below it.
    let mut beneath = renderer("Text#back {background:$accent;}");
    let bare = View::<()>::stack(
        "root",
        vec![View::text("back", "Blur background").layout(Attrs {
            width: Sizing::Px(150.),
            height: Sizing::Px(100.),
            ..Attrs::default()
        })],
    );
    beneath.render(&bare, 240, 140).unwrap();
    // Pixel outside the rounded shape is restored, including an opaque child.
    let mut with_back = renderer(&format!("{css} Text#back {{background:$accent;}}"));
    with_back.render(&scene(150.), 240, 140).unwrap();
    assert_eq!(
        with_back.pixels()[12 * 240 + 12],
        beneath.pixels()[12 * 240 + 12]
    );
    assert_eq!(with_back.pixels()[50 * 240 + 80], 0xffffffff);
}
#[test]
fn invalid_blur_and_clip_styles_are_rejected_before_painting() {
    for css in [
        "Text{backdrop-blur:-1;}",
        "Text{backdrop-blur:65;}",
        "Text{backdrop-blur:none;}",
        "Text{clip:circle;}",
    ] {
        assert!(Theme::parse(DARK, BASE, css).is_err());
    }
}

#[test]
fn rounded_mask_without_blur_keeps_partial_damage_correct() {
    let css="Text#back {background:$accent;} Column#glass {background:#ffffff20;radius:24;clip:rounded;}";
    let mut retained = renderer(css);
    retained.render(&scene(150.), 240, 140).unwrap();
    let stats = retained.render(&scene(80.), 240, 140).unwrap();
    assert!(stats.damaged_pixels > 0 && stats.damaged_pixels < 240 * 140);
    let mut full = renderer(css);
    full.render(&scene(80.), 240, 140).unwrap();
    assert_eq!(retained.pixels(), full.pixels());
}
