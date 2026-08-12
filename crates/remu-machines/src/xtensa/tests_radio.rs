#[test]
fn esp32s3_wifi_and_ble_use_shared_deterministic_radio_api() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let mut wifi_frame = vec![0_u8; 24];
    wifi_frame[4..10].fill(0xff);
    machine
        .wifi_engine()
        .unwrap()
        .start(remu_radio::WifiMode::Station)
        .unwrap();
    machine
        .wifi_engine()
        .unwrap()
        .queue_tx(wifi_frame.clone())
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert!(
        machine
            .radio_replay_artifact()
            .events
            .iter()
            .any(|event| matches!(
                event,
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.protocol == remu_radio::RadioProtocol::Wifi
                        && request.frame.origin == remu_radio::FrameOrigin::Emulated
            ))
    );
    machine.now = SimTime::from_ticks(192);
    assert_eq!(machine.service_radio().unwrap(), 0);

    machine
        .ble_controller()
        .unwrap()
        .process_h4(&[1, 3, 12, 0])
        .unwrap();
    assert_eq!(
        machine.ble_controller().unwrap().take_h4_output(),
        Some(vec![4, 0x0e, 4, 1, 3, 12, 0])
    );

    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            wifi_frame.clone(),
            0,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(384);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(machine.wifi_engine().unwrap().take_rx(), Some(wifi_frame));
}

#[test]
fn esp32s3_coexistence_preempts_airtime_and_denies_lower_priority_work() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let mut wifi_frame = vec![0_u8; 24];
    wifi_frame[4..10].fill(0xff);
    machine
        .wifi_engine()
        .unwrap()
        .start(remu_radio::WifiMode::Station)
        .unwrap();
    machine
        .wifi_engine()
        .unwrap()
        .queue_tx(wifi_frame)
        .unwrap();
    machine
        .ble_controller()
        .unwrap()
        .process_h4(&[1, 0x0a, 0x20, 1, 1])
        .unwrap();

    // Functional frames are collected in deterministic Wi-Fi/BLE order. BLE
    // therefore preempts the lower-priority Wi-Fi grant at the same timestamp.
    assert_eq!(machine.service_radio().unwrap(), 2);
    let artifact = machine.radio_replay_artifact();
    let wifi_id = artifact.events.iter().find_map(|event| match event {
        remu_radio::MediumEvent::Submitted { id, request }
            if request.frame.protocol == remu_radio::RadioProtocol::Wifi =>
        {
            Some(*id)
        }
        _ => None,
    });
    assert!(artifact.events.iter().any(|event| matches!(
        event,
        remu_radio::MediumEvent::Truncated { id, at }
            if Some(*id) == wifi_id && *at == SimTime::ZERO
    )));
    assert!(artifact.coexistence_events.iter().any(|event| matches!(
        event,
        remu_radio::CoexistenceEvent::Preempted {
            protocol: remu_radio::RadioProtocol::Wifi,
            by: remu_radio::RadioProtocol::BluetoothLe,
            ..
        }
    )));

    // The BLE grant remains active. A second Wi-Fi frame at the same timestamp
    // is denied and never appears as medium airtime.
    let submitted_before = artifact
        .events
        .iter()
        .filter(|event| matches!(event, remu_radio::MediumEvent::Submitted { .. }))
        .count();
    machine
        .wifi_engine()
        .unwrap()
        .queue_tx(vec![0_u8; 24])
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 0);
    let artifact = machine.radio_replay_artifact();
    assert_eq!(
        artifact
            .events
            .iter()
            .filter(|event| matches!(event, remu_radio::MediumEvent::Submitted { .. }))
            .count(),
        submitted_before
    );
    assert!(artifact.coexistence_events.iter().any(|event| matches!(
        event,
        remu_radio::CoexistenceEvent::Denied {
            protocol: remu_radio::RadioProtocol::Wifi,
            owner: remu_radio::RadioProtocol::BluetoothLe,
            ..
        }
    )));
}

#[test]
fn esp32s3_shared_radio_reset_cancels_active_coexistence_ownership() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .wifi_engine()
        .unwrap()
        .start(remu_radio::WifiMode::Station)
        .unwrap();
    machine
        .wifi_engine()
        .unwrap()
        .queue_tx(vec![0_u8; 24])
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);
    machine
        .bus
        .write(0x6002_6018, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();

    machine.service_radio().unwrap();

    assert_eq!(machine.radio_coexistence.owner(), None);
    assert!(matches!(
        machine.radio_coexistence.events().last(),
        Some(remu_radio::CoexistenceEvent::Reset { at }) if *at == SimTime::ZERO
    ));
    assert!(matches!(
        machine.radio_replay_artifact().events.last(),
        Some(remu_radio::MediumEvent::Truncated { at, .. }) if *at == SimTime::ZERO
    ));
}

