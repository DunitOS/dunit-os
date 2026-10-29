use dunit_widgets::{ScrollAxis, ScrollCommand::*, Thumb};

fn close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= expected.abs().max(1.) * 0.00001,
        "actual={actual}, expected={expected}"
    );
}

#[test]
fn extents_resize_and_shrink_clamp_position() {
    let mut axis = ScrollAxis::new(100., 1000.);
    assert_eq!(axis.offset, 0.);
    assert_eq!(axis.max_offset(), 900.);
    assert!(axis.move_by(800.));
    assert!(axis.set_extents(300., 1000.));
    assert_eq!(axis.offset, 700.);
    assert!(axis.set_extents(300., 200.));
    assert_eq!(axis.offset, 0.);
    assert_eq!(axis.max_offset(), 0.);
    assert!(!axis.set_extents(300., 200.));
    assert!(!axis.move_by(100.));
    assert!(axis.set_extents(0., 200.));
    assert!(axis.move_by(200.));
    assert_eq!(axis.offset, 200.);
}

#[test]
fn commands_use_line_and_viewport_steps_and_clamp() {
    let mut axis = ScrollAxis::new(100., 350.);
    assert!(axis.apply(LineDown, 25.));
    assert_eq!(axis.offset, 25.);
    assert!(axis.apply(PageDown, f32::NAN));
    assert_eq!(axis.offset, 125.);
    assert!(axis.apply(PageDown, 0.));
    assert_eq!(axis.offset, 225.);
    assert!(axis.apply(PageDown, 0.));
    assert_eq!(axis.offset, 250.);
    assert!(!axis.apply(End, 0.));
    assert!(axis.apply(PageUp, 0.));
    assert_eq!(axis.offset, 150.);
    assert!(axis.apply(LineUp, 25.));
    assert_eq!(axis.offset, 125.);
    assert!(axis.apply(Home, f32::INFINITY));
    assert_eq!(axis.offset, 0.);
    assert!(!axis.apply(LineUp, 25.));
    assert!(axis.apply(End, 0.));
    assert_eq!(axis.offset, 250.);
    assert!(!axis.apply(LineDown, -1.));
    assert!(!axis.apply(LineDown, f32::INFINITY));
    assert!(!axis.apply(LineUp, f32::NAN));
}

#[test]
fn reveal_uses_minimum_movement_and_handles_oversized_ranges() {
    let mut axis = ScrollAxis::new(100., 1000.);
    assert!(!axis.reveal(20., 80.));
    assert!(axis.reveal(90., 120.));
    assert_eq!(axis.offset, 20.);
    assert!(axis.reveal(10., 30.));
    assert_eq!(axis.offset, 10.);
    assert!(axis.reveal(500., 800.));
    assert_eq!(axis.offset, 500.);
    assert!(!axis.reveal(400., 800.));
    assert!(axis.reveal(100., 300.));
    assert_eq!(axis.offset, 200.);
    assert!(axis.reveal(2000., 3000.));
    assert_eq!(axis.offset, 900.);
    assert!(axis.reveal(-100., -1.));
    assert_eq!(axis.offset, 0.);
    assert!(!axis.reveal(-100., 3000.));
    let old = axis;
    for (start, end) in [(5., 4.), (f32::NAN, 1.), (0., f32::INFINITY)] {
        assert!(!axis.reveal(start, end));
        assert_eq!(axis, old);
    }
    let mut zero = ScrollAxis::new(0., 100.);
    assert!(zero.reveal(50., 75.));
    assert_eq!(zero.offset, 50.);
    assert!(zero.reveal(100., 100.));
    assert_eq!(zero.offset, 100.);
}

#[test]
fn thumb_proportions_minimum_and_track_endpoints() {
    let mut axis = ScrollAxis::new(100., 1000.);
    assert_eq!(
        axis.thumb(10., 200., 5.),
        Some(Thumb {
            start: 10.,
            length: 20.
        })
    );
    axis.apply(End, 0.);
    assert_eq!(
        axis.thumb(10., 200., 5.),
        Some(Thumb {
            start: 190.,
            length: 20.
        })
    );
    axis.apply(Home, 0.);
    assert_eq!(
        axis.thumb(-50., 200., 40.),
        Some(Thumb {
            start: -50.,
            length: 40.
        })
    );
    axis.move_by(450.);
    assert_eq!(
        axis.thumb(-50., 200., 40.),
        Some(Thumb {
            start: 30.,
            length: 40.
        })
    );
    assert_eq!(
        axis.thumb(10., 3., 40.),
        Some(Thumb {
            start: 10.,
            length: 3.
        })
    );
    let old = axis;
    assert!(!axis.drag_thumb(20., 2., 10., 3., 40.));
    assert_eq!(axis, old);
    assert_eq!(ScrollAxis::new(100., 100.).thumb(0., 100., 10.), None);
    assert_eq!(ScrollAxis::new(200., 100.).thumb(0., 100., 10.), None);
}

