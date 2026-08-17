#[test]
fn esp32c6_wifi_airtime_uses_causal_rf_channel_and_power() {
    for (channel, center_khz) in [(1, 2_412_000), (6, 2_437_000), (11, 2_462_000)] {
        for (power_qdbm, power_dbm) in [(32, 8), (56, 14), (80, 20)] {
            let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
            program_esp32c6_wifi_rf(&mut machine, channel, power_qdbm);
            let (spectrum, observed_power) = machine.c6_wifi_rf_airtime().unwrap();
            assert_eq!(spectrum, remu_radio::Spectrum::new(center_khz, 20_000));
            assert_eq!(observed_power, power_dbm);
        }
    }
}

#[test]
fn esp32c6_wifi_airtime_rejects_incomplete_stale_and_forced_rf_state() {
    let mut missing = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let MachineError::RadioLegality(error) = missing.c6_wifi_rf_airtime().unwrap_err() else {
        panic!("missing RF state should be a hard legality error");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::RfPllLock);

    let mut incomplete = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    program_esp32c6_wifi_rf(&mut incomplete, 6, 56);
    for (address, value) in [
        (0x600a_08cc, 0),
        (0x600a_08d0, 0),
        (0x600a_08d4, 0xfe),
    ] {
        incomplete
            .bus
            .write(address, AccessWidth::Word, value, incomplete.now)
            .unwrap();
    }
    let MachineError::RadioLegality(error) = incomplete.c6_wifi_rf_airtime().unwrap_err() else {
        panic!("incomplete replacement gain table should be a hard legality error");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::RfCalibration);

    let mut unsupported_power = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    program_esp32c6_wifi_rf(&mut unsupported_power, 6, 34);
    let MachineError::RadioLegality(error) =
        unsupported_power.c6_wifi_rf_airtime().unwrap_err()
    else {
        panic!("unsupported fractional power should be a hard legality error");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::RfPower);

    let mut forced_off = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    program_esp32c6_wifi_rf(&mut forced_off, 11, 80);
    forced_off
        .bus
        .write(
            0x600a_0910,
            AccessWidth::Word,
            0x200,
            forced_off.now,
        )
        .unwrap();
    let MachineError::RadioLegality(error) = forced_off.c6_wifi_rf_airtime().unwrap_err() else {
        panic!("forced-off frontend should be a hard legality error");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::RfFrontend);

    let mut stale = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    program_esp32c6_wifi_rf(&mut stale, 1, 32);
    stale
        .esp32c6_peripherals
        .as_ref()
        .unwrap()
        .wifi_rf
        .invalidate_wifi_rf();
    let MachineError::RadioLegality(error) = stale.c6_wifi_rf_airtime().unwrap_err() else {
        panic!("reset-invalidated RF state should be a hard legality error");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::RfPllLock);
}

#[test]
fn esp32c6_wifi_receive_tuning_follows_guest_rf_state() {
    let frame = vec![0x80; 16];
    let duration = SimTime::from_ticks(frame.len() as u64 * 32);

    let mut off_channel = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    program_esp32c6_wifi_rf(&mut off_channel, 6, 56);
    off_channel
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            frame.clone(),
            -30,
        )
        .unwrap();
    off_channel
        .radio_medium
        .as_mut()
        .unwrap()
        .advance_to(duration)
        .unwrap();
    assert!(!off_channel.radio_medium.as_ref().unwrap().events().iter().any(
        |event| matches!(event, remu_radio::MediumEvent::Reception { receiver, .. } if *receiver == remu_radio::NodeId(1))
    ));

    let mut on_channel = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    program_esp32c6_wifi_rf(&mut on_channel, 6, 56);
    on_channel
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_437_000, 20_000),
            "wifi-ht20",
            frame,
            -30,
        )
        .unwrap();
    on_channel
        .radio_medium
        .as_mut()
        .unwrap()
        .advance_to(duration)
        .unwrap();
    assert!(on_channel.radio_medium.as_ref().unwrap().events().iter().any(
        |event| matches!(event, remu_radio::MediumEvent::Reception { receiver, outcome: remu_radio::DeliveryOutcome::Delivered, .. } if *receiver == remu_radio::NodeId(1))
    ));

    let mut disabled = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    program_esp32c6_wifi_rf(&mut disabled, 1, 56);
    disabled
        .bus
        .write(0x600a_0910, AccessWidth::Word, 0x200, disabled.now)
        .unwrap();
    let MachineError::RadioLegality(error) = disabled
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            vec![0x80; 16],
            -30,
        )
        .unwrap_err()
    else {
        panic!("RX while the frontend is forced off should be a hard legality error");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::RfFrontend);
}

