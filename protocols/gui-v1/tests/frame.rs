use gui_protocol_v1::{
    frame::{Damage, Frame},
    Rect,
};
#[test]
fn strided_offset_images_copy_only_damage_and_reject_short_backings_atomically() {
    let frame = Frame {
        buffer: 2,
        commit: 5,
        width: 2,
        height: 2,
        stride: 12,
        offset: 4,
        format: 1,
        damage: Damage::from_validated(&[Rect {
            x: 1,
            y: 1,
            w: 1,
            h: 1,
        }]),
    };
    let mut source = vec![0; 24];
    for at in [4, 8, 16] {
        source[at..at + 4].copy_from_slice(&7u32.to_le_bytes());
    }
    source[20..24].copy_from_slice(&9u32.to_le_bytes());
    let mut target = vec![1, 2, 3, 4];
    assert_eq!(frame.copy_into(&source, &mut target, false), Some(1));
    assert_eq!(target, [1, 2, 3, 9]);
    let before = target.clone();
    assert_eq!(frame.copy_into(&source[..23], &mut target, true), None);
    assert_eq!(target, before);
    assert_eq!(frame.copy_into(&source, &mut target, true), Some(4));
    assert_eq!(target, [7, 7, 7, 9]);
    let bad = Frame {
        damage: Damage::from_validated(&[Rect {
            x: -1,
            y: 0,
            w: 1,
            h: 1,
        }]),
        ..frame
    };
    assert_eq!(bad.copy_into(&source, &mut target, false), None);
    assert_eq!(target, [7, 7, 7, 9]);
}
#[test]
fn empty_damage_requests_continuous_legacy_refresh() {
    assert!(Damage::from_validated(&[]).continuous);
    let mut explicit = Damage::from_validated(&[Rect {
        x: 0,
        y: 0,
        w: 2,
        h: 2,
    }]);
    assert!(!explicit.continuous);
    explicit.force_full();
    assert!(!explicit.continuous);
    assert!(explicit.rects().is_empty());
}
