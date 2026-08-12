#[test]
fn esp32c6_illegal_native_wifi_dma_is_a_hard_machine_error() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 9) | (1 << 10) | (1 << 17) | (1 << 18),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            0x600a_4d6c,
            AccessWidth::Word,
            (3_u64 << 30) | 2,
            SimTime::ZERO,
        )
        .unwrap();

    let error = machine.service_radio().unwrap_err();
    let MachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::DmaAddress);
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Wifi);
    assert!(error.to_string().contains("0x40800002"));
}

#[test]
fn esp32c6_illegal_native_wifi_ba_state_is_a_hard_machine_error() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 9) | (1 << 10),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            0x600a_4290,
            AccessWidth::Word,
            (1_u64 << 31) | 5,
            SimTime::ZERO,
        )
        .unwrap();
    let mut rts = vec![0xb4, 0, 0, 0];
    rts.extend_from_slice(&[0x02, 6, 7, 8, 9, 10]);
    rts.extend_from_slice(&[0x02, 1, 2, 3, 4, 5]);
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            rts.clone(),
            -35,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(rts.len() as u64 * 32);

    let error = machine.service_radio().unwrap_err();
    let MachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::SchedulerState);
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Wifi);
    assert!(error.detail.contains("impossible active control"));
}

#[test]
fn esp32c6_native_wifi_tx_excludes_hardware_fcs_from_the_rf_frame() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let descriptor = 0x4082_1000_u32;
    let buffer = 0x4082_1100_u32;
    let frame: Vec<u8> = (0_u8..30).collect();
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    let mut buffer_bytes = Vec::new();
    buffer_bytes.extend_from_slice(&34_u32.to_le_bytes());
    buffer_bytes.extend_from_slice(&0_u32.to_le_bytes());
    buffer_bytes.extend_from_slice(&frame);
    buffer_bytes.extend_from_slice(&[0xa5; 4]);
    machine
        .debug_write_memory(u64::from(buffer), &buffer_bytes)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_4d6c,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();

    assert_eq!(machine.service_radio().unwrap(), 1);
    assert!(machine
        .radio_replay_artifact()
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(
            event,
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.protocol == remu_radio::RadioProtocol::Wifi
                    && request.frame.bytes == frame
        )));
}

#[test]
fn esp32c6_native_wifi_tx_applies_the_firmware_selected_ccmp_key() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let descriptor = 0x4082_1000_u32;
    let buffer = 0x4082_1100_u32;
    let local = [0x52, 0x45, 0x02, 0x00, 0x00, 0x01];
    let peer = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0x01];
    let key = [
        0xb8, 0x53, 0xfa, 0xfe, 0x53, 0x02, 0x44, 0xed, 0xbd, 0xd3, 0xa9, 0x86, 0x48, 0x0b,
        0xed, 0xcf,
    ];
    machine
        .bus
        .write(0x600a_4064, AccessWidth::Word, 0x0002_4552, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600a_4068, AccessWidth::Word, 0x0001_0100, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600a_5bc0, AccessWidth::Word, 0xccbb_aa02, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600a_5bc4, AccessWidth::Word, 0xc16c_01dd, SimTime::ZERO)
        .unwrap();
    for (index, word) in key.chunks_exact(4).enumerate() {
        machine
            .bus
            .write(
                0x600a_5bc8 + index as u64 * 4,
                AccessWidth::Word,
                u64::from(u32::from_le_bytes(word.try_into().unwrap())),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x600a_4814, AccessWidth::Word, 1 << 24, SimTime::ZERO)
        .unwrap();

    let mut frame = vec![0xd0, 0x40, 0, 0];
    frame.extend_from_slice(&peer);
    frame.extend_from_slice(&local);
    frame.extend_from_slice(&[0xff; 6]);
    frame.extend_from_slice(&[0x10, 0]);
    frame.extend_from_slice(&[1, 0, 0, 0xe0, 0, 0, 0, 0]);
    frame.extend_from_slice(&[
        0x7f, 0x18, 0xfe, 0x34, 0xa4, 0xc2, 0xe0, 0xfe, 0xdd, 9, 0x18, 0xfe, 0x34, 4, 2,
        0x43, 0x43, 0x4d, 0x50,
    ]);
    frame.extend_from_slice(&[0; 8]);
    let mut expected = frame.clone();
    remu_radio::protect_native_ccmp_frame(&key, &mut expected).unwrap();

    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    let mut buffer_bytes = Vec::new();
    buffer_bytes.extend_from_slice(&((frame.len() + 4) as u32).to_le_bytes());
    buffer_bytes.extend_from_slice(&0_u32.to_le_bytes());
    buffer_bytes.extend_from_slice(&frame);
    buffer_bytes.extend_from_slice(&[0; 4]);
    machine
        .debug_write_memory(u64::from(buffer), &buffer_bytes)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_4d6c,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();

    assert_eq!(machine.service_radio().unwrap(), 1);
    assert!(machine
        .radio_replay_artifact()
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(
            event,
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.protocol == remu_radio::RadioProtocol::Wifi
                    && request.frame.bytes == expected
        )));
    assert_ne!(&expected[32..expected.len() - 8], &frame[32..frame.len() - 8]);
}

