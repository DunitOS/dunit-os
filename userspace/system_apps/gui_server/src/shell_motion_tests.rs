//! Host checks exercise the same native component tracks used by the scene.
extern crate alloc;
#[path = "animation.rs"]
mod animation;
#[path = "lua_policy.rs"]
mod lua_policy;
#[path = "shell_motion.rs"]
mod shell_motion;
use lua_policy::{Animation, Effect, Property};
use shell_motion::Tracks;
fn action(property: Property, from: i64, to: i64) -> Animation {
    let mut curve = [0; 33];
    for (i, v) in curve.iter_mut().enumerate() {
        *v = ((i * 65535 + 16) / 32) as i32;
    }
    Animation {
        window_id: 0,
        target: 1,
        effect: Effect::Fade,
        duration_ms: 200,
        property: Some(property),
        from,
        to,
        curve,
    }
}
#[test]
fn scene_geometry_and_opacity_reach_their_endpoints() {
    let mut tracks = Tracks::default();
    for a in [
        action(Property::TranslationX, 0, 30),
        action(Property::Width, 100, 60),
        action(Property::Scale, 65535, 131070),
        action(Property::Opacity, 65535, 0),
    ] {
        tracks.submit(&a, 1000).unwrap();
    }
    assert_eq!(
        tracks.sample(1, (10, 20, 100, 50), 1000).rect,
        (10, 20, 100, 50)
    );
    let end = tracks.sample(1, (10, 20, 100, 50), 1200);
    assert_eq!(end.rect, (40, 20, 120, 100));
    assert_eq!(end.opacity, 0);
    assert_eq!(tracks.sample(2, (0, 0, 30, 40), 1200).rect, (0, 0, 30, 40));
}
#[test]
fn dss_background_and_text_transitions_are_independent_and_interruptible() {
    use dunit_style::{cascade::NodeStyle, Cascade};
    let sheet=dunit_style::parse("Component {background:#0000;color:#ff0000;transition:background 200ms linear, color 200ms linear;} Component:hover {background:#00ff00ff;color:#0000ff;}").unwrap();
    let mut cascade = Cascade::new();
    cascade.push(sheet);
    let normal = cascade.resolve(&NodeStyle::element("Component"));
    let hover = cascade.resolve(&NodeStyle {
        element: "Component",
        id: None,
        classes: &[],
        states: &["hover"],
    });
    let mut tracks = Tracks::default();
    let rect = (0, 0, 50, 20);
    tracks.transition(
        1,
        rect,
        &normal,
        &hover,
        &hover.transitions(),
        1000,
        0xff000000,
    );
    let start = tracks.sample(1, rect, 1000);
    assert_eq!(start.background, Some(0));
    assert_eq!(start.color, Some(0xffff0000));
    let middle = tracks.sample(1, rect, 1100);
    assert!((120..=135).contains(&(middle.background.unwrap() >> 24)));
    tracks.transition(
        1,
        rect,
        &hover,
        &normal,
        &normal.transitions(),
        1100,
        0xff000000,
    );
    let interrupted = tracks.sample(1, rect, 1100);
    assert_eq!(interrupted.background, middle.background);
    assert_eq!(interrupted.color, middle.color);
    assert_eq!(tracks.sample(1, rect, 1300).background, Some(0));
}
#[test]
fn invalid_curve_does_not_replace_a_running_track() {
    let mut tracks = Tracks::default();
    let good = action(Property::Opacity, 65535, 0);
    tracks.submit(&good, 0).unwrap();
    let mut bad = action(Property::Opacity, 0, 65535);
    bad.curve[32] = 0;
    assert!(tracks.submit(&bad, 100).is_err());
    assert_eq!(tracks.sample(1, (0, 0, 1, 1), 200).opacity, 0);
}
