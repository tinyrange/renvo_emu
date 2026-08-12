use super::*;

#[test]
fn wifi_crypto_table_matches_the_s3_hal_layout_and_zeroizes_on_reset() {
    let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    let slot = 3_u64;
    let base = WIFI_MAC_CRYPTO_TABLE + slot * WIFI_MAC_CRYPTO_ENTRY_STRIDE;
    mac.write(base, AccessWidth::Word, 0x4433_2211, SimTime::ZERO)
        .unwrap();
    mac.write(
        base + 4,
        AccessWidth::Word,
        u64::from((7_u32 << 21) | 0x6655),
        SimTime::ZERO,
    )
    .unwrap();
    for word in 0..8_u64 {
        mac.write(
            base + 8 + word * 4,
            AccessWidth::Word,
            0x0302_0100 + word * 0x0404_0404,
            SimTime::ZERO,
        )
        .unwrap();
    }
    mac.write(
        WIFI_MAC_CRYPTO_VALID,
        AccessWidth::Word,
        1 << slot,
        SimTime::ZERO,
    )
    .unwrap();

    let entry = handle.crypto_key_entry(slot as u8).unwrap();
    assert_eq!(entry.match_low, 0x4433_2211);
    assert_eq!(entry.control, (7 << 21) | 0x6655);
    assert_eq!(&entry.key[..8], &[0, 1, 2, 3, 4, 5, 6, 7]);
    assert!(handle.validate_crypto_key_table().is_ok());

    mac.reset(ResetKind::PowerOn);
    assert!(handle.crypto_key_entry(slot as u8).is_none());
    for word in 0..WIFI_MAC_CRYPTO_ENTRY_WORDS as u64 {
        assert_eq!(
            mac.read(base + word * 4, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
    }
}

#[test]
fn wifi_crypto_table_rejects_a_valid_slot_outside_hal_control_classes() {
    let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    mac.write(
        WIFI_MAC_CRYPTO_TABLE + 4,
        AccessWidth::Word,
        2 << 21,
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(WIFI_MAC_CRYPTO_VALID, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert!(
        handle
            .validate_crypto_key_table()
            .unwrap_err()
            .contains("impossible control class 2")
    );
}

#[test]
fn ble_half_slot_timer_raises_one_native_interrupt_at_programmed_fine_time() {
    let mut ble = Esp32S3BleExchangeMemoryRegisters::new("ble");
    let handle = ble.handle();
    let armed_at = SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 10 + 100);
    ble.write(BLE_TIMER_HALF_SLOT, AccessWidth::Word, 12, armed_at)
        .unwrap();
    ble.write(BLE_TIMER_FINE, AccessWidth::Word, 624, armed_at)
        .unwrap();
    ble.write(
        BLE_INTERRUPT_CLEAR,
        AccessWidth::Word,
        u64::from(BLE_TIMER_INTERRUPT),
        armed_at,
    )
    .unwrap();
    ble.write(
        BLE_INTERRUPT_ENABLE,
        AccessWidth::Word,
        u64::from(BLE_TIMER_INTERRUPT),
        armed_at,
    )
    .unwrap();

    handle.advance_to(SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 12 - 1));
    assert!(!handle.interrupt_pending());
    handle.advance_to(SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 12));
    assert!(handle.interrupt_pending());
    ble.write(
        BLE_ECO_INTERRUPT_DIAGNOSTIC,
        AccessWidth::Word,
        0x8000_001e,
        SimTime::ZERO,
    )
    .unwrap();
    ble.write(
        BLE_TIMER_HALF_SLOT,
        AccessWidth::Word,
        14,
        SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 12),
    )
    .unwrap();
    ble.write(
        BLE_TIMER_FINE,
        AccessWidth::Word,
        624,
        SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 12),
    )
    .unwrap();
    assert_eq!(
        ble.read(
            BLE_ECO_INTERRUPT_DIAGNOSTIC,
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        u64::from(
            0x8000_001e | BLE_ECO_ACTIVE_STATE | (BLE_TIMER_INTERRUPT << BLE_ECO_STATUS_SHIFT)
        )
    );
    ble.write(
        BLE_INTERRUPT_CLEAR,
        AccessWidth::Word,
        u64::from(BLE_TIMER_INTERRUPT),
        SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 12),
    )
    .unwrap();
    assert_eq!(
        ble.read(
            BLE_ECO_INTERRUPT_DIAGNOSTIC,
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x8000_001e
    );
    handle.advance_to(SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 13));
    assert!(!handle.interrupt_pending());
    handle.advance_to(SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 14));
    assert!(handle.interrupt_pending());
}

