#[test]
fn esp32s3_unmapped_native_ble_slot_is_a_hard_machine_error() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .bus
        .write(0x6003_1100, AccessWidth::Word, 1 << 31, SimTime::ZERO)
        .unwrap();

    let error = machine.service_radio().unwrap_err();
    let XtensaMachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::MemoryMapping);
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::BluetoothLe);
    assert!(error.detail.contains("scheduler event slot"));
}
#[test]
fn esp32s3_invalid_native_ble_channel_is_a_hard_machine_error() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let slot_address = 0x3fca_1000_u32;
    let cs_address = 0x3fca_2000_u32;
    for (index, (em_offset, cpu_address)) in [(0x0000_u32, slot_address), (0x0400, cs_address)]
        .into_iter()
        .enumerate()
    {
        let mapping = ((em_offset >> 2) << 18) | ((cpu_address & 0x000f_ffff) >> 2);
        machine
            .bus
            .write(
                0x6003_1204 + index as u64 * 4,
                AccessWidth::Word,
                u64::from(mapping),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x6003_12c4, AccessWidth::Word, 0x03, SimTime::ZERO)
        .unwrap();
    let mut slot = [0_u8; 16];
    slot[6..8].copy_from_slice(&624_u16.to_le_bytes());
    slot[8..10].copy_from_slice(&0x0200_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(slot_address), &slot)
        .unwrap();
    let mut control_structure = [0_u8; 34];
    control_structure[12..16].copy_from_slice(&0x8e89_bed6_u32.to_le_bytes());
    control_structure[22..24].copy_from_slice(&63_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(cs_address), &control_structure)
        .unwrap();
    machine
        .bus
        .write(0x6003_1100, AccessWidth::Word, 1 << 31, SimTime::ZERO)
        .unwrap();

    let error = machine.service_radio().unwrap_err();
    let XtensaMachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::SchedulerState);
    assert!(error.detail.contains("invalid BLE channel 63"));
}

