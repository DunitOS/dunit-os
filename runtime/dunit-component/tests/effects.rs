use dunit_component::{Attrs, Font, Renderer, Sizing, Theme, View};
const DARK: &str = include_str!("../../../assets/dwm/sdk/dark.dss");
const BASE: &str = include_str!("../../../assets/dwm/sdk/base.dss");
fn renderer(overlay: &str) -> Renderer {
    let theme = Theme::parse(DARK, BASE, overlay).unwrap();
    Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        theme.cascade,
        theme.background,
    )
}
fn scene(width: f32) -> View<()> {
    View::column(
        "root",
        vec![View::text("card", "").layout(Attrs {
            width: Sizing::Px(width),
            height: Sizing::Px(45.),
            ..Attrs::default()
        })],
    )
}
#[test]
fn shrinking_effect_box_cleans_old_shadow_and_matches_full_render() {
    for offset in ["-7 -5", "5 7"] {
        let style=format!("Column {{padding:20;}} Text#card {{radius:12; background-gradient:horizontal #ff000080 #0000ffff; shadow:{offset} 10 #00000080; border-width:2;border-color:#ffffff;}}");
        let mut incremental = renderer(&style);
        incremental.render(&scene(120.), 200, 120).unwrap();
        let before = incremental.pixels().to_vec();
        let stats = incremental.render(&scene(55.), 200, 120).unwrap();
        assert!(stats.damaged_pixels > 120 * 45 && stats.damaged_pixels < 200 * 120);
        let mut full = renderer(&style);
        full.render(&scene(55.), 200, 120).unwrap();
        assert_eq!(incremental.pixels(), full.pixels());
        let damage = stats.damage.unwrap();
        for y in 0..120 {
            for x in 0..200 {
                if x < damage.x
                    || x >= damage.x + damage.width
                    || y < damage.y
                    || y >= damage.y + damage.height
                {
                    assert_eq!(
                        before[(y * 200 + x) as usize],
                        incremental.pixels()[(y * 200 + x) as usize]
                    );
                }
            }
        }
        assert_eq!(
            incremental
                .render(&scene(55.), 200, 120)
                .unwrap()
                .damaged_pixels,
            0
        );
    }
}
#[test]
fn effect_value_validation_rejects_incomplete_and_unbounded_values() {
    for value in ["diagonal #fff #000", "vertical #fff", "vertical bad #000"] {
        assert!(
            Theme::parse(DARK, BASE, &format!("Text{{background-gradient:{value};}}")).is_err()
        );
    }
    for value in [
        "0 0 -1 #000",
        "0 0 65 #000",
        "1025 0 4 #000",
        "0 0 2",
        "red",
    ] {
        assert!(Theme::parse(DARK, BASE, &format!("Text{{shadow:{value};}}")).is_err());
    }
    assert!(Theme::parse(DARK, BASE, "Text {shadow:none; background-gradient:none;}").is_ok());
}

#[test]
fn shadow_repaint_preserves_ancestor_clips_and_overlapping_siblings() {
    let overlay = "Stack {padding:20;} Column {padding:5; background:$surface;}
        Text#card {radius:12; background-gradient:horizontal #ff000080 #0000ff80;
                    shadow:-8 6 12 #00000080;}
        Text#top {radius:8; background:#00ff0080;}";
    let scene = |width| {
        View::<()>::stack(
            "root",
            vec![
                View::column(
                    "clip",
                    vec![View::text("card", "").layout(Attrs {
                        width: Sizing::Px(width),
                        height: Sizing::Px(55.),
                        ..Attrs::default()
                    })],
                )
                .layout(Attrs {
                    width: Sizing::Px(140.),
                    height: Sizing::Px(75.),
                    ..Attrs::default()
                }),
                View::text("top", "").layout(Attrs {
                    width: Sizing::Px(65.),
                    height: Sizing::Px(65.),
                    ..Attrs::default()
                }),
            ],
        )
    };
    let mut incremental = renderer(overlay);
    incremental.render(&scene(220.), 220, 140).unwrap();
    incremental.render(&scene(70.), 220, 140).unwrap();
    let mut full = renderer(overlay);
    full.render(&scene(70.), 220, 140).unwrap();
    assert_eq!(incremental.pixels(), full.pixels());
    // A shrinking node cannot alter pixels beyond its ancestor clip.
    let background = 0xff121820;
    assert_eq!(incremental.pixels()[60 * 220 + 180], background);
}