#[test]
fn esp32c6_illegal_wifi_crypto_slot_is_a_hard_firmware_state_error() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_5804,
            AccessWidth::Word,
            1 << 21,
            machine.now,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_4814, AccessWidth::Word, 1, machine.now)
        .unwrap();

    let MachineError::RadioLegality(error) = machine.service_radio().unwrap_err() else {
        panic!("invalid native crypto entry should be a hard legality error");
    };
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Wifi);
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::SchedulerState);
    assert!(error.detail.contains("impossible control class 1"));
}

#[test]
fn esp32c6_illegal_tsf_timer_order_is_a_hard_firmware_state_error() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_d074,
            AccessWidth::Word,
            1_u64 << 31,
            machine.now,
        )
        .unwrap();

    let MachineError::RadioLegality(error) = machine.service_radio().unwrap_err() else {
        panic!("invalid native TSF timer order should be a hard legality error");
    };
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Wifi);
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::SchedulerState);
    assert!(
        error
            .detail
            .contains("enabled before its firmware interrupt bit")
    );
}

#[test]
fn esp32c6_tsf_timer_reaches_firmware_through_native_power_interrupt_status() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(0x6001_0000, AccessWidth::Word, 5, machine.now)
        .unwrap();
    machine
        .bus
        .write(0x6001_0008, AccessWidth::Word, 6, machine.now)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 9) | (1 << 10),
            machine.now,
        )
        .unwrap();
    for (address, value) in [
        (0x600a_d078, 2_u64),
        (0x600a_d0b4, 0x80),
        (0x600a_d0a8, 0x80),
        (0x600a_d074, (1_u64 << 31) | (1 << 30) | 3),
    ] {
        machine
            .bus
            .write(address, AccessWidth::Word, value, machine.now)
            .unwrap();
    }
    machine.now = SimTime::from_ticks(32);

    assert_eq!(machine.service_radio().unwrap(), 1);
    let interrupt_matrix = &machine.esp32c6_peripherals.as_ref().unwrap().interrupt_matrix;
    assert!(!interrupt_matrix.cpu_interrupt_pending(5));
    assert!(interrupt_matrix.cpu_interrupt_pending(6));
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_d0ac,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        0x80
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_d0b0,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        0x80
    );
    machine
        .bus
        .write(0x600a_d0b4, AccessWidth::Word, 0x80, machine.now)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_d0b0,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        0
    );
    assert_eq!(machine.service_radio().unwrap(), 0);
}

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
fn esp32c6_malformed_secured_rx_is_dma_delivered_auto_acked_and_replayable() {
    fn run() -> Vec<u8> {
        let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
        let rx_address = 0x4080_0240_u32;
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
            // Promiscuous receive plus hardware automatic ACK transmission.
            (0x600a_3004, (1 << 7) | 1),
            (0x600a_30e0, u64::from(rx_address)),
        ] {
            machine
                .bus
                .write(address, AccessWidth::Word, value, SimTime::ZERO)
                .unwrap();
        }
        machine
            .bus
            .write(0x600a_3000, AccessWidth::Word, 0x42, SimTime::ZERO)
            .unwrap();
        assert_eq!(machine.service_radio().unwrap(), 1);

        let mut frame = Vec::from(0x9869_u16.to_le_bytes());
        frame.push(0x2a);
        frame.extend_from_slice(&0x1234_u16.to_le_bytes());
        frame.extend_from_slice(&0x5678_u16.to_le_bytes());
        frame.extend_from_slice(&0x9abc_u16.to_le_bytes());
        // ENC-MIC-32 security control with the required frame counter and MIC
        // deliberately absent. This is hostile over-air data, not an illegal
        // peripheral configuration.
        frame.push(5);
        let frame = remu_radio::Ieee802154Mac::with_fcs(frame);
        machine
            .inject_radio_frame(
                remu_radio::RadioProtocol::Ieee802154,
                remu_radio::Spectrum::new(2_405_000, 2_000),
                "ieee802154-oqpsk-250k",
                frame.clone(),
                -40,
            )
            .unwrap();
        machine.now = SimTime::from_ticks(frame.len() as u64 * 32);
        assert_eq!(machine.service_radio().unwrap(), 1);

        let delivered = &frame[..frame.len() - 2];
        assert_eq!(
            machine
                .debug_read_memory(u64::from(rx_address), delivered.len() + 3)
                .unwrap(),
            [
                vec![(delivered.len() + 2) as u8],
                delivered.to_vec(),
                // The default shared-medium path loss turns the injected
                // -40 dBm transmit power into a deterministic -80 dBm receive
                // sample, which the native MAC maps to LQI 63.
                vec![(-80_i8) as u8, 63],
            ]
            .concat()
        );
        let replay = machine.radio_replay_artifact().unwrap();
        let ack = replay
            .events
            .iter()
            .find_map(|event| match event {
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.origin == remu_radio::FrameOrigin::Emulated
                        && request.frame.protocol == remu_radio::RadioProtocol::Ieee802154 =>
                {
                    Some(request.frame.bytes.as_slice())
                }
                _ => None,
            })
            .expect("native MAC emitted an ACK");
        assert_eq!(&ack[..3], &[0x02, 0, 0x2a]);
        assert!(remu_radio::Ieee802154Mac::has_valid_fcs(ack));
        assert!(replay.coexistence_events.iter().any(|event| matches!(
            event,
            remu_radio::CoexistenceEvent::Granted {
                protocol: remu_radio::RadioProtocol::Ieee802154,
                ..
            }
        )));
        let ack_airtime = remu_core::SimDuration::from_ticks(ack.len() as u64 * 32);
        machine.now += ack_airtime;
        assert_eq!(machine.service_radio().unwrap(), 1);
        assert!(machine.radio_pending_ieee802154_ack.is_empty());
        machine.radio_replay_artifact().unwrap().to_json().unwrap()
    }

    assert_eq!(run(), run());
}

