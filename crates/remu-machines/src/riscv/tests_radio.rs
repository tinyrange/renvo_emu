#[test]
fn esp32c6_radio_frontend_exposes_clock_split_and_ieee802154_events() {
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
        .write(0x600a_3060, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x41, SimTime::ZERO)
        .unwrap();
    let handles = machine.esp32c6_peripherals.as_ref().unwrap();
    assert!(handles.modem.ieee802154_ready());
    assert_eq!(
        handles.ieee802154.take_command(),
        Some(remu_devices::EspIeee802154Command::TxStart)
    );
    handles.ieee802154.complete_tx();
    assert!(handles.ieee802154.interrupt_pending());
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_3064,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap()
            & 1,
        1
    );
}

#[test]
fn esp32c6_phy_i2c_command_memory_retains_firmware_program_words() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    for (address, value) in [
        (0x600a_fc00, 0x0060_0267),
        (0x600a_fc04, 0x0720_026b),
        (0x600a_fc70, 0x0020_f667),
    ] {
        machine
            .bus
            .write(address, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    address,
                    AccessWidth::Word,
                    remu_core::AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            value
        );
    }
}

#[test]
fn esp32c6_ieee802154_dma_transmit_and_explicit_host_receive_use_shared_medium() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let tx_address = 0x4080_0100_u32;
    let rx_address = 0x4080_0200_u32;
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
        .write(0x600a_3054, AccessWidth::Word, 0xb5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_30d0,
            AccessWidth::Word,
            u64::from(tx_address),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            0x600a_30e0,
            AccessWidth::Word,
            u64::from(rx_address),
            SimTime::ZERO,
        )
        .unwrap();
    for (offset, byte) in [5_u8, 0x61, 0x88, 0x01, 0, 0].into_iter().enumerate() {
        machine
            .bus
            .write(
                u64::from(tx_address) + offset as u64,
                AccessWidth::Byte,
                u64::from(byte),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x41, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);
    machine.now = SimTime::from_ticks(160);
    assert_eq!(machine.service_radio().unwrap(), 1);
    let replay = machine.radio_replay_artifact().unwrap();
    assert!(replay.events.iter().any(|event| matches!(
        event,
        remu_radio::MediumEvent::Submitted { request, .. }
            if request.frame.bytes
                == remu_radio::Ieee802154Mac::with_fcs(vec![0x61, 0x88, 0x01])
                && request.frame.origin == remu_radio::FrameOrigin::Emulated
    )));

    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x42, machine.now)
        .unwrap();
    machine
        .bus
        .write(0x600a_3004, AccessWidth::Word, 1 << 7, machine.now)
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Ieee802154,
            remu_radio::Spectrum::new(2_405_000, 2_000),
            "ieee802154-oqpsk-250k",
            remu_radio::Ieee802154Mac::with_fcs(vec![0x01, 0x00, 0x02, 0xaa]),
            0,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(352);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine.debug_read_memory(u64::from(rx_address), 7).unwrap(),
        [6, 0x01, 0x00, 0x02, 0xaa, (-40_i8) as u8, 191]
    );
}

