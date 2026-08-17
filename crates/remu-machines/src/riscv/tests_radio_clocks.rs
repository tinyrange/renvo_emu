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