#[test]
fn esp32c6_clock_gate_during_hardware_auto_ack_is_a_hard_machine_error() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let rx_address = 0x4080_02e0_u32;
    for (address, value) in [
        (0x600a_9804, (1 << 23) | (1 << 24)),
        (0x600a_3048, 3),
        (0x600a_3004, (1 << 7) | 1),
        (0x600a_30e0, u64::from(rx_address)),
        (0x600a_3000, 0x42),
    ] {
        machine
            .bus
            .write(address, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    assert_eq!(machine.service_radio().unwrap(), 1);

    let mut frame = Vec::from(0x9869_u16.to_le_bytes());
    frame.push(0x2a);
    frame.extend_from_slice(&0x1234_u16.to_le_bytes());
    frame.extend_from_slice(&0x5678_u16.to_le_bytes());
    frame.extend_from_slice(&0x9abc_u16.to_le_bytes());
    frame.push(5);
    let frame = remu_radio::Ieee802154Mac::with_fcs(frame);
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Ieee802154,
            remu_radio::Spectrum::new(2_405_000, 2_000),
            "ieee802154-oqpsk-250k",
            frame.clone(),
            -40,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(frame.len() as u64 * 32);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(machine.radio_pending_ieee802154_ack.len(), 1);

    machine
        .bus
        .write(0x600a_9804, AccessWidth::Word, 0, machine.now)
        .unwrap();
    let MachineError::RadioLegality(error) = machine.service_radio().unwrap_err() else {
        panic!("expected a radio legality error");
    };
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Ieee802154);
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::DomainReady);
    assert!(error.detail.contains("Transmit"));
    let replay = machine.radio_replay_artifact().unwrap();
    assert!(matches!(
        replay.events.last(),
        Some(remu_radio::MediumEvent::Truncated { at, .. }) if *at == machine.now
    ));
    assert!(matches!(
        replay.coexistence_events.last(),
        Some(remu_radio::CoexistenceEvent::PowerDown {
            protocol: remu_radio::RadioProtocol::Ieee802154,
            at,
            ..
        }) if *at == machine.now
    ));
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
fn esp32c6_ieee802154_stop_clock_gate_wake_and_rearm_replays_exactly() {
    fn run() -> Vec<u8> {
        let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
        let rx_address = 0x4080_02c0_u32;
        for (address, value) in [
            (0x600a_9804, (1 << 23) | (1 << 24)),
            (0x600a_3048, 3),
            (0x600a_3004, 1 << 7),
            (0x600a_30e0, u64::from(rx_address)),
            (0x600a_3000, 0x42),
        ] {
            machine
                .bus
                .write(address, AccessWidth::Word, value, machine.now)
                .unwrap();
        }
        assert_eq!(machine.service_radio().unwrap(), 1);

        // The public raw-link sleep sequence stops the MAC before firmware
        // gates its APB/MAC clocks.
        machine
            .bus
            .write(0x600a_3000, AccessWidth::Word, 0x45, machine.now)
            .unwrap();
        assert_eq!(machine.service_radio().unwrap(), 1);
        machine
            .bus
            .write(0x600a_9804, AccessWidth::Word, 0, machine.now)
            .unwrap();
        assert_eq!(machine.service_radio().unwrap(), 0);

        let sleeping_frame = remu_radio::Ieee802154Mac::with_fcs(vec![0x01, 0, 0x31]);
        machine
            .inject_radio_frame(
                remu_radio::RadioProtocol::Ieee802154,
                remu_radio::Spectrum::new(2_405_000, 2_000),
                "ieee802154-oqpsk-250k",
                sleeping_frame.clone(),
                -40,
            )
            .unwrap();
        machine.now += remu_core::SimDuration::from_ticks(sleeping_frame.len() as u64 * 32);
        assert_eq!(machine.service_radio().unwrap(), 0);
        assert_eq!(
            machine.debug_read_memory(u64::from(rx_address), 6).unwrap(),
            [0xa5; 6]
        );

        machine
            .bus
            .write(
                0x600a_9804,
                AccessWidth::Word,
                (1 << 23) | (1 << 24),
                machine.now,
            )
            .unwrap();
        assert_eq!(machine.service_radio().unwrap(), 0);
        machine
            .bus
            .write(0x600a_3000, AccessWidth::Word, 0x42, machine.now)
            .unwrap();
        assert_eq!(machine.service_radio().unwrap(), 1);

        let wake_frame = remu_radio::Ieee802154Mac::with_fcs(vec![0x01, 0, 0x32]);
        machine
            .inject_radio_frame(
                remu_radio::RadioProtocol::Ieee802154,
                remu_radio::Spectrum::new(2_405_000, 2_000),
                "ieee802154-oqpsk-250k",
                wake_frame.clone(),
                -40,
            )
            .unwrap();
        machine.now += remu_core::SimDuration::from_ticks(wake_frame.len() as u64 * 32);
        assert_eq!(machine.service_radio().unwrap(), 1);
        assert_eq!(
            machine.debug_read_memory(u64::from(rx_address), 7).unwrap(),
            [5, 0x01, 0, 0x32, (-80_i8) as u8, 63, 0xa5]
        );
        machine.radio_replay_artifact().unwrap().to_json().unwrap()
    }

    assert_eq!(run(), run());
}

#[test]
fn esp32c6_ieee802154_clock_gate_during_receive_is_a_hard_machine_error() {
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
        .write(0x600a_3000, AccessWidth::Word, 0x42, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);
    machine
        .bus
        .write(0x600a_9804, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();

    let MachineError::RadioLegality(error) = machine.service_radio().unwrap_err() else {
        panic!("expected a radio legality error");
    };
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Ieee802154);
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::DomainReady);
    assert!(error.detail.contains("Receive"));
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