#[test]
fn esp32s3_radio_power_gate_truncates_active_airtime() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .wifi_engine()
        .unwrap()
        .start(remu_radio::WifiMode::Station)
        .unwrap();
    machine
        .wifi_engine()
        .unwrap()
        .queue_tx(vec![0_u8; 24])
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);
    machine
        .bus
        .write(0x6002_6014, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();

    machine.service_radio().unwrap();

    assert_eq!(machine.radio_coexistence.owner(), None);
    assert!(matches!(
        machine.radio_coexistence.events().last(),
        Some(remu_radio::CoexistenceEvent::PowerDown {
            protocol: remu_radio::RadioProtocol::Wifi,
            at,
            ..
        }) if *at == SimTime::ZERO
    ));
    assert!(matches!(
        machine.radio_replay_artifact().events.last(),
        Some(remu_radio::MediumEvent::Truncated { at, .. }) if *at == SimTime::ZERO
    ));
}

#[test]
fn esp32s3_power_gated_unmapped_grant_is_a_hard_machine_error() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .radio_coexistence
        .request(remu_radio::CoexistenceRequest {
            protocol: remu_radio::RadioProtocol::Wifi,
            start: SimTime::ZERO,
            duration: remu_core::SimDuration::from_ticks(100),
            priority: 8,
            preemptible: true,
        })
        .unwrap();
    machine
        .bus
        .write(0x6002_6014, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();

    let XtensaMachineError::RadioLegality(error) = machine.service_radio().unwrap_err() else {
        panic!("expected a radio legality error");
    };
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Coexistence);
    assert_eq!(
        error.rule,
        remu_radio::RadioLegalityRule::CoexistenceOwnership
    );
    assert!(error.detail.contains("no matching RF transmission"));
}

#[test]
fn esp32s3_illegal_native_wifi_dma_is_a_hard_machine_error() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .bus
        .write(
            0x6003_3d08,
            AccessWidth::Word,
            (3_u64 << 30) | 2,
            SimTime::ZERO,
        )
        .unwrap();

    let error = machine.service_radio().unwrap_err();
    let XtensaMachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::DmaAddress);
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Wifi);
    assert!(error.to_string().contains("0x3fc00002"));
}

#[test]
fn esp32s3_native_wifi_tx_accepts_the_firmware_fcs_allowance_only() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let descriptor = 0x3fca_1000_u32;
    let buffer = 0x3fca_1102_u32;
    let frame: Vec<u8> = (0_u8..30).collect();
    let control = 32_u32 | (34_u32 << 12) | (1 << 31);
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&control.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    let mut buffer_bytes = frame.clone();
    buffer_bytes.extend_from_slice(&[0xa5; 4]);
    machine
        .debug_write_memory(u64::from(buffer), &buffer_bytes)
        .unwrap();
    machine
        .bus
        .write(
            0x6003_3d08,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();

    assert_eq!(machine.service_radio().unwrap(), 1);
    assert!(machine
        .radio_replay_artifact()
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
fn esp32s3_native_wifi_ack_timeout_is_delayed_and_publishes_vendor_status_five() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let descriptor = 0x3fca_1000_u32;
    let buffer = 0x3fca_1100_u32;
    let mut frame = vec![0x08, 0x00, 0, 0];
    frame.extend_from_slice(&[0x02, 6, 7, 8, 9, 10]);
    frame.extend_from_slice(&[0x02, 1, 2, 3, 4, 5]);
    frame.extend_from_slice(&[0; 8]);
    let control = frame.len() as u32 | ((frame.len() as u32 + 4) << 12) | (1 << 31);
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&control.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    machine
        .debug_write_memory(u64::from(buffer), &frame)
        .unwrap();
    machine
        .bus
        .write(0x6003_3d04, AccessWidth::Word, 300, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x6003_3d08,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();

    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .bus
            .read(0x6003_3cb0, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap(),
        0
    );
    let deadline = u64::try_from(frame.len()).unwrap() * 8 + 300;
    machine.now = SimTime::from_ticks(deadline);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .bus
            .read(0x6003_4320, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap() as u32
            & (0xf << 12),
        5 << 12
    );
}

#[test]
fn esp32s3_matching_native_wifi_ack_completes_successfully() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let descriptor = 0x3fca_1000_u32;
    let buffer = 0x3fca_1100_u32;
    let transmitter = [0x02, 1, 2, 3, 4, 5];
    let mut frame = vec![0x08, 0x00, 0, 0];
    frame.extend_from_slice(&[0x02, 6, 7, 8, 9, 10]);
    frame.extend_from_slice(&transmitter);
    frame.extend_from_slice(&[0; 8]);
    let control = frame.len() as u32 | ((frame.len() as u32 + 4) << 12) | (1 << 31);
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&control.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    machine
        .debug_write_memory(u64::from(buffer), &frame)
        .unwrap();
    machine
        .bus
        .write(
            0x6003_3d08,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);

    let tx_end = u64::try_from(frame.len()).unwrap() * 8;
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
    machine.now = SimTime::from_ticks(tx_end + 80);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .bus
            .read(0x6003_4320, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap() as u32
            & (0xf << 12),
        0
    );
}

#[test]
fn esp32s3_native_wifi_rx_ba_session_answers_a_compressed_bar() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let local = [0x02, 6, 7, 8, 9, 10];
    let peer = [0x02, 1, 2, 3, 4, 5];
    for (address, value) in [
        (0x6003_327c, u32::from_le_bytes(peer[..4].try_into().unwrap())),
        (
            0x6003_3278,
            u32::from(u16::from_le_bytes(peer[4..].try_into().unwrap())),
        ),
        (0x6003_3280, 0x123),
        (0x6003_3274, (3 << 30) | (3 << 12) | 5),
    ] {
        machine
            .bus
            .write(
                address,
                AccessWidth::Word,
                u64::from(value),
                SimTime::ZERO,
            )
            .unwrap();
    }

    let mut qos = vec![0x88, 0, 0, 0];
    qos.extend_from_slice(&local);
    qos.extend_from_slice(&peer);
    qos.extend_from_slice(&[0; 6]);
    qos.extend_from_slice(&(0x124_u16 << 4).to_le_bytes());
    qos.extend_from_slice(&[((3 << 5) | 3), 0]);
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            qos.clone(),
            -35,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(qos.len() as u64 * 8);
    machine.service_radio().unwrap();

    let mut bar = vec![0x84, 0, 0, 0];
    bar.extend_from_slice(&local);
    bar.extend_from_slice(&peer);
    bar.extend_from_slice(&(0x0004_u16 | (3_u16 << 12)).to_le_bytes());
    bar.extend_from_slice(&(0x123_u16 << 4).to_le_bytes());
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            bar.clone(),
            -35,
        )
        .unwrap();
    machine.now = machine
        .now
        .checked_add(remu_core::SimDuration::from_ticks(bar.len() as u64 * 8))
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);

    let block_ack = machine
        .radio_replay_artifact()
        .events
        .iter()
        .find_map(|event| match event {
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.origin == remu_radio::FrameOrigin::Emulated
                    && request.frame.bytes.starts_with(&[0x94, 0]) =>
            {
                Some(request.frame.bytes.clone())
            }
            _ => None,
        })
        .expect("native MAC emitted compressed block ACK");
    assert_eq!(&block_ack[4..10], &peer);
    assert_eq!(&block_ack[10..16], &local);
    assert_eq!(&block_ack[20..28], &2_u64.to_le_bytes());
}

