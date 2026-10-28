use dunit_component::{Attrs, Font, Renderer, Sizing, Theme, UiError, View};
const DARK: &str = include_str!("../../../assets/dwm/sdk/dark.dss");
const BASE: &str = include_str!("../../../assets/dwm/sdk/base.dss");
const STYLE: &str = "Text#card {background:#000000; color:#ffffff;radius:0;
    transition:background 100ms linear, radius 100ms ease-in;}
    Text#card.active {background:#ffffff;radius:20;}";
fn renderer(style: &str) -> Renderer {
    let theme = Theme::parse(DARK, BASE, style).unwrap();
    Renderer::new(
        Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap(),
        theme.cascade,
        theme.background,
    )
}
fn card(active: bool) -> View<()> {
    let card = View::text("card", "").layout(Attrs {
        width: Sizing::Px(100.),
        height: Sizing::Px(60.),
        ..Attrs::default()
    });
    if active {
        card.class("active")
    } else {
        card
    }
}
fn scene(active: bool) -> View<()> {
    View::column("root", vec![card(active)])
}
fn center(r: &Renderer) -> u32 {
    r.pixels()[40 * 160 + 50]
}
#[test]
fn timed_styles_have_exact_midpoints_and_endpoints_with_partial_damage() {
    let mut r = renderer(STYLE);
    r.render_at(&scene(false), 160, 100, 0).unwrap();
    r.render_at(&scene(true), 160, 100, 10).unwrap();
    assert!(r.is_animating());
    assert_eq!(center(&r), 0xff000000);
    let stats = r.render_at(&scene(true), 160, 100, 60).unwrap();
    assert_eq!(stats.active_transitions, 2);
    assert_eq!(stats.animation_time_ms, Some(60));
    assert!(stats.damaged_pixels > 0 && stats.damaged_pixels < 160 * 100);
    assert_eq!(center(&r), 0xff808080);
    let mut reference = renderer("Text#card {background:#808080;radius:5;}");
    reference.render(&scene(true), 160, 100).unwrap();
    assert_eq!(r.pixels(), reference.pixels());
    r.render_at(&scene(true), 160, 100, 110).unwrap();
    assert_eq!(center(&r), 0xffffffff);
    assert!(!r.is_animating());
    let mut final_frame = renderer(STYLE);
    final_frame.render(&scene(true), 160, 100).unwrap();
    assert_eq!(r.pixels(), final_frame.pixels());
    assert_eq!(
        r.render_at(&scene(true), 160, 100, 200)
            .unwrap()
            .damaged_pixels,
        0
    );
}
#[test]
fn interrupted_transition_retargets_from_current_sample_without_jumping() {
    let mut r = renderer(STYLE);
    r.render_at(&scene(false), 160, 100, 0).unwrap();
    r.render_at(&scene(true), 160, 100, 10).unwrap();
    r.render_at(&scene(true), 160, 100, 60).unwrap();
    let midpoint = r.pixels().to_vec();
    r.render_at(&scene(false), 160, 100, 60).unwrap();
    assert_eq!(r.pixels(), midpoint);
    r.render_at(&scene(false), 160, 100, 110).unwrap();
    assert_eq!(center(&r), 0xff404040);
    r.render_at(&scene(false), 160, 100, 160).unwrap();
    assert_eq!(center(&r), 0xff000000);
    assert!(!r.is_animating());
}
#[test]
fn stable_keys_survive_reordering_but_removed_or_retyped_nodes_snap() {
    let view = |active, reverse| {
        let mut children = vec![card(active), View::text("other", "")];
        if reverse {
            children.reverse();
        }
        View::<()>::column("root", children)
    };
    let mut r = renderer(STYLE);
    r.render_at(&view(false, false), 160, 100, 0).unwrap();
    r.render_at(&view(true, false), 160, 100, 10).unwrap();
    r.render_at(&view(true, true), 160, 100, 60).unwrap();
    assert!(r.is_animating());
    assert_eq!(center(&r), 0xff808080);
    let empty = View::<()>::column("root", vec![]);
    r.render_at(&empty, 160, 100, 70).unwrap();
    assert!(!r.is_animating());
    r.render_at(&scene(true), 160, 100, 80).unwrap();
    assert_eq!(center(&r), 0xffffffff);
    assert!(!r.is_animating());
    r.render_at(&scene(false), 160, 100, 90).unwrap();
    assert!(r.is_animating());
    let retyped = View::<()>::column("root", vec![View::column("card", vec![]).class("active")]);
    r.render_at(&retyped, 160, 100, 100).unwrap();
    assert!(!r.is_animating());
}
#[test]
fn cancellation_paths_settle_without_replaying() {
    for mode in 0..6 {
        let mut r = renderer(&format!("{STYLE} Text#card.quiet {{transition:none;}} Text#card.zero {{transition:background 0ms;}}"));
        r.render_at(&scene(false), 160, 100, 0).unwrap();
        r.render_at(&scene(true), 160, 100, 10).unwrap();
        match mode {
            0 => {
                r.render(&scene(true), 160, 100).unwrap();
            }
            1 => {
                r.set_motion_enabled(false);
                r.render_at(&scene(true), 160, 100, 20).unwrap();
            }
            2 => {
                r.render_at(&scene(true), 180, 100, 20).unwrap();
            }
            3 => {
                r.render_at(&scene(true), 160, 100, 5).unwrap();
            }
            _ => {
                let view = View::column(
                    "root",
                    vec![card(true).class(if mode == 4 { "quiet" } else { "zero" })],
                );
                r.render_at(&view, 160, 100, 20).unwrap();
            }
        }
        assert!(!r.is_animating());
        let width = if mode == 2 { 180 } else { 160 };
        assert_eq!(r.pixels()[40 * width + 50], 0xffffffff);
    }
}
#[test]
fn unsupported_transitions_and_invalid_classes_are_rejected() {
    for value in [
        "opacity 10ms",
        "padding 10ms",
        "background",
        "radius -1ms",
        "radius 100ms typo",
    ] {
        assert!(Theme::parse(DARK, BASE, &format!("Text{{transition:{value};}}")).is_err());
    }
    let bad = scene(false).class("bad class");
    assert_eq!(bad.validate(), Err(UiError::InvalidClass));
    assert!(card(true).classes().iter().any(|s| s == "active"));
}