#[test]
fn ble_exchange_memory_exposes_mapping_kick_and_w1c_interrupt_contract() {
    let mut ble = Esp32S3BleExchangeMemoryRegisters::new("ble");
    let handle = ble.handle();
    let cpu_address = 0x3fca_4000_u32;
    let em_offset = 0x0c34_u32;
    let encoded_mapping =
        ((em_offset >> 2) << BLE_EM_OFFSET_SHIFT) | ((cpu_address & 0x000f_ffff) >> 2);
    ble.write(
        BLE_EM_MAPPING_BANK0_FIRST,
        AccessWidth::Word,
        u64::from(encoded_mapping),
        SimTime::ZERO,
    )
    .unwrap();
    // A programmed base does not become visible to hardware until the ROM
    // marks its mapping slot allocated in the corresponding bitmap.
    assert_eq!(handle.resolve_em_address(em_offset as u16), None);
    ble.write(
        BLE_EM_MAPPING_VALID_LOW,
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    ble.write(
        BLE_SCHEDULER_KICK,
        AccessWidth::Word,
        u64::from(BLE_SCHEDULER_START | 0x42),
        SimTime::ZERO,
    )
    .unwrap();

    assert_eq!(
        handle.em_mappings(),
        [Esp32S3BleEmMapping {
            em_offset,
            cpu_address,
        }]
    );
    assert_eq!(handle.resolve_em_address(0x0c76), Some(cpu_address + 0x42));
    assert_eq!(handle.resolve_em_address(0x0800), None);
    assert_eq!(
        handle.take_schedule_kick(),
        Some(Esp32S3BleScheduleKick {
            control: BLE_SCHEDULER_START | 0x42,
        })
    );
    assert_eq!(handle.take_schedule_kick(), None);

    ble.write(
        BLE_EM_MAPPING_VALID_LOW,
        AccessWidth::Word,
        0,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.resolve_em_address(0x0c76), None);

    handle.raise_interrupt((1 << 2) | (1 << 6));
    assert!(handle.interrupt_pending());
    ble.write(
        BLE_INTERRUPT_CLEAR,
        AccessWidth::Word,
        1 << 2,
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.interrupt_pending());
    assert_eq!(
        ble.read(BLE_INTERRUPT_STATUS, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        1 << 6
    );
    ble.write(
        BLE_INTERRUPT_CLEAR,
        AccessWidth::Word,
        1 << 6,
        SimTime::ZERO,
    )
    .unwrap();
    assert!(!handle.interrupt_pending());

    handle.schedule_radio_completion(SimTime::from_ticks(12), (1 << 1) | (1 << 5));
    handle.advance_to(SimTime::from_ticks(11));
    assert!(!handle.interrupt_pending());
    handle.advance_to(SimTime::from_ticks(12));
    assert!(handle.interrupt_pending());
    let diagnostic = ble
        .read(
            BLE_ECO_INTERRUPT_DIAGNOSTIC,
            AccessWidth::Word,
            SimTime::from_ticks(12),
        )
        .unwrap();
    ble.write(
        BLE_ECO_INTERRUPT_DIAGNOSTIC,
        AccessWidth::Word,
        diagnostic,
        SimTime::from_ticks(12),
    )
    .unwrap();
    assert!(!handle.interrupt_pending());
}

#[test]
fn ble_crypt_start_captures_one_ecb_block_and_raises_native_interrupt() {
    let mut ble = Esp32S3BleExchangeMemoryRegisters::new("ble");
    let handle = ble.handle();
    let input = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
        0xff,
    ];
    for (word, bytes) in input.chunks_exact(4).enumerate() {
        ble.write(
            BLE_CRYPT_KEY_BASE + word as u64 * 4,
            AccessWidth::Word,
            u64::from(u32::from_le_bytes(bytes.try_into().unwrap())),
            SimTime::ZERO,
        )
        .unwrap();
    }
    ble.write(
        BLE_CRYPT_INPUT_RESULT_OFFSET,
        AccessWidth::Word,
        0x0128,
        SimTime::ZERO,
    )
    .unwrap();
    ble.write(
        BLE_CRYPT_START,
        AccessWidth::Word,
        u64::from(BLE_CRYPT_START_REQUEST),
        SimTime::ZERO,
    )
    .unwrap();

    let command = handle.take_crypt_command().unwrap();
    assert_eq!(command.key, input);
    assert_eq!(command.input_offset, 0x0128);
    assert_eq!(
        ble.read(BLE_CRYPT_START, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0
    );
    assert!(!handle.interrupt_pending());
    handle.complete_crypt();
    assert!(handle.interrupt_pending());
    ble.write(
        BLE_INTERRUPT_CLEAR,
        AccessWidth::Word,
        u64::from(BLE_CRYPT_INTERRUPT),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(!handle.interrupt_pending());
}

#[test]
fn packet_detector_is_idle_at_reset_and_after_rom_restart() {
    let mut phy = Esp32S3PhyRegisters::new("phy");
    let reset = phy
        .read(
            PHY_PACKET_DETECTOR_CONTROL,
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap() as u32;
    assert_eq!(
        reset & PHY_PACKET_DETECTOR_STATE_MASK,
        PHY_PACKET_DETECTOR_IDLE
    );

    phy.write(
        PHY_PACKET_DETECTOR_CONTROL,
        AccessWidth::Word,
        u64::from(PHY_PACKET_DETECTOR_RESTART),
        SimTime::ZERO,
    )
    .unwrap();
    let restarted = phy
        .read(
            PHY_PACKET_DETECTOR_CONTROL,
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap() as u32;
    assert_ne!(restarted & PHY_PACKET_DETECTOR_RESTART, 0);
    assert_eq!(
        restarted & PHY_PACKET_DETECTOR_STATE_MASK,
        PHY_PACKET_DETECTOR_IDLE
    );
}

#[test]
fn packet_detector_status_is_hardware_owned() {
    let mut phy = Esp32S3PhyRegisters::new("phy");
    phy.write(
        PHY_PACKET_DETECTOR_CONTROL,
        AccessWidth::Word,
        u32::MAX as u64 & !PHY_PACKET_DETECTOR_STATE_MASK as u64,
        SimTime::ZERO,
    )
    .unwrap();
    let value = phy
        .read(
            PHY_PACKET_DETECTOR_CONTROL,
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap() as u32;
    assert_eq!(
        value & PHY_PACKET_DETECTOR_STATE_MASK,
        PHY_PACKET_DETECTOR_IDLE
    );
}

#[test]
fn tx_dc_calibration_completes_after_command_edge() {
    let mut phy = Esp32S3PhyRegisters::new("phy");
    phy.write(
        PHY_TX_DC_CALIBRATION_CONTROL,
        AccessWidth::Word,
        0x0011_3cf1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        phy.read(
            PHY_TX_DC_CALIBRATION_CONTROL,
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap() as u32
            & PHY_TX_DC_CALIBRATION_DONE,
        0
    );

    phy.write(
        PHY_TX_DC_CALIBRATION_CONTROL,
        AccessWidth::Word,
        0x0011_3cf3,
        SimTime::ZERO,
    )
    .unwrap();
    let completed = phy
        .read(
            PHY_TX_DC_CALIBRATION_CONTROL,
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap() as u32;
    assert_ne!(completed & PHY_TX_DC_CALIBRATION_DONE, 0);
    assert_eq!(completed & 0xc000_0000, 0);
}

#[test]
fn agc_page_retains_rom_initialization_words_and_resets() {
    let mut agc = Esp32S3AgcRegisters::new("agc");
    agc.write(0x13c, AccessWidth::Word, 0x0130_0000, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        agc.read(0x13c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x0130_0000
    );
    agc.reset(ResetKind::PowerOn);
    assert_eq!(
        agc.read(0x13c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
}

#[test]
fn rx_iq_estimator_sets_front_end_completion() {
    let mut fe = Esp32S3FeRegisters::new("fe");
    fe.write(
        FE_IQ_ESTIMATE_CONTROL,
        AccessWidth::Word,
        u64::from(FE_IQ_ESTIMATE_START),
        SimTime::ZERO,
    )
    .unwrap();
    assert_ne!(
        fe.read(FE_IQ_ESTIMATE_STATUS, AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32
            & FE_IQ_ESTIMATE_DONE,
        0
    );
}

#[test]
fn wifi_mac_window_covers_all_three_native_pages() {
    let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
    for (offset, value) in [(0x0400, 3), (0x1d04, 0x1234), (0x2d04, 0x5678)] {
        mac.write(offset, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            mac.read(offset, AccessWidth::Word, SimTime::ZERO).unwrap(),
            value
        );
    }
    assert!(mac.read(0x3000, AccessWidth::Word, SimTime::ZERO).is_err());
}

#[test]
fn wifi_mac_reset_command_acknowledges_ready() {
    let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
    mac.write(
        WIFI_MAC_RESET_CONTROL,
        AccessWidth::Word,
        u64::from(WIFI_MAC_RESET_START),
        SimTime::ZERO,
    )
    .unwrap();
    let status = mac
        .read(WIFI_MAC_RESET_CONTROL, AccessWidth::Word, SimTime::ZERO)
        .unwrap() as u32;
    assert_eq!(
        status & (WIFI_MAC_RESET_START | WIFI_MAC_RESET_READY),
        WIFI_MAC_RESET_START | WIFI_MAC_RESET_READY
    );
}

#[test]
fn wifi_mac_rx_match_uses_firmware_programmed_interface_address() {
    let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    assert_eq!(handle.rx_match_mask(&[0xff; 6]), 1);

    mac.write(
        WIFI_MAC_INTERFACE_ADDRESS_LOW + WIFI_MAC_INTERFACE_ADDRESS_STRIDE,
        AccessWidth::Word,
        0x2233_4455,
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        WIFI_MAC_INTERFACE_ADDRESS_HIGH + WIFI_MAC_INTERFACE_ADDRESS_STRIDE,
        AccessWidth::Word,
        0x0111,
        SimTime::ZERO,
    )
    .unwrap();

    assert_eq!(
        handle.rx_match_mask(&[0x55, 0x44, 0x33, 0x22, 0x11, 0x01]),
        1 << 1
    );
    assert_eq!(
        handle.rx_match_mask(&[0x54, 0x44, 0x33, 0x22, 0x11, 0x02]),
        0
    );
    assert_eq!(handle.rx_match_mask(&[0xff; 6]), 1 << 1);
}

#[test]
fn wifi_mac_rx_block_ack_tracks_the_native_firmware_window() {
    let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    let peer = [0x02, 1, 2, 3, 4, 5];
    mac.write(
        WIFI_MAC_RX_BA_MAC_LOW_HIGH,
        AccessWidth::Word,
        u64::from(u32::from_le_bytes(peer[..4].try_into().unwrap())),
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        WIFI_MAC_RX_BA_MAC_HIGH_HIGH,
        AccessWidth::Word,
        u64::from(u16::from_le_bytes(peer[4..].try_into().unwrap())),
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        WIFI_MAC_RX_BA_SEQUENCE_HIGH,
        AccessWidth::Word,
        0x0ffe,
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        WIFI_MAC_RX_BA_CONTROL_HIGH,
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
        WIFI_MAC_RX_BA_CONTROL_HIGH,
        AccessWidth::Word,
        u64::from(WIFI_MAC_RX_BA_VALID | 5),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.validate_block_ack_sessions().is_err());
}

#[test]
fn wifi_tx_queue_completion_asserts_and_clears_mac_interrupt() {
    let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    mac.write(
        WIFI_MAC_INTERRUPT_MASK,
        AccessWidth::Word,
        u64::from(WIFI_MAC_EVENT_TX_DONE),
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        WIFI_MAC_TX_QUEUE_CONTROL_HIGH,
        AccessWidth::Word,
        u64::from(WIFI_MAC_TX_QUEUE_ENABLE | 0x5678),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        handle.take_tx_descriptor(),
        Some(Esp32S3WifiTxDescriptor {
            queue: 0,
            address: 0x3fc0_5678,
        })
    );
    assert!(!handle.interrupt_pending());
    assert_eq!(
        mac.read(WIFI_MAC_TX_QUEUE_STATE, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0
    );
    assert!(handle.tx_active(0));
    assert!(handle.complete_tx(0, crate::EspWifiTxOutcome::AckTimeout));
    assert!(handle.interrupt_pending());
    assert_eq!(
        mac.read(WIFI_MAC_TX_QUEUE_STATE, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        1
    );
    assert_eq!(
        mac.read(
            WIFI_MAC_TX_QUEUE_COMPLETION_HIGH,
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap() as u32
            & WIFI_MAC_TX_QUEUE_COMPLETION_STATUS,
        5 << 12
    );
    assert_ne!(
        mac.read(WIFI_MAC_INTERRUPT_EVENT, AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32
            & WIFI_MAC_EVENT_TX_DONE,
        0
    );

    mac.write(
        WIFI_MAC_INTERRUPT_CLEAR,
        AccessWidth::Word,
        u64::from(WIFI_MAC_EVENT_TX_DONE),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(!handle.interrupt_pending());
    mac.write(
        WIFI_MAC_TX_QUEUE_STATE_CLEAR,
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        mac.read(WIFI_MAC_TX_QUEUE_STATE, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0
    );
    assert_eq!(
        mac.read(WIFI_MAC_INTERRUPT_EVENT, AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32
            & WIFI_MAC_EVENT_TX_DONE,
        0
    );
}

#[test]
fn wifi_rx_base_advances_native_ring_and_asserts_event() {
    let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
    let handle = mac.handle();
    mac.write(
        WIFI_MAC_INTERRUPT_MASK,
        AccessWidth::Word,
        u64::from(WIFI_MAC_EVENT_RX_DONE),
        SimTime::ZERO,
    )
    .unwrap();
    mac.write(
        WIFI_MAC_RX_BASE,
        AccessWidth::Word,
        0x3fca_1000,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        handle.rx_descriptor(),
        Some(Esp32S3WifiRxDescriptor {
            address: 0x3fca_1000
        })
    );
    handle.complete_rx_descriptor(0x3fca_1000, 0x3fca_100c);
    assert_eq!(
        handle.rx_descriptor(),
        Some(Esp32S3WifiRxDescriptor {
            address: 0x3fca_100c
        })
    );
    assert_eq!(
        mac.read(WIFI_MAC_RX_NEXT, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0x3fca_100c
    );
    assert_eq!(
        mac.read(WIFI_MAC_RX_LAST, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0x000a_1000
    );
    assert!(handle.interrupt_pending());
    mac.write(
        WIFI_MAC_INTERRUPT_CLEAR,
        AccessWidth::Word,
        u64::from(WIFI_MAC_EVENT_RX_DONE),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(!handle.interrupt_pending());
}

#[test]
fn wifi_wdev_latches_tsf_and_provides_replayable_entropy() {
    let mut mac = Esp32S3WifiMacRegisters::new("wifi-mac");
    assert_eq!(
        mac.read(
            WIFI_MAC_CURRENT_TIME,
            AccessWidth::Word,
            SimTime::from_ticks(0x1234_5678),
        )
        .unwrap(),
        0x1234_5678
    );
    mac.write(
        WIFI_MAC_TSF_LATCH_CONTROL,
        AccessWidth::Word,
        1,
        SimTime::from_ticks(0x1_2345_6789),
    )
    .unwrap();
    assert_eq!(
        mac.read(WIFI_MAC_TSF_LOW, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0x2345_6789
    );
    assert_eq!(
        mac.read(WIFI_MAC_TSF_HIGH, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        1
    );

    let first = mac
        .read(WIFI_MAC_RANDOM_DATA, AccessWidth::Word, SimTime::ZERO)
        .unwrap();
    let second = mac
        .read(WIFI_MAC_RANDOM_DATA, AccessWidth::Word, SimTime::ZERO)
        .unwrap();
    assert_ne!(first, second);
    mac.reset(ResetKind::PowerOn);
    assert_eq!(
        mac.read(WIFI_MAC_RANDOM_DATA, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        first
    );
}