#[test]
fn esp32s3_native_ble_scheduler_transmits_exchange_memory_pdu_and_completes() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let slot_address = 0x3fca_1000_u32;
    let cs_address = 0x3fca_2000_u32;
    let descriptor_address = 0x3fca_3000_u32;
    let payload_address = 0x3fca_4000_u32;
    for (index, (em_offset, cpu_address)) in [
        (0x0000_u32, slot_address),
        (0x0400, cs_address),
        (0x1400, descriptor_address),
        (0x2400, payload_address),
    ]
    .into_iter()
    .enumerate()
    {
        let mapping = ((em_offset >> 2) << 18) | ((cpu_address & 0x000f_ffff) >> 2);
        machine
            .bus
            .write(
                0x6003_1204 + index as u64 * 4,
                AccessWidth::Word,
                u64::from(mapping),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x6003_12c4, AccessWidth::Word, 0x0f, SimTime::ZERO)
        .unwrap();

    let mut slot = [0_u8; 16];
    slot[0..2].copy_from_slice(&0x2802_u16.to_le_bytes());
    slot[2..6].copy_from_slice(&2_u32.to_le_bytes());
    slot[6..8].copy_from_slice(&624_u16.to_le_bytes());
    // The scheduler stores the 90-byte BLE control-structure offset divided by two.
    slot[8..10].copy_from_slice(&0x0200_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(slot_address), &slot)
        .unwrap();

    let mut control_structure = [0_u8; 90];
    control_structure[12..16].copy_from_slice(&0x8e89_bed6_u32.to_le_bytes());
    control_structure[16..20].copy_from_slice(&0x0055_5555_u32.to_le_bytes());
    control_structure[22..24].copy_from_slice(&39_u16.to_le_bytes());
    control_structure[28..30].copy_from_slice(&0x1400_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(cs_address), &control_structure)
        .unwrap();

    let advertising_data = b"\x02\x01\x06\x0b\x09Renvo-BLE1";
    let mut descriptor = [0_u8; 32];
    descriptor[2..4].copy_from_slice(&0x1542_u16.to_le_bytes());
    descriptor[4..6].copy_from_slice(&0x2400_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor_address), &descriptor)
        .unwrap();
    machine
        .debug_write_memory(u64::from(payload_address), advertising_data)
        .unwrap();
    machine
        .bus
        .write(0x6003_1100, AccessWidth::Word, 1 << 31, SimTime::ZERO)
        .unwrap();

    assert_eq!(machine.service_radio().unwrap(), 0);
    machine.now = SimTime::from_ticks(9_999);
    assert_eq!(machine.service_radio().unwrap(), 0);
    machine.now = SimTime::from_ticks(10_000);
    assert_eq!(machine.service_radio().unwrap(), 1);
    let replay = machine.radio_replay_artifact();
    let request = replay
        .events
        .iter()
        .find_map(|event| match event {
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.protocol == remu_radio::RadioProtocol::BluetoothLe =>
            {
                Some(request)
            }
            _ => None,
        })
        .unwrap();
    let mut expected = vec![0x42, 0x15, 0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
    expected.extend_from_slice(advertising_data);
    assert_eq!(request.frame.bytes, expected);
    assert_eq!(request.frame.spectrum.center_khz, 2_480_000);
    assert_eq!(request.frame.origin, remu_radio::FrameOrigin::Emulated);
    assert_eq!(request.start, SimTime::from_ticks(10_000));
    assert_eq!(
        machine
            .bus
            .read(
                0x6003_1010,
                AccessWidth::Word,
                remu_core::AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );

    machine.now = SimTime::from_ticks(10_000 + (expected.len() * 8) as u64);
    machine.service_radio().unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                u64::from(slot_address),
                AccessWidth::HalfWord,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & 0x38,
        2 << 3
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x6003_1010,
                AccessWidth::Word,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        0
    );
    machine.now = SimTime::from_ticks(10_000 + (expected.len() * 8) as u64 + 2_400);
    machine.service_radio().unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                u64::from(slot_address),
                AccessWidth::HalfWord,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & 0x38,
        4 << 3
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x6003_1010,
                AccessWidth::Word,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        1 << 5
    );
}