#[test]
fn esp32c6_native_wifi_tx_without_a_selected_crypto_key_is_a_hard_error() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let descriptor = 0x4082_1000_u32;
    let buffer = 0x4082_1100_u32;
    let mut frame = vec![0xd0, 0x40, 0, 0];
    frame.extend_from_slice(&[0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0x01]);
    frame.extend_from_slice(&[0x52, 0x45, 0x02, 0, 0, 1]);
    frame.extend_from_slice(&[0xff; 6]);
    frame.extend_from_slice(&[0x10, 0]);
    frame.extend_from_slice(&[1, 0, 0, 0xe0, 0, 0, 0, 0]);
    frame.extend_from_slice(&[0; 16]);
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    let mut buffer_bytes = Vec::new();
    buffer_bytes.extend_from_slice(&((frame.len() + 4) as u32).to_le_bytes());
    buffer_bytes.extend_from_slice(&0_u32.to_le_bytes());
    buffer_bytes.extend_from_slice(&frame);
    machine
        .debug_write_memory(u64::from(buffer), &buffer_bytes)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_4d6c,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();

    let MachineError::RadioLegality(error) = machine.service_radio().unwrap_err() else {
        panic!("expected a radio legality error");
    };
    assert_eq!(
        error.rule,
        remu_radio::RadioLegalityRule::CryptoKeySelection
    );
    assert!(error.detail.contains("does not match a valid interface"));
}


#[test]
fn esp32c6_matching_native_wifi_ack_completes_successfully() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let descriptor = 0x4082_1000_u32;
    let buffer = 0x4082_1100_u32;
    let transmitter = [0x02, 1, 2, 3, 4, 5];
    let mut frame = vec![0x08, 0x00, 0, 0];
    frame.extend_from_slice(&[0x02, 6, 7, 8, 9, 10]);
    frame.extend_from_slice(&transmitter);
    frame.extend_from_slice(&[0; 8]);
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    let mut buffer_bytes = Vec::new();
    buffer_bytes.extend_from_slice(&((frame.len() + 4) as u32).to_le_bytes());
    buffer_bytes.extend_from_slice(&0_u32.to_le_bytes());
    buffer_bytes.extend_from_slice(&frame);
    buffer_bytes.extend_from_slice(&[0; 4]);
    machine
        .debug_write_memory(u64::from(buffer), &buffer_bytes)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_4d6c,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);

    let tx_end = u64::try_from(frame.len()).unwrap() * 32;
    let mut ack = vec![0xd4, 0x00, 0, 0];
    ack.extend_from_slice(&transmitter);
    machine
        .inject_radio_frame_at(
            SimTime::from_ticks(tx_end),
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            ack,
            -35,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(tx_end + 320);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .bus
            .read(0x600a_54ec, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap() as u32
            & (0xf << 12),
        0
    );
}

#[test]
fn esp32c6_native_wifi_rts_receives_a_hardware_cts() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 9) | (1 << 10),
            SimTime::ZERO,
        )
        .unwrap();
    let local = [0x02, 6, 7, 8, 9, 10];
    let peer = [0x02, 1, 2, 3, 4, 5];
    let mut rts = vec![0xb4, 0, 0, 0];
    rts.extend_from_slice(&local);
    rts.extend_from_slice(&peer);
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            rts.clone(),
            -35,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(rts.len() as u64 * 32);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert!(machine
        .radio_replay_artifact()
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(
            event,
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.origin == remu_radio::FrameOrigin::Emulated
                    && request.frame.bytes == [0xc4, 0, 0, 0, 2, 1, 2, 3, 4, 5]
        )));
}

#[test]
fn esp32c6_native_wifi_tx_rejects_an_fcs_without_a_mac_frame() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let descriptor = 0x4082_1000_u32;
    let buffer = 0x4082_1100_u32;
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    machine
        .debug_write_memory(u64::from(buffer), &4_u32.to_le_bytes())
        .unwrap();
    machine
        .bus
        .write(
            0x600a_4d6c,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();

    let MachineError::RadioLegality(error) = machine.service_radio().unwrap_err() else {
        panic!("expected a radio legality error");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::DmaLength);
    assert!(error.detail.contains("does not contain a MAC frame"));
}

