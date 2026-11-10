#![cfg(feature = "gui")]
#[path = "../../../userspace/system_apps/gui_server/src/pointer_mode.rs"]
mod policy;
use gui_protocol_v1::control::PointerMode;
#[test]
fn focus_owner_and_relative_source_are_enforced() {
    let mut s = policy::State::default();
    assert_eq!(
        s.request(1, Some(1), PointerMode::Locked),
        Err(libdunit::EOPNOTSUPP)
    );
    s.sample((10, 10), (10, 10), true, (0, 0, 100, 100));
    assert_eq!(
        s.request(1, Some(2), PointerMode::Locked),
        Err(libdunit::EACCES)
    );
    s.request(1, Some(1), PointerMode::Locked).unwrap();
    assert_eq!(
        s.request(2, Some(2), PointerMode::Locked),
        Err(libdunit::EBUSY)
    );
    assert_eq!(
        s.request(2, Some(2), PointerMode::Normal),
        Err(libdunit::EACCES)
    );
    assert_eq!(s.mode, PointerMode::Locked);
    assert_eq!(s.reconcile(Some(2)), Some(1));
    assert_eq!(s.reconcile(Some(2)), None);
    assert_eq!(s.mode, PointerMode::Normal);
    assert_eq!(s.owner, None);
    assert_eq!(s.reconcile(Some(1)), None);
}
#[test]
fn lock_freezes_cursor_and_confinement_uses_unclamped_deltas() {
    let mut s = policy::State::default();
    s.sample((50, 50), (50, 50), true, (0, 0, 100, 100));
    s.request(1, Some(1), PointerMode::Locked).unwrap();
    assert_eq!(
        s.sample((99, 99), (5000, 5000), true, (20, 20, 60, 60)),
        (50, 50)
    );
    s.request(1, Some(1), PointerMode::Confined).unwrap();
    assert_eq!(
        s.sample((99, 99), (5000, 5000), true, (20, 20, 60, 60)),
        (79, 79)
    );
    // Device's absolute projection is still at the edge; reverse motion must
    // immediately move the virtual confined pointer back into the client.
    assert_eq!(
        s.sample((99, 99), (-10, -10), true, (20, 20, 60, 60)),
        (69, 69)
    );
    assert_eq!(
        s.sample((0, 0), (-5000, -5000), true, (20, 20, 60, 60)),
        (20, 20)
    );
    s.bound((30, 30, 10, 10));
    assert_eq!(s.position, (30, 30));
    s.release();
    assert_eq!(s.sample((0, 0), (1, 2), true, (0, 0, 100, 100)), (31, 32));
    assert_eq!(
        s.sample((90, 80), (0, 0), false, (0, 0, 100, 100)),
        (90, 80)
    );
}
