//! Pixel evidence for the resolved SDK appearance, labels and clipping.
use dunit_render::{paint, Surface};
use dunit_style::{cascade::NodeStyle, parse, Cascade};
use dunit_text::Font;
use dunit_ui::{layout, parse as dui};

fn font() -> Font {
    Font::parse(
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/DejaVuSans.ttf"
        ))
        .unwrap(),
    )
    .unwrap()
}
fn render(
    tag: &str,
    checked: bool,
    width: usize,
    height: usize,
    sheet: &str,
    label: &str,
) -> Vec<u32> {
    let tree = dui(&format!(
        "Column {{ {tag} width={width} height={height} \"{label}\" }}"
    ))
    .unwrap();
    let layout = layout(&tree, 100., 60.);
    let mut cascade = Cascade::new();
    cascade.push(parse(sheet).unwrap());
    let states: &[&str] = if checked { &["checked"] } else { &[] };
    let style = |id| {
        cascade.resolve(&NodeStyle {
            element: tree.node(id).kind.tag(),
            id: None,
            classes: &[],
            states,
        })
    };
    let mut pixels = vec![0; 100 * 60];
    paint(
        &tree,
        &layout,
        &style,
        &font(),
        &mut Surface::new(&mut pixels, 100, 60),
    )
    .unwrap();
    pixels
}
fn sdk() -> String {
    format!(
        "{}\n{}",
        include_str!("../../../assets/dwm/sdk/dark.dss"),
        include_str!("../../../assets/dwm/sdk/base.dss")
    )
}
#[test]
fn sdk_checked_marks_and_thumb_position_change_pixels_without_changing_label() {
    for tag in ["Checkbox", "Radio", "Switch"] {
        let off = render(tag, false, 90, 28, &sdk(), "OK");
        let on = render(tag, true, 90, 28, &sdk(), "OK");
        assert_ne!(off, on, "{tag} checked appearance");
        // Label area remains identical and contains ink; indicator is separated.
        let label_x = if tag == "Switch" { 46 } else { 30 };
        let mut label_ink = false;
        for y in 0..28 {
            for x in label_x..90 {
                assert_eq!(off[y * 100 + x], on[y * 100 + x]);
                label_ink |= on[y * 100 + x] == 0xffcdd6f4;
            }
        }
        assert!(label_ink, "{tag} label painted");
        let mark_color = if tag == "Switch" {
            0xffcdd6f4
        } else {
            0xff62b5e5
        };
        assert!(
            on.iter().any(|&p| p == mark_color),
            "{tag} has visible mark"
        );
        if tag == "Switch" {
            assert_eq!(off[14 * 100 + 10], mark_color);
            assert_eq!(on[14 * 100 + 30], mark_color);
            assert_ne!(off[14 * 100 + 30], mark_color);
        }
    }
}
#[test]
fn oversized_indicators_and_labels_are_contained_even_in_raw_paint() {
    let sheet = "Checkbox, Radio, Switch { font-size: 24; color: #ffffff; indicator-width: 70; indicator-height: 50; indicator-mark-size: 35; indicator-background: #334455; indicator-mark-color: #ffffff; indicator-radius: 25; }";
    for tag in ["Checkbox", "Radio", "Switch"] {
        let pixels = render(tag, true, 12, 10, sheet, "Overflow");
        assert!(pixels.iter().any(|&p| p != 0));
        for y in 0..60 {
            for x in 0..100 {
                if x >= 12 || y >= 10 {
                    assert_eq!(pixels[y * 100 + x], 0, "{tag}: {x},{y}");
                }
            }
        }
    }
}
#[test]
fn indicator_dimensions_are_driven_by_dss() {
    let small = "Radio { indicator-width: 10; indicator-height: 10; indicator-radius: 5; indicator-background: #ffffff; }";
    let large = "Radio { indicator-width: 20; indicator-height: 20; indicator-radius: 10; indicator-background: #ffffff; }";
    let a = render("Radio", false, 60, 30, small, "");
    let b = render("Radio", false, 60, 30, large, "");
    assert!(b.iter().filter(|&&p| p != 0).count() > 3 * a.iter().filter(|&&p| p != 0).count());
}

#[test]
fn toggle_box_and_both_profiles_resolve_checked_appearance() {
    for profile in [
        include_str!("../../../assets/dwm/sdk/dark.dss"),
        include_str!("../../../assets/dwm/sdk/light.dss"),
    ] {
        let sheet = format!(
            "{profile}\n{}",
            include_str!("../../../assets/dwm/sdk/base.dss")
        );
        for tag in ["ToggleButton", "Checkbox", "Radio", "Switch"] {
            assert_ne!(
                render(tag, false, 90, 28, &sheet, "OK"),
                render(tag, true, 90, 28, &sheet, "OK")
            );
        }
    }
}