#[test]
fn esp32c6_ieee802154_ack_request_enters_native_rx_ack_and_completes_matching_sequence() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let tx_address = 0x4080_0280_u32;
    let rx_address = 0x4080_02c0_u32;
    machine
        .bus
        .write(
            0x600a_9804,
            AccessWidth::Word,
            (1 << 23) | (1 << 24),
            SimTime::ZERO,
        )
        .unwrap();
    for (address, value) in [
        (0x600a_3048, 3_u64),
        (0x600a_3004, 1 << 3),
        (0x600a_30d0, u64::from(tx_address)),
        (0x600a_30e0, u64::from(rx_address)),
    ] {
        machine
            .bus
            .write(address, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    for (offset, byte) in [5_u8, 0x21, 0x00, 0x2a, 0, 0].into_iter().enumerate() {
        machine
            .bus
            .write(
                u64::from(tx_address) + offset as u64,
                AccessWidth::Byte,
                u64::from(byte),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x41, SimTime::ZERO)
        .unwrap();
    machine.service_radio().unwrap();
    machine.now = SimTime::from_ticks(160);
    machine.service_radio().unwrap();
    assert_eq!(
        machine
            .esp32c6_peripherals
            .as_ref()
            .unwrap()
            .ieee802154
            .awaiting_ack_sequence(),
        Some(0x2a)
    );

    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Ieee802154,
            remu_radio::Spectrum::new(2_405_000, 2_000),
            "ieee802154-oqpsk-250k",
            remu_radio::Ieee802154Mac::with_fcs(vec![0x02, 0x00, 0x2a]),
            0,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(320);
    machine.service_radio().unwrap();
    assert_eq!(
        machine
            .esp32c6_peripherals
            .as_ref()
            .unwrap()
            .ieee802154
            .awaiting_ack_sequence(),
        None
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_3064,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & (1 << 3),
        1 << 3
    );
    assert_eq!(
        machine.debug_read_memory(u64::from(rx_address), 6).unwrap(),
        [5, 0x02, 0x00, 0x2a, (-40_i8) as u8, 191]
    );
}

#[test]
fn esp32c6_ieee802154_dma_security_applies_vendor_programmed_ccm_star() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let tx_address = 0x4080_0300_u32;
    machine
        .bus
        .write(
            0x600a_9804,
            AccessWidth::Word,
            (1 << 23) | (1 << 24) | (1 << 27),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_3048, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_30d0,
            AccessWidth::Word,
            u64::from(tx_address),
            SimTime::ZERO,
        )
        .unwrap();

    let fcf = 0x9849_u16;
    let mut frame = Vec::from(fcf.to_le_bytes());
    frame.push(0x2a);
    frame.extend_from_slice(&0x1234_u16.to_le_bytes());
    frame.extend_from_slice(&0x5678_u16.to_le_bytes());
    frame.extend_from_slice(&0x9abc_u16.to_le_bytes());
    frame.push(5);
    frame.extend_from_slice(&7_u32.to_le_bytes());
    let payload_offset = frame.len();
    frame.extend_from_slice(b"secured");
    frame.extend_from_slice(&[0; 4]);
    let mut wire_frame = frame.clone();
    wire_frame.extend_from_slice(&[0; 2]);
    machine
        .bus
        .write(
            0x600a_3128,
            AccessWidth::Word,
            1 | ((payload_offset as u64) << 8),
            SimTime::ZERO,
        )
        .unwrap();
    for (word, value) in [
        0x0403_0201_u32,
        0x0807_0605,
        0x1111_1111,
        0x1111_1111,
        0x1111_1111,
        0x1111_1111,
    ]
    .into_iter()
    .enumerate()
    {
        machine
            .bus
            .write(
                0x600a_312c + word as u64 * 4,
                AccessWidth::Word,
                u64::from(value),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(
            u64::from(tx_address),
            AccessWidth::Byte,
            wire_frame.len() as u64,
            SimTime::ZERO,
        )
        .unwrap();
    for (offset, byte) in wire_frame.iter().copied().enumerate() {
        machine
            .bus
            .write(
                u64::from(tx_address) + 1 + offset as u64,
                AccessWidth::Byte,
                u64::from(byte),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x41, SimTime::ZERO)
        .unwrap();
    machine.service_radio().unwrap();

    let replay = machine.radio_replay_artifact().unwrap();
    let protected = replay
        .events
        .iter()
        .find_map(|event| match event {
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.origin == remu_radio::FrameOrigin::Emulated =>
            {
                Some(&request.frame.bytes)
            }
            _ => None,
        })
        .expect("secured frame submitted");
    assert_eq!(&protected[..payload_offset], &frame[..payload_offset]);
    assert_ne!(&protected[payload_offset..payload_offset + 7], b"secured");
    assert_eq!(protected.len(), wire_frame.len());
    assert!(remu_radio::Ieee802154Mac::has_valid_fcs(protected));
}

#[test]
fn esp32c6_ieee802154_security_failures_preserve_vendor_reason_codes() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let tx_address = 0x4080_0380_u32;
    machine
        .bus
        .write(
            0x600a_9804,
            AccessWidth::Word,
            (1 << 23) | (1 << 24) | (1 << 27),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_3048, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_30d0,
            AccessWidth::Word,
            u64::from(tx_address),
            SimTime::ZERO,
        )
        .unwrap();

    let mut assert_failure = |frame: &[u8], hardware_offset: u8, reason: u8, count: u32| {
        let wire_length = frame.len() + 2;
        machine
            .bus
            .write(
                0x600a_3128,
                AccessWidth::Word,
                1 | (u64::from(hardware_offset) << 8),
                machine.now,
            )
            .unwrap();
        machine
            .bus
            .write(
                u64::from(tx_address),
                AccessWidth::Byte,
                wire_length as u64,
                machine.now,
            )
            .unwrap();
        for (offset, byte) in frame.iter().copied().enumerate() {
            machine
                .bus
                .write(
                    u64::from(tx_address) + 1 + offset as u64,
                    AccessWidth::Byte,
                    u64::from(byte),
                    machine.now,
                )
                .unwrap();
        }
        for offset in frame.len()..wire_length {
            machine
                .bus
                .write(
                    u64::from(tx_address) + 1 + offset as u64,
                    AccessWidth::Byte,
                    0,
                    machine.now,
                )
                .unwrap();
        }
        machine
            .bus
            .write(0x600a_3000, AccessWidth::Word, 0x41, machine.now)
            .unwrap();
        machine.service_radio().unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    0x600a_3084,
                    AccessWidth::Word,
                    AccessKind::Read,
                    machine.now,
                )
                .unwrap(),
            (19 << 4) | (u64::from(reason) << 16)
        );
        assert_eq!(
            machine
                .bus
                .read(
                    0x600a_3178,
                    AccessWidth::Word,
                    AccessKind::Read,
                    machine.now,
                )
                .unwrap(),
            u64::from(count)
        );
        machine
            .bus
            .write(0x600a_3064, AccessWidth::Word, 1 << 5, machine.now)
            .unwrap();
    };

    // Security enable register set, but FCF security bit clear.
    assert_failure(&[0x01, 0x00, 1, 0xaa], 5, 1, 1);
    // Security level zero is reserved for a hardware-protected transmit.
    assert_failure(&[0x09, 0x00, 1, 0, 1, 0, 0, 0, 0xaa], 8, 2, 2);
    // Reserved address modes fail while parsing the secured MAC header.
    assert_failure(&[0x08, 0x04, 1, 5, 1, 0, 0, 0, 0xaa], 9, 3, 3);
    // A payload offset before the complete auxiliary header is invalid.
    assert_failure(&[0x09, 0x00, 1, 5, 1, 0, 0, 0, 0xaa], 4, 4, 4);
    // C6 transmit security requires the auxiliary frame counter.
    assert_failure(&[0x09, 0x00, 1, 0x25, 0xaa], 5, 5, 5);
}

#[test]
fn esp32c6_ieee802154_cca_reports_busy_and_leaves_csma_retry_to_firmware() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let tx_address = 0x4080_0400_u32;
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
        .write(0x600a_3050, AccessWidth::Word, 8, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_3054,
            AccessWidth::Word,
            0xb5 | (1 << 14),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            0x600a_30d0,
            AccessWidth::Word,
            u64::from(tx_address),
            SimTime::ZERO,
        )
        .unwrap();
    for (offset, byte) in [5_u8, 0x01, 0x00, 0x2a, 0, 0].into_iter().enumerate() {
        machine
            .bus
            .write(
                u64::from(tx_address) + offset as u64,
                AccessWidth::Byte,
                u64::from(byte),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Ieee802154,
            remu_radio::Spectrum::new(2_405_000, 2_000),
            "ieee802154-oqpsk-250k",
            vec![0; 100],
            0,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x43, SimTime::ZERO)
        .unwrap();

    machine.service_radio().unwrap();
    assert_eq!(
        machine.radio_pending_ieee802154_cca,
        Some(SimTime::from_ticks(128))
    );
    machine.now = SimTime::from_ticks(128);
    machine.service_radio().unwrap();
    assert!(machine.radio_pending_ieee802154_cca.is_none());
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_3084,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        25 << 4
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_317c,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        1
    );
    assert!(
        !machine
            .radio_replay_artifact()
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(
                event,
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.origin == remu_radio::FrameOrigin::Emulated
            ))
    );

    // CSMA policy lives in guest firmware: retry only after the interfering
    // frame has ended, then the same one-shot peripheral command succeeds.
    machine
        .bus
        .write(0x600a_3064, AccessWidth::Word, 1 << 5, machine.now)
        .unwrap();
    machine.now = SimTime::from_ticks(4000);
    machine.service_radio().unwrap();
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x43, machine.now)
        .unwrap();
    machine.service_radio().unwrap();
    assert_eq!(
        machine.radio_pending_ieee802154_cca,
        Some(SimTime::from_ticks(4128))
    );
    machine.now = SimTime::from_ticks(4128);
    machine.service_radio().unwrap();
    assert!(
        machine
            .radio_replay_artifact()
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(
                event,
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.origin == remu_radio::FrameOrigin::Emulated
            ))
    );
}

