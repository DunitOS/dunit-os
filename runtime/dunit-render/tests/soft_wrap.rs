use dunit_render::{CaretAffinity as A, TextInput, TextPosition as P, VisualLines};
use dunit_style::{Style, Value};
use dunit_text::Font;
use dunit_ui::Rect;
use dunit_widgets::text_input::EditCommand as E;
fn font() -> Font {
    Font::parse(include_bytes!("../../../assets/fonts/DejaVuSans.ttf").to_vec()).unwrap()
}
fn width(f: &Font, s: &str) -> f32 {
    s.chars()
        .map(|ch| f.glyph_metrics(f.glyph_index(ch)).advance as f32 * f.scale_for_px(16.))
        .sum()
}
fn map(f: &Font, s: &str, w: f32) -> VisualLines {
    VisualLines::new(s, f, 16., w, true, true, false)
}
fn style() -> Style {
    let mut s = Style::default();
    s.set("font-size", Value::Number(16.));
    s.set("caret-width", Value::Number(2.));
    s
}
fn input(value: &str) -> TextInput {
    TextInput {
        spinbox: None,
        value: value.into(),
        placeholder: String::new(),
        password: false,
        multiline: true,
        soft_wrap: true,
        affinity: A::Downstream,
        cursor: 0,
        anchor: 0,
        focused: true,
        caret_visible: true,
        scroll_x: 0.,
        scroll_y: 0.,
    }
}
#[test]
fn words_and_hard_lines_keep_all_original_bytes_and_trailing_blank_lines() {
    let f = font();
    let text = "alpha beta\n\nЖЖ\n";
    let m = map(&f, text, width(&f, "alpha ") + 0.1);
    assert_eq!(
        m.lines
            .iter()
            .map(|l| &text[l.start..l.end])
            .collect::<Vec<_>>(),
        ["alpha ", "beta", "", "ЖЖ", ""]
    );
    assert!(m.lines[0].soft_break);
    assert!(!m.lines[1].soft_break);
    assert_eq!(m.lines.last().unwrap().start, text.len());
}
#[test]
fn oversized_clusters_and_zero_width_always_progress_without_splitting_graphemes() {
    let f = font();
    let text = "a\u{301}👩\u{200d}💻🇷🇺z";
    for w in [0., 1., f32::NAN] {
        let m = map(&f, text, w);
        let ranges: Vec<_> = m.lines.iter().map(|l| &text[l.start..l.end]).collect();
        assert_eq!(ranges, ["a\u{301}", "👩\u{200d}💻", "🇷🇺", "z"]);
        assert_eq!(ranges.concat(), text);
    }
}
#[test]
fn wrap_boundary_has_two_caret_positions_and_hits_preserve_the_chosen_row() {
    let f = font();
    let m = map(&f, "abcdef", width(&f, "abc") + 0.1);
    assert_eq!(m.lines.len(), 2);
    assert_eq!(
        m.position(P {
            index: 3,
            affinity: A::Upstream
        }),
        (width(&f, "abc"), 0)
    );
    assert_eq!(
        m.position(P {
            index: 3,
            affinity: A::Downstream
        }),
        (0., 1)
    );
    assert_eq!(
        m.hit(0, 1000.),
        P {
            index: 3,
            affinity: A::Upstream
        }
    );
    assert_eq!(
        m.hit(1, -1000.),
        P {
            index: 3,
            affinity: A::Downstream
        }
    );
    assert_eq!(m.hit(usize::MAX, 1000.).index, 6);
}
#[test]
fn repeated_vertical_navigation_retains_pixel_goal_across_short_rows() {
    let f = font();
    let s = style();
    let rect = Rect {
        x: 0.,
        y: 0.,
        w: 300.,
        h: 100.,
    };
    let mut i = input("WWWW\ni\nWWWW");
    i.cursor = 3;
    let (p, goal) = i
        .visual_navigation(&f, &s, rect, E::Down { extend: false }, None)
        .unwrap();
    assert_eq!(p.index, 6);
    i.cursor = p.index;
    i.affinity = p.affinity;
    let (p, goal2) = i
        .visual_navigation(&f, &s, rect, E::Down { extend: true }, goal)
        .unwrap();
    assert_eq!(p.index, 10);
    assert_eq!(goal, goal2);
    i.cursor = p.index;
    i.affinity = p.affinity;
    let (p, _) = i
        .visual_navigation(&f, &s, rect, E::Up { extend: false }, goal2)
        .unwrap();
    assert_eq!(p.index, 6);
}
#[test]
fn visual_home_end_use_affinity_and_document_commands_remain_editor_commands() {
    let f = font();
    let s = style();
    let rect = Rect {
        x: 0.,
        y: 0.,
        w: width(&f, "abc") + 2.1,
        h: 100.,
    };
    let mut i = input("abcdef");
    let (end, _) = i
        .visual_navigation(&f, &s, rect, E::End { extend: true }, None)
        .unwrap();
    assert_eq!(
        end,
        P {
            index: 3,
            affinity: A::Upstream
        }
    );
    i.cursor = end.index;
    i.affinity = end.affinity;
    let (home, _) = i
        .visual_navigation(&f, &s, rect, E::Home { extend: false }, None)
        .unwrap();
    assert_eq!(home.index, 0);
    i.affinity = A::Downstream;
    let (home, _) = i
        .visual_navigation(&f, &s, rect, E::Home { extend: false }, None)
        .unwrap();
    assert_eq!(home.index, 3);
    assert!(i
        .visual_navigation(&f, &s, rect, E::DocumentEnd { extend: false }, None)
        .is_none());
}
#[test]
fn wrapping_recomputes_extents_hit_and_caret_reveal_after_resize() {
    let f = font();
    let s = style();
    let mut i = input("abcdefghijklmno");
    i.cursor = i.value.len();
    i.anchor = i.cursor;
    let narrow = Rect {
        x: 0.,
        y: 0.,
        w: width(&f, "abc") + 2.1,
        h: 20.,
    };
    let rows = i.visual_lines(&f, &s, narrow).lines.len();
    assert!(rows > 2);
    i.fit_scroll(&f, &s, narrow);
    assert!(i.scroll_y > 0.);
    assert_eq!(i.scroll_x, 0.);
    let (x, y) = i.scroll_axes(&f, &s, narrow);
    assert_eq!(x.max_offset(), 0.);
    assert!(y.max_offset() > 0.);
    let wide = Rect {
        w: 500.,
        h: 100.,
        ..narrow
    };
    i.fit_scroll(&f, &s, wide);
    assert_eq!((i.scroll_x, i.scroll_y), (0., 0.));
    assert_eq!(i.visual_lines(&f, &s, wide).lines.len(), 1);
    assert_eq!(i.hit_position_at(&f, &s, wide, 0., 0.).index, 0);
    assert_eq!(i.value, "abcdefghijklmno");
}
