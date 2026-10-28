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

fn dependency_scene(changed: bool, probe_first: bool) -> View<()> {
    let probe = View::stack(
        "probe-position",
        vec![View::text("probe", "")
            .class(if changed { "changed" } else { "idle" })
            .layout(Attrs {
                width: Sizing::Px(4.),
                height: Sizing::Px(4.),
                ..Attrs::default()
            })],
    );
    let glass = View::stack(
        "glass-position",
        vec![View::stack(
            "glass",
            vec![View::text("nested", "").layout(Attrs {
                width: Sizing::Px(30.),
                height: Sizing::Px(20.),
                ..Attrs::default()
            })],
        )
        .layout(Attrs {
            width: Sizing::Px(80.),
            height: Sizing::Px(60.),
            ..Attrs::default()
        })],
    );
    View::stack(
        "root",
        if probe_first {
            vec![probe, glass]
        } else {
            vec![glass, probe]
        },
    )
}

#[test]
fn disjoint_edits_stay_partial_but_filter_outputs_and_halos_rebuild() {
    // A two-pass radius-8 filter at (60,60)..(140,120) samples 16px
    // beyond each side. Include halo-only changes, diagonal neighbours,
    // fractional padding, both paint orders and a nested rounded filter.
    for (x, y, partial) in [
        (5., 60., true),
        (38., 80., true),
        (45., 80., false),
        (60., 60., false),
        (145., 80., false),
        (160., 80., true),
        (80., 38., true),
        (80., 45., false),
        (80., 125., false),
        (80., 140., true),
        (45., 45., false),
        (160., 140., true),
        (38.25, 80.5, true),
        (44.75, 80.5, false),
    ] {
        for probe_first in [true, false] {
            let css = format!(
                "Stack {{padding:0;background:#00000000;}}
                Stack#root {{background-gradient:horizontal #ff8800 #0033ff;}}
                Stack#probe-position {{padding:{y} 0 0 {x};}}
                Stack#glass-position {{padding:60 0 0 60;}}
                Stack#glass {{backdrop-blur:8;clip:rounded;radius:20;background:#ffffff20;}}
                Text#nested {{backdrop-blur:3;radius:8;background:#ffffff40;}}
                Text#probe.idle {{background:#ff0000;}}
                Text#probe.changed {{background:#00ff00;}}"
            );
            let mut retained = renderer(&css);
            retained
                .render(&dependency_scene(false, probe_first), 220, 180)
                .unwrap();
            let original = retained.pixels().to_vec();
            for changed in [true, false, true, false] {
                let view = dependency_scene(changed, probe_first);
                let stats = retained.render(&view, 220, 180).unwrap();
                assert!(stats.damaged_pixels > 0);
                assert_eq!(
                    stats.damaged_pixels < 220 * 180,
                    partial,
                    "probe=({x},{y}), first={probe_first}"
                );
                let mut full = renderer(&css);
                full.render(&view, 220, 180).unwrap();
                assert_eq!(
                    retained.pixels(),
                    full.pixels(),
                    "probe=({x},{y}), first={probe_first}"
                );
                assert_eq!(retained.render(&view, 220, 180).unwrap().damaged_pixels, 0);
            }
            assert_eq!(retained.pixels(), original);
        }
    }
}

#[test]
fn rounded_zero_blur_does_not_create_a_filter_dependency() {
    for radius in [0., 0.49] {
        let css = format!(
            "Text#back {{background:$accent;}}
            Column#glass {{backdrop-blur:{radius};clip:rounded;radius:24;background:#ffffff20;}}"
        );
        let mut retained = renderer(&css);
        retained.render(&scene(150.), 240, 140).unwrap();
        let stats = retained.render(&scene(80.), 240, 140).unwrap();
        assert!(stats.damaged_pixels > 0 && stats.damaged_pixels < 240 * 140);
        let mut full = renderer(&css);
        full.render(&scene(80.), 240, 140).unwrap();
        assert_eq!(retained.pixels(), full.pixels());
    }
}

#[test]
fn enabling_and_removing_a_filter_checks_both_snapshots() {
    let css = "Stack {padding:0;background:#00000000;}
        Stack#root {background-gradient:horizontal #ff8800 #0033ff;}
        Stack#probe-position {padding:10 0 0 10;}
        Stack#glass-position {padding:60 0 0 60;}
        Text#probe.idle {background:#ffffff20;backdrop-blur:0;}
        Text#probe.changed {background:#ffffff20;backdrop-blur:0.5;}";
    let mut retained = renderer(css);
    retained
        .render(&dependency_scene(false, true), 220, 180)
        .unwrap();
    for changed in [true, false, true, false] {
        let view = dependency_scene(changed, true);
        assert_eq!(
            retained.render(&view, 220, 180).unwrap().damaged_pixels,
            220 * 180
        );
        let mut full = renderer(css);
        full.render(&view, 220, 180).unwrap();
        assert_eq!(retained.pixels(), full.pixels());
    }
}