#[test]
fn dragging_preserves_grab_anchor_and_clamps_both_ends() {
    let mut axis = ScrollAxis::new(100., 500.);
    axis.move_by(200.);
    assert_eq!(
        axis.thumb(10., 100., 0.),
        Some(Thumb {
            start: 50.,
            length: 20.
        })
    );
    assert!(!axis.drag_thumb(65., 15., 10., 100., 0.));
    assert!(axis.drag_thumb(85., 15., 10., 100., 0.));
    assert_eq!(axis.offset, 300.);
    assert!(axis.drag_thumb(-100., 15., 10., 100., 0.));
    assert_eq!(axis.offset, 0.);
    assert!(axis.drag_thumb(1000., 15., 10., 100., 0.));
    assert_eq!(axis.offset, 400.);
    // Minimum-sized thumbs use their actual travel, not proportional size.
    assert!(axis.drag_thumb(55., 15., 10., 100., 40.));
    assert_eq!(axis.offset, 200.);
    // A stale grab beyond the resized thumb is clamped into that thumb.
    assert!(axis.drag_thumb(30., 100., 10., 100., 0.));
    assert_eq!(axis.offset, 0.);
    assert!(axis.drag_thumb(50., -10., 10., 100., 0.));
    assert_eq!(axis.offset, 200.);
}

#[test]
fn invalid_geometry_and_deltas_never_poison_a_valid_axis() {
    let mut axis = ScrollAxis::new(10., 100.);
    axis.move_by(20.);
    let old = axis;
    for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(!axis.move_by(invalid));
        assert!(!axis.drag_thumb(invalid, 0., 0., 100., 10.));
        assert!(!axis.drag_thumb(0., invalid, 0., 100., 10.));
        assert_eq!(axis.thumb(invalid, 100., 10.), None);
        assert_eq!(axis.thumb(0., invalid, 10.), None);
        assert_eq!(axis.thumb(0., 100., invalid), None);
        assert_eq!(axis, old);
        assert_eq!(ScrollAxis::new(invalid, invalid), ScrollAxis::default());
    }
    assert_eq!(axis.thumb(0., 0., 10.), None);
    assert_eq!(axis.thumb(0., -1., 10.), None);
    assert_eq!(ScrollAxis::new(-10., -50.), ScrollAxis::default());
    // Public fields are normalized by a subsequent valid mutation.
    let mut supplied = ScrollAxis {
        viewport: f32::NAN,
        content: 100.,
        offset: f32::INFINITY,
    };
    assert!(supplied.move_by(25.));
    assert_eq!(
        supplied,
        ScrollAxis {
            viewport: 0.,
            content: 100.,
            offset: 25.
        }
    );
}

#[test]
fn extreme_finite_values_and_tiny_tracks_remain_bounded() {
    let mut axis = ScrollAxis::new(1., f32::MAX);
    assert!(axis.move_by(f32::MAX));
    assert!(axis.offset.is_finite());
    assert_eq!(axis.offset, axis.max_offset());
    assert!(!axis.move_by(f32::MAX));
    assert!(axis.move_by(-f32::MAX));
    assert_eq!(axis.offset, 0.);
    let thumb = axis.thumb(f32::MAX, f32::MAX, 0.).unwrap();
    assert!(thumb.start.is_finite() && thumb.length.is_finite());
    axis.apply(End, 0.);
    let thumb = axis.thumb(f32::MAX, f32::MAX, 0.).unwrap();
    assert!(thumb.start.is_finite() && thumb.length.is_finite());
    assert!(axis.drag_thumb(-f32::MAX, f32::MAX, -f32::MAX, f32::MAX, 0.));
    assert!(axis.offset.is_finite() && axis.offset >= 0. && axis.offset <= axis.max_offset());
    let mut tiny = ScrollAxis::new(1., 2.);
    let thumb = tiny.thumb(0., f32::MIN_POSITIVE, 0.).unwrap();
    close(thumb.length, f32::MIN_POSITIVE / 2.);
    assert!(thumb.length > 0.);
    assert!(tiny.drag_thumb(f32::MIN_POSITIVE, 0., 0., f32::MIN_POSITIVE, 0.));
    assert_eq!(tiny.offset, 1.);
    let empty_view = ScrollAxis::new(0., 100.);
    assert_eq!(
        empty_view.thumb(0., 100., 10.),
        Some(Thumb {
            start: 0.,
            length: 10.
        })
    );
}
