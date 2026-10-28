//! DSS motion tests.

use dunit_style::cascade::NodeStyle;
use dunit_style::motion::{interpolate, lerp_color, Easing};
use dunit_style::value::{Color, Value};
use dunit_style::{parse, Cascade};

#[test]
fn easing_endpoints_and_shape() {
    for e in [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
    ] {
        assert_eq!(e.apply(0.0), 0.0);
        assert_eq!(e.apply(1.0), 1.0);
    }
    // Clamped outside 0..1.
    assert_eq!(Easing::Linear.apply(-1.0), 0.0);
    assert_eq!(Easing::Linear.apply(2.0), 1.0);
    // Ease-in starts slow: at t=0.5 it is below the linear midpoint.
    assert!(Easing::EaseIn.apply(0.5) < 0.5);
    // Ease-out ends slow: at t=0.5 it is above the linear midpoint.
    assert!(Easing::EaseOut.apply(0.5) > 0.5);
}

#[test]
fn color_interpolation_midpoint() {
    let a = Color::rgb(0, 0, 0);
    let b = Color::rgb(255, 100, 50);
    let mid = lerp_color(a, b, 0.5);
    assert_eq!(mid, Color::rgb(128, 50, 25));
}

#[test]
fn interpolate_numbers_with_easing() {
    // Linear halfway.
    assert_eq!(
        interpolate(
            &Value::Number(0.0),
            &Value::Number(10.0),
            0.5,
            Easing::Linear
        ),
        Value::Number(5.0)
    );
    // Ease-in halfway (0.25 of the way).
    assert_eq!(
        interpolate(
            &Value::Number(0.0),
            &Value::Number(100.0),
            0.5,
            Easing::EaseIn
        ),
        Value::Number(25.0)
    );
}

#[test]
fn mismatched_values_snap_at_half() {
    let from = Value::Keyword("hidden".into());
    let to = Value::Keyword("visible".into());
    assert_eq!(interpolate(&from, &to, 0.4, Easing::Linear), from);
    assert_eq!(interpolate(&from, &to, 0.6, Easing::Linear), to);
}

#[test]
fn transitions_parsed_from_style() {
    let sheet =
        parse(r#"Button { transition: background 160ms ease-out, color 100ms linear; }"#).unwrap();
    let mut cas = Cascade::new();
    cas.push(sheet);
    let s = cas.resolve(&NodeStyle::element("Button"));
    let ts = s.transitions();
    assert_eq!(ts.len(), 2);
    assert_eq!(ts[0].property, "background");
    assert_eq!(ts[0].duration_ms, 160.0);
    assert_eq!(ts[0].easing, Easing::EaseOut);
    assert_eq!(ts[1].property, "color");
    assert_eq!(ts[1].duration_ms, 100.0);
    assert_eq!(ts[1].easing, Easing::Linear);
}

#[test]
fn strict_descriptors_require_valid_timing_and_unique_properties() {
    use dunit_style::motion::Transition;
    let value = |declaration: &str| {
        let sheet = parse(&format!("Text {{transition:{declaration};}}")).unwrap();
        let mut cascade = Cascade::new();
        cascade.push(sheet);
        cascade
            .resolve(&NodeStyle::element("Text"))
            .get("transition")
            .unwrap()
            .clone()
    };
    let parsed =
        Transition::parse_strict(&value("background 100ms ease-out, radius 0 linear")).unwrap();
    assert_eq!(parsed.len(), 2);
    assert!(Transition::parse_strict(&value("none")).unwrap().is_empty());
    for invalid in [
        "background",
        "background -1ms",
        "radius 60001ms",
        "color 100ms typo",
        "color 1ms color 2ms",
        "100ms color",
    ] {
        assert!(
            Transition::parse_strict(&value(invalid)).is_none(),
            "{invalid}"
        );
    }
    for (name, valid) in [
        ("active", true),
        ("_quiet-1", true),
        ("", false),
        ("1active", false),
        ("two words", false),
        ("x.y", false),
    ] {
        assert_eq!(dunit_style::parse::valid_identifier(name), valid);
    }
}
