use dunit_wait_abi::*;
#[test]
fn records_match_the_public_little_endian_contract() {
    assert_eq!(core::mem::size_of::<Interest>(), 32);
    assert_eq!(core::mem::offset_of!(Interest, token), 16);
    assert_eq!(core::mem::offset_of!(Interest, deadline_ns), 24);
    let mut record = [0; 32];
    record[..4].copy_from_slice(&TIMER.to_le_bytes());
    record[4..8].copy_from_slice(&ELAPSED.to_le_bytes());
    record[16..24].copy_from_slice(&0x1234_5678_9abc_def0u64.to_le_bytes());
    record[24..].copy_from_slice(&991u64.to_le_bytes());
    assert_eq!(
        Interest::decode(&record),
        Interest::timer(991, 0x1234_5678_9abc_def0)
    );
    let bytes = Event {
        token: 9,
        events: EXITED,
        reserved: 0,
        data: u64::MAX,
    }
    .encode();
    assert_eq!(&bytes[..8], &9u64.to_le_bytes());
    assert_eq!(&bytes[8..16], &[32, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&bytes[16..], &[255; 8]);
}
#[test]
fn classes_reject_unknown_flags_and_inapplicable_fields() {
    for kind in 0..=255 {
        let mut entry = Interest::ipc(0);
        entry.kind = kind;
        assert_eq!(entry.valid(), kind == IPC || kind == DESCRIPTOR);
    }
    for mut entry in [
        Interest::ipc(1),
        Interest::timer(7, 2),
        Interest::process(9, 3),
        Interest::descriptor(7, READ | WRITE, 4),
    ] {
        assert!(entry.valid());
        entry.interest |= 1 << 31;
        assert!(!entry.valid());
    }
    assert!(!Interest::process(0, 0).valid());
    let mut entry = Interest::ipc(0);
    entry.object = 1;
    assert!(!entry.valid());
    entry = Interest::descriptor(0, 0, 0);
    entry.deadline_ns = 1;
    assert!(!entry.valid());
}