#[test]
fn esp32s3_native_wifi_tx_rejects_odd_payload_addresses() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let descriptor = 0x3fca_1000_u32;
    let buffer = 0x3fca_1101_u32;
    let control = 32_u32 | (34_u32 << 12) | (1 << 31);
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&control.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    machine
        .bus
        .write(
            0x6003_3d08,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();

    let XtensaMachineError::RadioLegality(error) = machine.service_radio().unwrap_err() else {
        panic!("expected a radio legality error");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::DmaAddress);
    assert!(error.detail.contains("2-byte aligned"));
}

#[test]
fn esp32s3_native_wifi_tx_rejects_payload_beyond_the_fcs_allowance() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let descriptor = 0x3fca_1000_u32;
    let buffer = 0x3fca_1100_u32;
    let control = 32_u32 | (37_u32 << 12) | (1 << 31);
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&control.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    machine
        .bus
        .write(
            0x6003_3d08,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();

    let XtensaMachineError::RadioLegality(error) = machine.service_radio().unwrap_err() else {
        panic!("expected a radio legality error");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::DmaLength);
    assert!(error.detail.contains("exceeds descriptor capacity 32"));
}

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

#[test]
fn esp32s3_native_wifi_rx_dma_writes_metadata_frame_and_completion() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let descriptor = 0x3fca_1000_u32;
    let buffer = 0x3fca_1100_u32;
    let capacity = 512_u32;
    let control = (1 << 31) | (capacity << 12) | capacity;
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
            0x6003_3088,
            AccessWidth::Word,
            u64::from(descriptor),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x6003_3c34, AccessWidth::Word, 1 << 14, SimTime::ZERO)
        .unwrap();
    let frame = vec![0x80, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2, 3];
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            frame.clone(),
            -40,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(256);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .debug_read_memory(u64::from(buffer) + 48, frame.len())
            .unwrap(),
        frame
    );
    let metadata = machine.debug_read_memory(u64::from(buffer), 48).unwrap();
    assert_eq!(metadata[0], (-40_i8) as u8);
    assert_eq!(metadata[3] & (1 << 4), 1 << 4);
    assert_eq!(metadata[8] & (1 << 1), 1 << 1);
    assert_eq!(metadata[10], 0);
    assert_eq!(metadata[11] & 0x0f, 1);
    assert_eq!(metadata[11] & (1 << 7), 1 << 7);
    assert_eq!(metadata[20], (-95_i8) as u8);
    assert_eq!(
        u32::from_le_bytes(metadata[44..48].try_into().unwrap()) & 0x0fff,
        16
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
    assert_eq!((completed >> 12) & 0x0fff, 64);
    assert_ne!(
        machine
            .bus
            .read(
                0x6003_3c3c,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & (1 << 14),
        0
    );
}