#[test]
fn esp32c6_overlapping_ieee802154_cca_is_a_hard_machine_error() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_9804,
            AccessWidth::Word,
            (1 << 23) | (1 << 24),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_3048, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x43, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x43, SimTime::ZERO)
        .unwrap();

    let error = machine.service_radio().unwrap_err();
    let MachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::OperationOverlap);
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Ieee802154);
}

#[test]
fn esp32c6_misaligned_native_ble_schedule_is_a_hard_machine_error() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 17) | (1 << 18),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_18fc, AccessWidth::Word, 0x102, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600a_1028, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();

    let error = machine.service_radio().unwrap_err();
    let MachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::DmaAddress);
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::BluetoothLe);
    assert!(error.to_string().contains("0x40800102"));
}

#[test]
fn esp32c6_native_ble_connection_rx_latches_success_once() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let schedule = 0x4080_1000_u32;
    machine
        .debug_write_memory(
            u64::from(schedule.wrapping_add(0x28)),
            &(1_u32 << 13).to_le_bytes(),
        )
        .unwrap();

    machine
        .mark_native_ble_connection_rx_success(schedule, false)
        .unwrap();
    assert_eq!(
        u32::from_le_bytes(
            machine
                .debug_read_memory(u64::from(schedule.wrapping_add(0x28)), 4)
                .unwrap()
                .try_into()
                .unwrap()
        ),
        (1 << 13) | (1 << 11)
    );
    let error = machine
        .mark_native_ble_connection_rx_success(schedule, false)
        .unwrap_err();
    let MachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::SchedulerState);
    assert!(error.to_string().contains("unexplained duplicate RX"));

    machine
        .mark_native_ble_connection_rx_success(schedule, true)
        .unwrap();

    machine
        .debug_write_memory(
            u64::from(schedule.wrapping_add(0x28)),
            &(1_u32 << 13).to_le_bytes(),
        )
        .unwrap();
    let error = machine
        .mark_native_ble_connection_rx_success(schedule, true)
        .unwrap_err();
    let MachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::SchedulerState);
    assert!(error
        .to_string()
        .contains("continued without an initial RX completion"));
}

#[test]
fn esp32c6_native_ble_connection_rx_rejects_released_schedule() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let schedule = 0x4080_1000_u32;
    machine
        .debug_write_memory(u64::from(schedule.wrapping_add(0x28)), &[0; 4])
        .unwrap();

    let error = machine
        .mark_native_ble_connection_rx_success(schedule, false)
        .unwrap_err();
    let MachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::SchedulerState);
    assert!(error.to_string().contains("after firmware released schedule"));
}

#[test]
fn esp32c6_native_ble_scan_writes_the_firmware_owned_rx_ring() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 17) | (1 << 18),
            SimTime::ZERO,
        )
        .unwrap();

    let schedule = 0x4080_1000_u32;
    let state = 0x4080_1100_u32;
    let header = 0x4080_1200_u32;
    let buffer = 0x4080_1300_u32;
    let mut schedule_bytes = vec![0_u8; 0x38];
    schedule_bytes[4..8].copy_from_slice(&state.to_le_bytes());
    schedule_bytes[8..12].copy_from_slice(&2_u32.to_le_bytes());
    schedule_bytes[12..16].copy_from_slice(&202_u32.to_le_bytes());
    schedule_bytes[0x28..0x2c].copy_from_slice(&(1_u32 << 13).to_le_bytes());
    schedule_bytes[0x35] = 2;
    machine
        .debug_write_memory(u64::from(schedule), &schedule_bytes)
        .unwrap();

    let mut state_bytes = vec![0_u8; 0x80];
    state_bytes[8..12].copy_from_slice(&header.wrapping_add(4).to_le_bytes());
    state_bytes[0x2c..0x30].copy_from_slice(&50_000_u32.to_le_bytes());
    state_bytes[0x5c..0x60].copy_from_slice(&header.to_le_bytes());
    machine
        .debug_write_memory(u64::from(state), &state_bytes)
        .unwrap();

    let mut header_bytes = vec![0_u8; 16];
    header_bytes[4..8].copy_from_slice(&header.wrapping_add(4).to_le_bytes());
    header_bytes[8..12].copy_from_slice(&buffer.to_le_bytes());
    machine
        .debug_write_memory(u64::from(header), &header_bytes)
        .unwrap();
    machine
        .debug_write_memory(u64::from(buffer), &[0_u8; 128])
        .unwrap();
    machine
        .debug_write_memory(u64::from(buffer) + 0x18, &0xffff_u32.to_le_bytes())
        .unwrap();

    machine
        .bus
        .write(
            0x600a_18fc,
            AccessWidth::Word,
            u64::from(schedule & 0x000f_ffff),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_1028, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);

    let frame = vec![
        0x42, 0x0c, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xc1, 0x02, 0x01, 0x06, 0x02, 0x09,
        0x52,
    ];
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::BluetoothLe,
            remu_radio::Spectrum::new(2_480_000, 2_000),
            "ble-1m",
            frame.clone(),
            -40,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(512);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .debug_read_memory(u64::from(buffer) + 0x1c, frame.len())
            .unwrap(),
        frame
    );
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_ne!(
        machine
            .bus
            .read(
                0x600a_130c,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & (1 << 27),
        0
    );
}

