#[test]
fn wifi_mac_reset_command_sets_ready_status() {
    let mut mac = EspC6WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    mac.write(
        C6_WIFI_MAC_RX_BASE,
        AccessWidth::Word,
        0x4080_1000,
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH,
        AccessWidth::Word,
        u64::from(C6_WIFI_MAC_TX_QUEUE_ENABLE | 0x1000),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.tx_active(0));
    assert!(handle.rx_descriptor().is_some());
    let generation = handle.reset_generation();
    mac.write(
        C6_WIFI_MAC_RESET_CONTROL,
        AccessWidth::Word,
        C6_WIFI_MAC_RESET_START as u64,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        mac.read(C6_WIFI_MAC_RESET_CONTROL, AccessWidth::Word, SimTime::ZERO,)
            .unwrap()
            & u64::from(C6_WIFI_MAC_RESET_READY),
        u64::from(C6_WIFI_MAC_RESET_READY)
    );
    assert_eq!(handle.reset_generation(), generation + 1);
    assert!(!handle.tx_active(0));
    assert!(handle.take_tx_descriptor().is_none());
    assert!(handle.rx_descriptor().is_none());
}

#[test]
fn wifi_mac_rx_descriptor_reload_command_self_clears() {
    let mut mac = EspC6WifiMacRegisters::new("wifi-mac");
    let configuration = 0x8803_0000_u32;
    mac.write(
        C6_WIFI_MAC_RX_CONTROL,
        AccessWidth::Word,
        u64::from(configuration | C6_WIFI_MAC_RX_DESCRIPTOR_RELOAD),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        mac.read(C6_WIFI_MAC_RX_CONTROL, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        u64::from(configuration)
    );
}

#[test]
fn wifi_mac_rx_match_uses_firmware_programmed_interface_address() {
    let mut mac = EspC6WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    assert_eq!(handle.rx_match_mask(&[0xff; 6]), 1);

    mac.write(
        C6_WIFI_MAC_INTERFACE_ADDRESS_LOW + C6_WIFI_MAC_INTERFACE_ADDRESS_STRIDE,
        AccessWidth::Word,
        0x0002_4552,
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        C6_WIFI_MAC_INTERFACE_ADDRESS_HIGH + C6_WIFI_MAC_INTERFACE_ADDRESS_STRIDE,
        AccessWidth::Word,
        u64::from(C6_WIFI_MAC_INTERFACE_ADDRESS_VALID | 0x0100),
        SimTime::ZERO,
    )
    .unwrap();

    assert_eq!(
        handle.rx_match_mask(&[0x52, 0x45, 0x02, 0x00, 0x00, 0x01]),
        1 << 1
    );
    assert_eq!(
        handle.rx_match_mask(&[0x52, 0x45, 0x02, 0x00, 0x00, 0x02]),
        0
    );
    assert_eq!(handle.rx_match_mask(&[0xff; 6]), 1 << 1);
}

#[test]
fn wifi_mac_rx_block_ack_tracks_the_native_firmware_window() {
    let mut mac = EspC6WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    let peer = [0x02, 1, 2, 3, 4, 5];
    mac.write(
        C6_WIFI_MAC_RX_BA_MAC_LOW_HIGH,
        AccessWidth::Word,
        u64::from(u32::from_le_bytes(peer[..4].try_into().unwrap())),
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        C6_WIFI_MAC_RX_BA_MAC_HIGH_HIGH,
        AccessWidth::Word,
        u64::from(u16::from_le_bytes(peer[4..].try_into().unwrap())),
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        C6_WIFI_MAC_RX_BA_SEQUENCE_HIGH,
        AccessWidth::Word,
        0x0ffe,
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        C6_WIFI_MAC_RX_BA_CONTROL_HIGH,
        AccessWidth::Word,
        u64::from((3_u32 << 30) | (3 << 12) | 5),
        SimTime::ZERO,
    )
    .unwrap();

    assert!(handle.record_block_ack_mpdu(&peer, 3, 0x0fff));
    assert!(handle.record_block_ack_mpdu(&peer, 3, 0x0000));
    assert_eq!(handle.block_ack_bitmap(&peer, 3, 0x0fff), Some(3));
    assert!(!handle.record_block_ack_mpdu(&peer, 2, 0));

    mac.reset(ResetKind::PowerOn);
    assert_eq!(handle.block_ack_bitmap(&peer, 3, 0x0fff), None);
    mac.write(
        C6_WIFI_MAC_RX_BA_CONTROL_HIGH,
        AccessWidth::Word,
        u64::from(C6_WIFI_MAC_RX_BA_VALID | 5),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.validate_block_ack_sessions().is_err());
}

#[test]
fn wifi_mac_tx_completion_drives_native_event_and_queue_state() {
    let mut mac = EspC6WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    mac.write(
        C6_WIFI_MAC_INTERRUPT_MASK,
        AccessWidth::Word,
        u64::from(C6_WIFI_MAC_EVENT_TX_DONE),
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        C6_WIFI_MAC_TX_QUEUE_PROTECTION_HIGH,
        AccessWidth::Word,
        u64::from(C6_WIFI_MAC_TX_QUEUE_RTS_ENABLED),
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH,
        AccessWidth::Word,
        u64::from(C6_WIFI_MAC_TX_QUEUE_ENABLE | 0x1234),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        handle.take_tx_descriptor(),
        Some(EspC6WifiTxDescriptor {
            queue: 0,
            address: 0x4080_1234,
        })
    );
    assert!(handle.tx_rts_enabled(0));
    assert!(!handle.tx_rts_enabled(1));
    assert!(!handle.interrupt_pending());
    assert_eq!(
        mac.read(C6_WIFI_MAC_TX_QUEUE_STATE, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0
    );
    assert!(handle.tx_active(0));
    assert!(handle.complete_tx(0, crate::EspWifiTxOutcome::AckTimeout));
    assert!(handle.interrupt_pending());
    assert_eq!(
        mac.read(C6_WIFI_MAC_TX_QUEUE_STATE, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        1
    );
    assert_eq!(
        mac.read(
            C6_WIFI_MAC_TX_QUEUE_COMPLETION_HIGH,
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap() as u32
            & C6_WIFI_MAC_TX_QUEUE_COMPLETION_STATUS,
        5 << 12
    );
    assert_eq!(
        mac.read(
            C6_WIFI_MAC_TX_QUEUE_COMPLETION_COUNT_HIGH,
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap() as u32
            & C6_WIFI_MAC_TX_QUEUE_COMPLETION_COUNT,
        0
    );
    assert_eq!(
        mac.read(
            C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH,
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap() as u32
            & C6_WIFI_MAC_TX_QUEUE_ENABLE,
        1 << 30
    );
    mac.write(
        C6_WIFI_MAC_INTERRUPT_CLEAR,
        AccessWidth::Word,
        u64::from(C6_WIFI_MAC_EVENT_TX_DONE),
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        C6_WIFI_MAC_TX_QUEUE_STATE_CLEAR,
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert!(!handle.interrupt_pending());
    assert_eq!(
        mac.read(C6_WIFI_MAC_TX_QUEUE_STATE, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0
    );
}

#[test]
fn wifi_mac_tx_completion_publishes_native_block_ack_record() {
    let mut mac = EspC6WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    mac.write(
        C6_WIFI_MAC_TX_QUEUE_CONTROL_HIGH,
        AccessWidth::Word,
        u64::from(C6_WIFI_MAC_TX_QUEUE_ENABLE | 0x1234),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.take_tx_descriptor().is_some());
    assert!(handle.complete_tx_record(
        0,
        crate::EspWifiTxOutcome::Success,
        2,
        Some(crate::EspWifiTxBlockAck {
            status: 3,
            starting_sequence: 0xffe,
            bitmap: 0x8000_0000_0000_0005,
        }),
    ));
    assert_eq!(
        mac.read(C6_WIFI_MAC_TX_QUEUE_COMPLETION_COUNT_HIGH, AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32
            & C6_WIFI_MAC_TX_QUEUE_COMPLETION_COUNT,
        2 << 16
    );
    assert_eq!(
        mac.read(C6_WIFI_MAC_TX_QUEUE_BA_STATUS_HIGH, AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32
            & 0x000f_0fff,
        (3 << 16) | 0xffe
    );
    assert_eq!(
        mac.read(C6_WIFI_MAC_TX_QUEUE_BA_BITMAP_LOW_HIGH, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        5
    );
    assert_eq!(
        mac.read(C6_WIFI_MAC_TX_QUEUE_BA_BITMAP_HIGH_HIGH, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0x8000_0000
    );
}

#[test]
fn wifi_rx_base_advances_native_ring_and_asserts_event() {
    let mut mac = EspC6WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    mac.write(
        C6_WIFI_MAC_INTERRUPT_MASK,
        AccessWidth::Word,
        u64::from(C6_WIFI_MAC_EVENT_RX_DONE),
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        C6_WIFI_MAC_RX_BASE,
        AccessWidth::Word,
        0x4082_1000,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        handle.rx_descriptor(),
        Some(EspC6WifiRxDescriptor {
            address: 0x4082_1000
        })
    );
    handle.complete_rx_descriptor(0x4082_1000, 0x4082_100c);
    assert_eq!(
        handle.rx_descriptor(),
        Some(EspC6WifiRxDescriptor {
            address: 0x4082_100c
        })
    );
    assert_eq!(
        mac.read(C6_WIFI_MAC_RX_NEXT, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0x4082_100c
    );
    assert_eq!(
        mac.read(C6_WIFI_MAC_RX_LAST, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0x0002_1000
    );
    assert!(handle.interrupt_pending());
    mac.write(
        C6_WIFI_MAC_INTERRUPT_CLEAR,
        AccessWidth::Word,
        u64::from(C6_WIFI_MAC_EVENT_RX_DONE),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(!handle.interrupt_pending());
}

#[test]
fn modem_reset_strobes_increment_domain_generations() {
    let (mut syscon, mut lpcon, handle) = EspC6ModemControl::new_pair("syscon", "lpcon");
    let initial = handle.reset_generations();
    syscon
        .write(
            0x10,
            AccessWidth::Word,
            (1 << 10) | (1 << 16) | (1 << 24),
            SimTime::ZERO,
        )
        .unwrap();
    lpcon
        .write(0x24, AccessWidth::Word, 1 << 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        handle.reset_generations(),
        [
            initial[0] + 1,
            initial[1] + 1,
            initial[2] + 1,
            initial[3] + 1
        ]
    );
    let after_rising_edges = handle.reset_generations();
    syscon
        .write(
            0x10,
            AccessWidth::Word,
            (1 << 10) | (1 << 16) | (1 << 24),
            SimTime::ZERO,
        )
        .unwrap();
    lpcon
        .write(0x24, AccessWidth::Word, 1 << 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        handle.reset_generations(),
        [
            after_rising_edges[0],
            after_rising_edges[1],
            after_rising_edges[2],
            after_rising_edges[3] + 1,
        ]
    );
}

#[test]
fn ieee802154_command_completion_and_w1c_interrupt_work() {
    let (mut device, handle) = EspIeee802154::new("ieee802154");
    device
        .write(0x60, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    device
        .write(0x00, AccessWidth::Word, 0x41, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.take_command(), Some(EspIeee802154Command::TxStart));
    assert!(!handle.interrupt_pending());
    handle.complete_tx();
    assert!(handle.interrupt_pending());
    assert_eq!(
        device.read(0x64, AccessWidth::Word, SimTime::ZERO).unwrap() & 1,
        1
    );
    device
        .write(0x64, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert!(!handle.interrupt_pending());
}

#[test]
fn phy_tsf_latch_and_four_timer_interrupts_follow_vendor_hal_layout() {
    let mut phy = EspC6PhyRegisters::new("phy");
    let handle = phy.handle();
    phy.write(
        C6_PHY_TSF_LATCH_CONTROL,
        AccessWidth::Word,
        1,
        SimTime::from_ticks(160),
    )
    .unwrap();
    assert_eq!(
        phy.read(
            C6_PHY_TSF_LOW,
            AccessWidth::Word,
            SimTime::from_ticks(160)
        )
        .unwrap(),
        10
    );
    assert_eq!(
        phy.read(
            C6_PHY_TSF_HIGH,
            AccessWidth::Word,
            SimTime::from_ticks(160)
        )
        .unwrap(),
        0
    );

    phy.write(
        C6_PHY_TSF_TIMER_TARGET_BASE,
        AccessWidth::Word,
        12,
        SimTime::ZERO,
    )
    .unwrap();
    phy.write(
        C6_PHY_POWER_INTERRUPT_CLEAR,
        AccessWidth::Word,
        0x80,
        SimTime::ZERO,
    )
    .unwrap();
    phy.write(
        C6_PHY_POWER_INTERRUPT_ENABLE,
        AccessWidth::Word,
        0x80,
        SimTime::ZERO,
    )
    .unwrap();
    phy.write(
        C6_PHY_TSF_TIMER_CONTROL_BASE,
        AccessWidth::Word,
        u64::from(C6_PHY_TSF_TIMER_ENABLE | C6_PHY_TSF_TIMER_WAKEUP_ENABLE | 3),
        SimTime::ZERO,
    )
    .unwrap();
    handle.validate_tsf_timers().unwrap();
    assert_eq!(handle.advance_to(SimTime::from_ticks(191)), 0);
    assert_eq!(handle.advance_to(SimTime::from_ticks(192)), 1);
    assert!(handle.interrupt_pending());
    assert_eq!(
        phy.read(
            C6_PHY_POWER_INTERRUPT_RAW,
            AccessWidth::Word,
            SimTime::from_ticks(192)
        )
        .unwrap(),
        0x80
    );
    assert_eq!(
        phy.read(
            C6_PHY_POWER_INTERRUPT_STATUS,
            AccessWidth::Word,
            SimTime::from_ticks(192)
        )
        .unwrap(),
        0x80
    );
    phy.write(
        C6_PHY_POWER_INTERRUPT_CLEAR,
        AccessWidth::Word,
        0x80,
        SimTime::from_ticks(192),
    )
    .unwrap();
    assert!(!handle.interrupt_pending());
    assert_eq!(handle.advance_to(SimTime::from_ticks(208)), 0);
}

#[test]
fn phy_tsf_legality_rejects_orders_the_vendor_hal_never_emits() {
    let mut phy = EspC6PhyRegisters::new("phy");
    let handle = phy.handle();
    phy.write(
        C6_PHY_TSF_TIMER_CONTROL_BASE,
        AccessWidth::Word,
        u64::from(C6_PHY_TSF_TIMER_ENABLE),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(
        handle
            .validate_tsf_timers()
            .unwrap_err()
            .contains("enabled before its firmware interrupt bit")
    );
    phy.write(
        C6_PHY_TSF_TIMER_CONTROL_BASE,
        AccessWidth::Word,
        u64::from(C6_PHY_TSF_TIMER_WAKEUP_ENABLE),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(
        handle
            .validate_tsf_timers()
            .unwrap_err()
            .contains("requests wakeup while disabled")
    );
}

#[test]
fn ieee802154_timers_use_simulation_time() {
    let (mut device, handle) = EspIeee802154::new("ieee802154");
    device
        .write(0x60, AccessWidth::Word, 1 << 8, SimTime::ZERO)
        .unwrap();
    device
        .write(0xa8, AccessWidth::Word, 10, SimTime::ZERO)
        .unwrap();
    device
        .write(0x00, AccessWidth::Word, 0x4c, SimTime::from_ticks(3))
        .unwrap();
    assert_eq!(
        device
            .read(0xac, AccessWidth::Word, SimTime::from_ticks(12))
            .unwrap(),
        9
    );
    let _ = device
        .read(0xac, AccessWidth::Word, SimTime::from_ticks(13))
        .unwrap();
    assert!(handle.interrupt_pending());
}

#[test]
fn ieee802154_stop_retires_ack_receive_state() {
    let (mut device, handle) = EspIeee802154::new("ieee802154");
    handle.complete_tx_expect_ack(0x45);
    assert_eq!(handle.awaiting_ack_sequence(), Some(0x45));
    device
        .write(0x00, AccessWidth::Word, 0x45, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.awaiting_ack_sequence(), None);
}
