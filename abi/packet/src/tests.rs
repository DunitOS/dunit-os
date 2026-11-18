use super::*;

#[test]
fn records_round_trip_max_batch_and_keep_trailing_output() {
    let frame = [0x4d; MAX_FRAME_LEN];
    let len = record_len(frame.len()).unwrap();
    assert_eq!(len, 1528);
    let mut batch = [0x92; MAX_BATCH_LEN];
    let used = len * MAX_RECORDS;
    for destination in batch[..used].chunks_exact_mut(len) {
        assert_eq!(encode_record(&frame, destination), Ok(len));
        assert_eq!(&destination[..8], &[234, 5, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&destination[len - 2..], &[0, 0]);
    }
    let mut records = Records::new(&batch[..used]).unwrap();
    assert_eq!(records.len(), MAX_RECORDS);
    for remaining in (0..MAX_RECORDS).rev() {
        assert_eq!(records.next(), Some(frame.as_slice()));
        assert_eq!(records.len(), remaining);
    }
    assert_eq!(records.next(), None);
    assert!(batch[used..].iter().all(|&byte| byte == 0x92));
}

#[test]
fn validate_all_records_before_yielding_any() {
    let frame = [0x31; 61];
    let len = record_len(frame.len()).unwrap();
    assert_eq!(len, 72);
    let mut batch = [0; 144];
    encode_record(&frame, &mut batch).unwrap();
    encode_record(&frame, &mut batch[len..]).unwrap();
    assert_eq!(Records::new(&batch).unwrap().count(), 2);
    for index in [len, len + 2, len + 4, batch.len() - 1] {
        let mut bad = batch;
        bad[index] = 1;
        assert_eq!(Records::new(&bad).unwrap_err(), WireError::Invalid);
    }
    assert_eq!(
        Records::new(&batch[..]).unwrap().nth(1),
        Some(frame.as_slice())
    );
    assert_eq!(
        Records::new(&batch[..len + 1]).unwrap_err(),
        WireError::Invalid
    );
    assert_eq!(
        Records::new(&batch[..batch.len() - 1]).unwrap_err(),
        WireError::Invalid
    );
}

#[test]
fn enforce_frame_count_and_byte_bounds_independently() {
    let frame = [0x14; MIN_FRAME_LEN];
    let len = record_len(frame.len()).unwrap();
    let mut batch = [0; 72 * 33];
    for destination in batch.chunks_exact_mut(len) {
        encode_record(&frame, destination).unwrap();
    }
    assert_eq!(Records::new(&batch[..len * 32]).unwrap().count(), 32);
    assert_eq!(Records::new(&batch).unwrap_err(), WireError::Invalid);
    assert_eq!(Records::new(&[]).unwrap_err(), WireError::Invalid);
    assert_eq!(
        Records::new(&[0; MAX_BATCH_LEN + 1]).unwrap_err(),
        WireError::Invalid
    );
}

#[test]
fn invalid_frames_or_short_outputs_do_not_mutate() {
    let mut output = [0xa5; 71];
    assert_eq!(
        encode_record(&[0; MIN_FRAME_LEN], &mut output),
        Err(WireError::BufferTooSmall)
    );
    assert_eq!(
        encode_record(&[0; MIN_FRAME_LEN - 1], &mut output),
        Err(WireError::Invalid)
    );
    assert_eq!(output, [0xa5; 71]);
    assert_eq!(record_len(usize::MAX), Err(WireError::Invalid));
}

#[test]
fn versioned_options_and_control_require_exact_zero_reserved_input() {
    assert_eq!(
        QueueOptions::decode(&QueueOptions.encode()),
        Ok(QueueOptions)
    );
    let mut options = QueueOptions.encode();
    assert_eq!(&options, &[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        QueueOptions::decode(&options[..15]),
        Err(WireError::Invalid)
    );
    options[2] = 1;
    assert_eq!(QueueOptions::decode(&options), Err(WireError::Invalid));
    options[0] = 2;
    options[2] = 0;
    assert_eq!(QueueOptions::decode(&options), Err(WireError::Invalid));

    let mut control = DeviceControl::Reset.encode();
    assert_eq!(DeviceControl::decode(&control), Ok(DeviceControl::Reset));
    control[2] = 2;
    assert_eq!(DeviceControl::decode(&control), Err(WireError::Unsupported));
    control[15] = 1;
    assert_eq!(DeviceControl::decode(&control), Err(WireError::Invalid));
}

#[test]
fn device_info_preserves_exact_offsets_and_counter_widths() {
    let info = DeviceInfo {
        link_up: true,
        mtu: 1500,
        next_rx_record_len: 1528,
        mac: [0x52, 0x54, 0, 0x12, 0x34, 0x56],
        rx_frames: 0x8877_6655_4433_2211,
        rx_bytes: 2,
        rx_dropped: 3,
        tx_frames: 4,
        tx_bytes: 5,
        tx_errors: u64::MAX,
    };
    let mut bytes = info.encode();
    assert_eq!(&bytes[..8], &[1, 0, 1, 0, 0xdc, 5, 0xf8, 5]);
    assert_eq!(&bytes[8..16], &[0x52, 0x54, 0, 0x12, 0x34, 0x56, 0, 0]);
    assert_eq!(
        &bytes[16..24],
        &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]
    );
    assert_eq!(DeviceInfo::decode(&bytes), Ok(info));
    bytes[2] |= 0x80;
    assert_eq!(DeviceInfo::decode(&bytes), Err(WireError::Invalid));
    bytes[2] &= !0x80;
    bytes[14] = 1;
    assert_eq!(DeviceInfo::decode(&bytes), Err(WireError::Invalid));
    assert_eq!(DeviceInfo::decode(&bytes[..63]), Err(WireError::Invalid));
}