#[test]
fn esp32s3_native_ble_scan_writes_receive_ring_metadata_and_interrupt() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let slot_address = 0x3fca_1000_u32;
    let cs_address = 0x3fca_2000_u32;
    let descriptor_address = 0x3fca_3000_u32;
    let payload_address = 0x3fca_4000_u32;
    for (index, (em_offset, cpu_address)) in [
        (0x0000_u32, slot_address),
        (0x0400, cs_address),
        (0x1000, descriptor_address),
        (0x3000, payload_address),
    ]
    .into_iter()
    .enumerate()
    {
        let mapping = ((em_offset >> 2) << 18) | ((cpu_address & 0x000f_ffff) >> 2);
        machine
            .bus
            .write(
                0x6003_1204 + index as u64 * 4,
                AccessWidth::Word,
                u64::from(mapping),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x6003_12c4, AccessWidth::Word, 0x0f, SimTime::ZERO)
        .unwrap();

    let mut slot = [0_u8; 16];
    slot[0..2].copy_from_slice(&0x0208_u16.to_le_bytes());
    slot[2..6].copy_from_slice(&2_u32.to_le_bytes());
    slot[6..8].copy_from_slice(&624_u16.to_le_bytes());
    slot[8..10].copy_from_slice(&0x0200_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(slot_address), &slot)
        .unwrap();
    let mut control_structure = [0_u8; 90];
    control_structure[12..16].copy_from_slice(&0x8e89_bed6_u32.to_le_bytes());
    control_structure[22..24].copy_from_slice(&39_u16.to_le_bytes());
    control_structure[32..34].copy_from_slice(&16_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(cs_address), &control_structure)
        .unwrap();
    let mut descriptor = [0_u8; 20];
    descriptor[0..2].copy_from_slice(&0x1000_u16.to_le_bytes());
    descriptor[2..4].copy_from_slice(&0x8000_u16.to_le_bytes());
    descriptor[18..20].copy_from_slice(&0x3000_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor_address), &descriptor)
        .unwrap();
    machine
        .bus
        .write(0x6003_1100, AccessWidth::Word, 1 << 31, SimTime::ZERO)
        .unwrap();
    machine.service_radio().unwrap();

    let pdu = vec![
        0x42, 0x0c, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xc1, 0x02, 0x01, 0x06, 0x02, 0x09, 0x52,
    ];
    machine
        .inject_radio_frame_at(
            SimTime::from_ticks(10_000),
            remu_radio::RadioProtocol::BluetoothLe,
            remu_radio::Spectrum::new(2_480_000, 2_000),
            "ble-1m",
            pdu.clone(),
            -40,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(10_000 + (pdu.len() * 8) as u64);
    assert_eq!(machine.service_radio().unwrap(), 1);

    let completed = machine
        .debug_read_memory(u64::from(descriptor_address), 20)
        .unwrap();
    assert_eq!(
        u16::from_le_bytes(completed[0..2].try_into().unwrap()),
        0x9000
    );
    assert_eq!(u16::from_le_bytes(completed[2..4].try_into().unwrap()), 0);
    assert_eq!(
        u16::from_le_bytes(completed[4..6].try_into().unwrap()),
        0x0c42
    );
    assert_eq!(
        u16::from_le_bytes(completed[12..14].try_into().unwrap()),
        624
    );
    assert_eq!(completed[6], 0xb0);
    assert_eq!(
        u16::from_le_bytes(completed[14..16].try_into().unwrap()),
        0
    );
    assert_eq!(
        machine
            .debug_read_memory(u64::from(payload_address), pdu.len() - 2)
            .unwrap(),
        pdu[2..]
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x6003_1010,
                AccessWidth::Word,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        1 << 2
    );
}