#[test]
fn esp32c6_wifi_and_ble_protocol_engines_follow_modem_clock_gates() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    assert!(matches!(
        machine.wifi_engine(),
        Err(MachineError::RadioNotReady("Wi-Fi"))
    ));
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 9) | (1 << 10) | (1 << 17) | (1 << 18),
            SimTime::ZERO,
        )
        .unwrap();
    let mut wifi_frame = vec![0_u8; 24];
    wifi_frame[4..10].fill(0xff);
    machine
        .wifi_engine()
        .unwrap()
        .start(remu_radio::WifiMode::Station)
        .unwrap();
    machine.wifi_engine().unwrap().queue_tx(wifi_frame).unwrap();
    machine
        .ble_controller()
        .unwrap()
        .process_h4(&[1, 3, 12, 0])
        .unwrap();
    assert_eq!(
        machine.ble_controller().unwrap().take_h4_output(),
        Some(vec![4, 0x0e, 4, 1, 3, 12, 0])
    );
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert!(
        machine
            .radio_replay_artifact()
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(
                event,
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.protocol == remu_radio::RadioProtocol::Wifi
            ))
    );
}

#[test]
fn esp32c6_coexistence_preempts_airtime_and_denies_lower_priority_work() {
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

    assert_eq!(machine.service_radio().unwrap(), 2);
    let artifact = machine.radio_replay_artifact().unwrap();
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
    let artifact = machine.radio_replay_artifact().unwrap();
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
fn esp32c6_modem_reset_cancels_active_coexistence_ownership() {
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
        .write(0x600a_f024, AccessWidth::Word, 1 << 1, SimTime::ZERO)
        .unwrap();

    machine.service_radio().unwrap();

    let arbiter = machine.radio_coexistence.as_ref().unwrap();
    assert_eq!(arbiter.owner(), None);
    assert!(matches!(
        arbiter.events().last(),
        Some(remu_radio::CoexistenceEvent::Reset { at }) if *at == SimTime::ZERO
    ));
    assert!(matches!(
        machine.radio_replay_artifact().unwrap().events.last(),
        Some(remu_radio::MediumEvent::Truncated { at, .. }) if *at == SimTime::ZERO
    ));
}

#[test]
fn esp32c6_radio_power_gate_truncates_active_airtime() {
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
        .write(0x600a_9814, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();

    machine.service_radio().unwrap();

    let arbiter = machine.radio_coexistence.as_ref().unwrap();
    assert_eq!(arbiter.owner(), None);
    assert!(matches!(
        arbiter.events().last(),
        Some(remu_radio::CoexistenceEvent::PowerDown {
            protocol: remu_radio::RadioProtocol::Wifi,
            at,
            ..
        }) if *at == SimTime::ZERO
    ));
    assert!(matches!(
        machine.radio_replay_artifact().unwrap().events.last(),
        Some(remu_radio::MediumEvent::Truncated { at, .. }) if *at == SimTime::ZERO
    ));
}

#[test]
fn esp32c6_power_gated_unmapped_grant_is_a_hard_machine_error() {
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
    machine.service_radio().unwrap();
    machine
        .radio_coexistence
        .as_mut()
        .unwrap()
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
        .write(0x600a_9814, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();

    let MachineError::RadioLegality(error) = machine.service_radio().unwrap_err() else {
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