#[test]
fn esp32c6_ble_peripheral_sequence_tracks_ack_duplicates_and_control_pdus() {
    let mut sequence = super::radio::C6BleLinkSequence::default();
    let version = vec![3, 6, 12, 14, 0xe5, 2, 0, 0];
    assert_eq!(
        sequence.peripheral_response(&[0x01, 0], Some(version)).unwrap(),
        Some(vec![7, 6, 12, 14, 0xe5, 2, 0, 0])
    );

    assert_eq!(
        sequence.peripheral_response(&[0x0f, 0], None).unwrap(),
        Some(vec![9, 0])
    );
    assert_eq!(
        sequence.peripheral_response(&[0x0f, 0], None).unwrap(),
        Some(vec![9, 0]),
        "a duplicate central PDU retransmits the outstanding peripheral PDU"
    );

    let features = vec![3, 9, 14, 0xff, 0x7f, 1, 7, 0x90, 0x1b, 0, 0];
    assert_eq!(
        sequence
            .peripheral_response(&[0x01, 0], Some(features))
            .unwrap(),
        Some(vec![7, 9, 14, 0xff, 0x7f, 1, 7, 0x90, 0x1b, 0, 0])
    );

    let mut acl_sequence = super::radio::C6BleLinkSequence::default();
    let att_mtu_response = vec![2, 7, 3, 0, 4, 0, 3, 0, 1];
    let transmitted = acl_sequence
        .peripheral_response(&[0x02, 0], Some(att_mtu_response))
        .unwrap()
        .unwrap();
    assert_eq!(transmitted, vec![6, 7, 3, 0, 4, 0, 3, 0, 1]);
    assert_eq!(
        acl_sequence.peripheral_response(&[0x02, 0], None).unwrap(),
        Some(transmitted),
        "an unacknowledged ACL data PDU is retransmitted byte-for-byte"
    );
    assert_eq!(
        acl_sequence.peripheral_response(&[0x0e, 0], None).unwrap(),
        Some(vec![9, 0]),
        "the central NESN retires the ACL PDU and advances peripheral SN"
    );

    let mut phy_sequence = super::radio::C6BleLinkSequence::default();
    assert_eq!(phy_sequence.begin_event().unwrap(), "ble-1m");
    phy_sequence
        .peripheral_response(&[3, 5, 0x18, 2, 2, 6, 0], None)
        .unwrap();
    for _ in 1..6 {
        assert_eq!(phy_sequence.begin_event().unwrap(), "ble-1m");
    }
    assert_eq!(phy_sequence.begin_event().unwrap(), "ble-2m");
    assert_eq!(phy_sequence.tx_phy(), "ble-2m");

    let mut illegal_phy_sequence = super::radio::C6BleLinkSequence::default();
    illegal_phy_sequence.begin_event().unwrap();
    assert!(
        illegal_phy_sequence
            .peripheral_response(&[3, 5, 0x18, 2, 2, 5, 0], None)
            .unwrap_err()
            .contains("is 5 events after")
    );
}

#[test]
fn esp32c6_native_wifi_rx_dma_writes_metadata_frame_and_completion() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 9) | (1 << 10) | (1 << 17) | (1 << 18),
            SimTime::ZERO,
        )
        .unwrap();
    let descriptor = 0x4080_1000_u32;
    let buffer = 0x4080_1100_u32;
    let capacity = 512_u32;
    let control = (1 << 31) | (capacity << 14) | capacity;
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&control.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_4084,
            AccessWidth::Word,
            u64::from(descriptor),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_4c40, AccessWidth::Word, 1 << 14, SimTime::ZERO)
        .unwrap();
    let frame = vec![0x80, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2, 6];
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            frame.clone(),
            -40,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(512);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .debug_read_memory(u64::from(buffer) + 92, frame.len())
            .unwrap(),
        frame
    );
    let completed = u32::from_le_bytes(
        machine
            .debug_read_memory(u64::from(descriptor), 4)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    assert_eq!(completed & (1 << 31), 0);
    assert_ne!(completed & (1 << 30), 0);
    assert_eq!((completed >> 14) & 0x3fff, 108);
    assert_ne!(
        machine
            .bus
            .read(
                0x600a_4c48,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & (1 << 14),
        0
    );
}