#[test]
fn esp32s3_native_ble_connection_retires_acknowledged_tx_and_synthesizes_empty_pdu() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let slot_address = 0x3fca_1000_u32;
    let cs_address = 0x3fca_2000_u32;
    let tx_descriptor_address = 0x3fca_3000_u32;
    let tx_payload_address = 0x3fca_4000_u32;
    let rx_descriptor_address = 0x3fca_5000_u32;
    let rx_payload_address = 0x3fca_6000_u32;
    for (index, (em_offset, cpu_address)) in [
        (0x0000_u32, slot_address),
        (0x0400, cs_address),
        (0x1400, tx_descriptor_address),
        (0x2400, tx_payload_address),
        (0x1000, rx_descriptor_address),
        (0x3000, rx_payload_address),
    ]
    .into_iter()
    .enumerate()
    {
        let mapping = ((em_offset >> 2) << 18) | ((cpu_address & 0x000f_ffff) >> 2);
        machine
            .bus
            .write(
                0x6003_1204 + index as u64 * 4,
                AccessWidth::Word,
                u64::from(mapping),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x6003_12c4, AccessWidth::Word, 0x3f, SimTime::ZERO)
        .unwrap();

    let mut slots = [0_u8; 32];
    for (index, coarse) in [2_u32, 4].into_iter().enumerate() {
        let offset = index * 16;
        slots[offset..offset + 2].copy_from_slice(&0x2802_u16.to_le_bytes());
        slots[offset + 2..offset + 6].copy_from_slice(&coarse.to_le_bytes());
        slots[offset + 6..offset + 8].copy_from_slice(&624_u16.to_le_bytes());
        slots[offset + 8..offset + 10].copy_from_slice(&0x0200_u16.to_le_bytes());
    }
    machine
        .debug_write_memory(u64::from(slot_address), &slots)
        .unwrap();

    let mut control_structure = [0_u8; 90];
    control_structure[12..16].copy_from_slice(&0xa1b2_c3d4_u32.to_le_bytes());
    control_structure[22..24].copy_from_slice(&5_u16.to_le_bytes());
    control_structure[28..30].copy_from_slice(&0x1400_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(cs_address), &control_structure)
        .unwrap();

    let mut tx_descriptors = [0_u8; 32];
    tx_descriptors[0..2].copy_from_slice(&0x1410_u16.to_le_bytes());
    tx_descriptors[2..4].copy_from_slice(&0x0613_u16.to_le_bytes());
    tx_descriptors[4..6].copy_from_slice(&0x2400_u16.to_le_bytes());
    tx_descriptors[16..18].copy_from_slice(&0x9400_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(tx_descriptor_address), &tx_descriptors)
        .unwrap();
    machine
        .debug_write_memory(
            u64::from(tx_payload_address),
            &[12, 9, 0xe5, 2, 22, 0],
        )
        .unwrap();

    let mut rx_descriptors = [0_u8; 40];
    rx_descriptors[0..2].copy_from_slice(&0x1014_u16.to_le_bytes());
    rx_descriptors[2..4].copy_from_slice(&0x8000_u16.to_le_bytes());
    rx_descriptors[18..20].copy_from_slice(&0x3000_u16.to_le_bytes());
    rx_descriptors[20..22].copy_from_slice(&0x1000_u16.to_le_bytes());
    rx_descriptors[22..24].copy_from_slice(&0x8000_u16.to_le_bytes());
    rx_descriptors[38..40].copy_from_slice(&0x3100_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(rx_descriptor_address), &rx_descriptors)
        .unwrap();

    machine
        .bus
        .write(0x6003_1100, AccessWidth::Word, 1 << 31, SimTime::ZERO)
        .unwrap();
    machine.service_radio().unwrap();
    machine
        .inject_radio_frame_at(
            SimTime::from_ticks(10_000),
            remu_radio::RadioProtocol::BluetoothLe,
            remu_radio::Spectrum::new(2_414_000, 2_000),
            "ble-1m",
            vec![1, 0],
            -40,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(10_016);
    machine.service_radio().unwrap();
    machine.now = SimTime::from_ticks(12_416);
    assert_eq!(machine.service_radio().unwrap(), 1);
    machine.now = SimTime::from_ticks(15_168);
    machine.service_radio().unwrap();

    machine
        .bus
        .write(
            0x6003_1100,
            AccessWidth::Word,
            (1 << 31) | 1,
            machine.now,
        )
        .unwrap();
    machine.service_radio().unwrap();
    machine
        .inject_radio_frame_at(
            SimTime::from_ticks(20_000),
            remu_radio::RadioProtocol::BluetoothLe,
            remu_radio::Spectrum::new(2_414_000, 2_000),
            "ble-1m",
            vec![15, 6, 12, 9, 0xff, 0xff, 1, 0],
            -40,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(20_064);
    machine.service_radio().unwrap();

    let completed = machine
        .debug_read_memory(u64::from(tx_descriptor_address), 18)
        .unwrap();
    assert_eq!(
        u16::from_le_bytes(completed[0..2].try_into().unwrap()),
        0x9410
    );
    assert_eq!(
        u16::from_le_bytes(completed[16..18].try_into().unwrap()),
        0x1400
    );
    assert_eq!(
        machine
            .bus
            .read(
                u64::from(cs_address + 28),
                AccessWidth::HalfWord,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        0x1410
    );

    machine.now = SimTime::from_ticks(22_464);
    assert_eq!(machine.service_radio().unwrap(), 1);
    let transmitted: Vec<Vec<u8>> = machine
        .radio_replay_artifact()
        .events
        .iter()
        .filter_map(|event| match event {
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.protocol == remu_radio::RadioProtocol::BluetoothLe
                    && request.frame.origin == remu_radio::FrameOrigin::Emulated =>
            {
                Some(request.frame.bytes.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(transmitted, vec![vec![0x17, 6, 12, 9, 0xe5, 2, 22, 0], vec![9, 0]]);
}
