use super::{S3BleLinkSequence, replace_s3_ble_aux_pointer, s3_ble_event_index};

#[test]
fn ble_event_index_comes_from_control_structure_not_scheduler_slot() {
    assert_eq!(s3_ble_event_index(0x0880), 0);
    assert_eq!(s3_ble_event_index(0x0881), 1);
    assert_eq!(s3_ble_event_index(0xffff), 31);
}

#[test]
fn extended_advertising_aux_pointer_uses_native_channel_offset_and_phy() {
    let mut frame = [0x27, 0x07, 0x06, 0x18, 0x07, 0x27, 0x20, 0x00, 0x00];
    assert!(replace_s3_ble_aux_pointer(&mut frame, 32, 0x2005));
    assert_eq!(frame, [0x27, 0x07, 0x06, 0x18, 0x07, 0x27, 32, 5, 32]);
}

#[test]
fn ble_phy_update_changes_both_directions_only_at_the_instant() {
    let mut sequence = S3BleLinkSequence::default();
    assert_eq!(sequence.begin_event().unwrap(), ("ble-1m", "ble-1m"));
    sequence
        .observe_received(&[3, 5, 0x18, 2, 2, 6, 0])
        .unwrap();
    for _ in 1..6 {
        assert_eq!(sequence.begin_event().unwrap(), ("ble-1m", "ble-1m"));
    }
    assert_eq!(sequence.begin_event().unwrap(), ("ble-2m", "ble-2m"));

    let mut illegal = S3BleLinkSequence::default();
    illegal.begin_event().unwrap();
    assert!(
        illegal
            .observe_received(&[3, 5, 0x18, 2, 2, 5, 0])
            .unwrap_err()
            .contains("is 5 events after")
    );

    let mut overlapping = S3BleLinkSequence::default();
    overlapping.begin_event().unwrap();
    overlapping
        .observe_received(&[3, 5, 0x18, 2, 2, 6, 0])
        .unwrap();
    assert_eq!(
        overlapping
            .observe_received(&[11, 5, 0x18, 2, 2, 7, 0])
            .unwrap_err(),
        "overlapping BLE PHY update procedures"
    );

    let mut invalid_phy = S3BleLinkSequence::default();
    invalid_phy.begin_event().unwrap();
    assert_eq!(
        invalid_phy
            .observe_received(&[3, 5, 0x18, 4, 2, 6, 0])
            .unwrap_err(),
        "invalid central TX PHY value 4"
    );

    let mut terminated = S3BleLinkSequence::default();
    terminated.begin_event().unwrap();
    assert!(terminated.observe_received(&[3, 2, 0x02, 0x13]).unwrap());
}

fn s3_sequence_at_start_encryption_response() -> S3BleLinkSequence {
    let mut sequence = S3BleLinkSequence::default();
    sequence
        .observe_received(&[
            3, 23, 3, 16, 17, 18, 19, 20, 21, 22, 23, 52, 18, 32, 33, 34, 35, 36, 37, 38, 39, 48,
            49, 50, 51,
        ])
        .unwrap();
    assert!(
        sequence
            .observe_security_ecb(
                [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                [
                    32, 33, 34, 35, 36, 37, 38, 39, 117, 204, 150, 89, 66, 217, 135, 254,
                ],
                [
                    184, 2, 158, 219, 232, 51, 140, 103, 146, 27, 73, 37, 174, 203, 206, 108,
                ],
            )
            .unwrap()
    );
    assert_eq!(
        sequence
            .prepare_transmitted(&[
                23, 13, 4, 117, 204, 150, 89, 66, 217, 135, 254, 147, 165, 63, 51,
            ])
            .unwrap(),
        [
            23, 13, 4, 117, 204, 150, 89, 66, 217, 135, 254, 147, 165, 63, 51
        ]
    );
    assert_eq!(
        sequence.prepare_transmitted(&[23, 1, 5]).unwrap(),
        [23, 1, 5]
    );
    let start_response = sequence
        .decode_received(&[15, 5, 184, 9, 240, 138, 197])
        .unwrap();
    assert_eq!(start_response, [15, 1, 6]);
    sequence.observe_received(&start_response).unwrap();
    sequence
}

#[test]
fn ble_encryption_tracks_firmware_ecb_ccm_counters_and_deferred_response() {
    let mut sequence = s3_sequence_at_start_encryption_response();
    assert_eq!(
        sequence.prepare_transmitted(&[13, 0]).unwrap(),
        [13, 4, 158, 245, 5, 206]
    );
    sequence.acknowledge_encrypted_transmission(false).unwrap();

    let empty = sequence.decode_received(&[5, 4, 51, 203, 243, 20]).unwrap();
    assert_eq!(empty, [5, 0]);
    sequence.observe_received(&empty).unwrap();
    let terminate = sequence
        .decode_received(&[11, 6, 229, 251, 202, 6, 119, 40])
        .unwrap();
    assert_eq!(terminate, [11, 2, 2, 19]);
    assert!(sequence.observe_received(&terminate).unwrap());
    assert_eq!(
        sequence
            .prepare_transmitted(&[19, 5, 6, 0, 0, 0, 0])
            .unwrap(),
        [19, 9, 64, 145, 48, 102, 250, 205, 206, 172, 115]
    );
}

#[test]
fn ble_encryption_rejects_bad_mic_and_unobserved_firmware_descriptor_state() {
    let bad_mic = s3_sequence_at_start_encryption_response();
    let mut corrupted = [5, 4, 51, 203, 243, 20];
    corrupted[5] ^= 1;
    assert!(
        bad_mic
            .decode_received(&corrupted)
            .unwrap_err()
            .contains("MIC verification failed")
    );

    let mut malformed = s3_sequence_at_start_encryption_response();
    assert!(
        malformed
            .prepare_transmitted(&[19, 5, 6, 0, 0, 0, 1])
            .unwrap_err()
            .contains("length 7 bytes")
    );
}