#[test]
fn radio_dot_is_round_and_checkbox_glyph_is_distinct() {
    let sheet = "Checkbox, Radio { indicator-width: 24; indicator-height: 24; indicator-mark-size: 18; indicator-mark-color: #ffffff; }";
    let radio = render("Radio", true, 24, 24, sheet, "");
    let checkbox = render("Checkbox", true, 24, 24, sheet, "");
    assert_eq!(radio[12 * 100 + 12], 0xffffffff);
    assert_eq!(radio[3 * 100 + 3], 0);
    assert_eq!(radio[12 * 100 + 3], 0xffffffff);
    assert_ne!(checkbox, radio);
    assert!(checkbox.iter().any(|&p| p == 0xffffffff));
}

#[test]
fn active_indicators_are_visible_and_disabled_controls_suppress_press() {
    let mut cascade = Cascade::new();
    cascade.push(parse(&sdk()).unwrap());
    for tag in ["Checkbox", "Radio", "Switch"] {
        for checked in [false, true] {
            let idle: &[&str] = if checked { &["checked"] } else { &[] };
            let active: &[&str] = if checked {
                &["checked", "active"]
            } else {
                &["active"]
            };
            let disabled: &[&str] = if checked {
                &["checked", "disabled"]
            } else {
                &["disabled"]
            };
            let disabled_active: &[&str] = if checked {
                &["checked", "disabled", "active"]
            } else {
                &["disabled", "active"]
            };
            let style = |states| {
                cascade.resolve(&NodeStyle {
                    element: tag,
                    id: None,
                    classes: &[],
                    states,
                })
            };
            assert_ne!(
                style(idle).color("indicator-background"),
                style(active).color("indicator-background")
            );
            assert_eq!(
                style(disabled).color("indicator-background"),
                style(disabled_active).color("indicator-background")
            );
            assert_eq!(
                style(disabled).color("indicator-border-color"),
                style(disabled_active).color("indicator-border-color")
            );
        }
    }
}

#[test]
fn taller_controls_translate_labels_and_indicators_together() {
    for profile in [
        include_str!("../../../assets/dwm/sdk/dark.dss"),
        include_str!("../../../assets/dwm/sdk/light.dss"),
    ] {
        let sheet = format!(
            "{profile}\n{}",
            include_str!("../../../assets/dwm/sdk/base.dss")
        );
        let mut cascade = Cascade::new();
        cascade.push(parse(&sheet).unwrap());
        for tag in ["Checkbox", "Radio", "Switch", "Button", "ToggleButton"] {
            let ink = dunit_render::pack_argb(
                cascade
                    .resolve(&NodeStyle::element(tag))
                    .color("color")
                    .unwrap(),
            );
            let short = render(tag, false, 90, 28, &sheet, "OK");
            let tall = render(tag, false, 90, 44, &sheet, "OK");
            let label_x = match tag {
                "Switch" => 47,
                "Checkbox" | "Radio" => 31,
                _ => 4,
            };
            let mut count = 0;
            for y in 0..28 {
                for x in label_x..80 {
                    if short[y * 100 + x] == ink {
                        assert_eq!(tall[(y + 8) * 100 + x], ink, "{tag} label y={y}");
                        count += 1;
                    }
                }
            }
            assert!(count > 0, "{tag} label has ink");
            if matches!(tag, "Checkbox" | "Radio" | "Switch") {
                for y in 5..23 {
                    for x in 5..(label_x - 8) {
                        assert_eq!(
                            short[y * 100 + x],
                            tall[(y + 8) * 100 + x],
                            "{tag} indicator {x},{y}"
                        );
                    }
                }
            }
        }
        // Text is intentionally top aligned even inside a taller explicit box.
        let short = render("Text", false, 90, 28, &sheet, "OK");
        let tall = render("Text", false, 90, 44, &sheet, "OK");
        assert_eq!(short, tall);
    }
}

#[test]
fn checked_indicator_focus_ring_is_separate_from_track() {
    let mut cascade = Cascade::new();
    cascade.push(parse(&sdk()).unwrap());
    for tag in ["Checkbox", "Radio", "Switch"] {
        let resolve = |states| {
            cascade.resolve(&NodeStyle {
                element: tag,
                id: None,
                classes: &[],
                states,
            })
        };
        let checked = resolve(&["checked"]);
        let focused = resolve(&["checked", "focus"]);
        assert_eq!(
            checked.number("border-width"),
            focused.number("border-width")
        );
        assert_ne!(checked.color("border-color"), focused.color("border-color"));
        let tree = dui(&format!("Column {{ {tag} width=90 height=44 \"OK\" }}")).unwrap();
        let layout = layout(&tree, 100., 60.);
        let mut images = Vec::new();
        for states in [&["checked"][..], &["checked", "focus"][..]] {
            let mut pixels = vec![0; 100 * 60];
            let style = |id| {
                cascade.resolve(&NodeStyle {
                    element: tree.node(id).kind.tag(),
                    id: None,
                    classes: &[],
                    states,
                })
            };
            paint(
                &tree,
                &layout,
                &style,
                &font(),
                &mut Surface::new(&mut pixels, 100, 60),
            )
            .unwrap();
            images.push(pixels);
        }
        assert_ne!(images[0], images[1], "{tag} focused checked pixels");
        // The outer ring lies above the vertically centered indicator.
        assert_ne!(
            images[0][40], images[1][40],
            "{tag} ring is outside indicator"
        );
    }
}
