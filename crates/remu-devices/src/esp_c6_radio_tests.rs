#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c6_ble_sleep_timer_advances_at_the_firmware_selected_rate() {
        let (mut modem, handle) = EspC6BleModem::new("ble-modem");
        assert_eq!(
            modem
                .read(C6_BLE_MODEM_TIMER_CURRENT, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        assert_eq!(
            modem
                .read(
                    C6_BLE_MODEM_TIMER_CURRENT,
                    AccessWidth::Word,
                    SimTime::from_ticks(159),
                )
                .unwrap(),
            0
        );
        assert_eq!(
            modem
                .read(
                    C6_BLE_MODEM_TIMER_CURRENT,
                    AccessWidth::Word,
                    SimTime::from_ticks(160),
                )
                .unwrap(),
            1
        );
        assert!(
            modem
                .write(
                    C6_BLE_MODEM_TIMER_CURRENT,
                    AccessWidth::Word,
                    7,
                    SimTime::ZERO,
                )
                .is_err()
        );

        modem
            .write(
                C6_BLE_MODEM_TIMER_COMPARE,
                AccessWidth::Word,
                3,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            modem
                .read(
                    C6_BLE_MODEM_TIMER_INTERRUPT_RAW,
                    AccessWidth::Word,
                    SimTime::from_ticks(479),
                )
                .unwrap(),
            0
        );
        assert!(!handle.interrupt_pending(SimTime::from_ticks(480)));
        assert_eq!(
            modem
                .read(
                    C6_BLE_MODEM_TIMER_INTERRUPT_RAW,
                    AccessWidth::Word,
                    SimTime::from_ticks(480),
                )
                .unwrap(),
            1
        );
        modem
            .write(
                C6_BLE_MODEM_TIMER_INTERRUPT_RAW,
                AccessWidth::Word,
                0,
                SimTime::from_ticks(480),
            )
            .unwrap();
        assert_eq!(
            modem
                .read(
                    C6_BLE_MODEM_TIMER_INTERRUPT_RAW,
                    AccessWidth::Word,
                    SimTime::from_ticks(640),
                )
                .unwrap(),
            0
        );

        modem
            .write(
                C6_BLE_MODEM_RTC_COMPARE,
                AccessWidth::Word,
                5,
                SimTime::ZERO,
            )
            .unwrap();
        modem
            .write(
                C6_BLE_MODEM_RTC_INTERRUPT_ENABLE,
                AccessWidth::Word,
                C6_BLE_MODEM_RTC_INTERRUPT_BIT.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.interrupt_pending(SimTime::from_ticks(799)));
        assert!(handle.interrupt_pending(SimTime::from_ticks(800)));
        assert_eq!(
            modem
                .read(
                    C6_BLE_MODEM_RTC_INTERRUPT_STATUS,
                    AccessWidth::Word,
                    SimTime::from_ticks(800),
                )
                .unwrap(),
            1
        );
        assert_eq!(
            modem
                .read(
                    C6_BLE_MODEM_RTC_TIMER0_PENDING,
                    AccessWidth::Word,
                    SimTime::from_ticks(800),
                )
                .unwrap(),
            1
        );
        modem
            .write(
                C6_BLE_MODEM_RTC_INTERRUPT_CLEAR,
                AccessWidth::Word,
                C6_BLE_MODEM_RTC_INTERRUPT_BIT.into(),
                SimTime::from_ticks(800),
            )
            .unwrap();
        assert!(!handle.interrupt_pending(SimTime::from_ticks(960)));
        assert_eq!(
            modem
                .read(
                    C6_BLE_MODEM_RTC_TIMER0_PENDING,
                    AccessWidth::Word,
                    SimTime::from_ticks(960),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn wifi_crypto_table_exposes_valid_hal_entries_and_reset_zeroizes_them() {
        let mut mac = EspC6WifiMacRegisters::new("wifi-mac");
        let handle = mac.handle();
        let slot = 2_u64;
        let base = C6_WIFI_MAC_CRYPTO_TABLE + slot * C6_WIFI_MAC_CRYPTO_ENTRY_STRIDE;
        mac.write(base, AccessWidth::Word, 0x4433_2211, SimTime::ZERO)
            .unwrap();
        mac.write(
            base + 4,
            AccessWidth::Word,
            u64::from((6_u32 << 21) | 0x6655),
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
            C6_WIFI_MAC_CRYPTO_VALID,
            AccessWidth::Word,
            1 << slot,
            SimTime::ZERO,
        )
        .unwrap();

        let entry = handle.crypto_key_entry(slot as u8).unwrap();
        assert_eq!(entry.match_low, 0x4433_2211);
        assert_eq!(entry.control, (6 << 21) | 0x6655);
        assert_eq!(&entry.key[..8], &[0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(handle.validate_crypto_key_table().is_ok());

        mac.reset(ResetKind::PowerOn);
        assert!(handle.crypto_key_entry(slot as u8).is_none());
        for word in 0..C6_WIFI_MAC_CRYPTO_ENTRY_WORDS as u64 {
            assert_eq!(
                mac.read(base + word * 4, AccessWidth::Word, SimTime::ZERO)
                    .unwrap(),
                0
            );
        }
    }

    #[test]
    fn wifi_crypto_table_rejects_a_valid_slot_outside_hal_control_classes() {
        let mut mac = EspC6WifiMacRegisters::new("wifi-mac");
        let handle = mac.handle();
        mac.write(
            C6_WIFI_MAC_CRYPTO_TABLE + 4,
            AccessWidth::Word,
            1 << 21,
            SimTime::ZERO,
        )
        .unwrap();
        mac.write(
            C6_WIFI_MAC_CRYPTO_VALID,
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert!(
            handle
                .validate_crypto_key_table()
                .unwrap_err()
                .contains("impossible control class 1")
        );
    }

    #[test]
    fn c6_ble_sleep_timer_rejects_impossible_wake_ordering() {
        let (mut modem, _) = EspC6BleModem::new("ble-modem");
        let enable_error = modem
            .write(
                C6_BLE_MODEM_RTC_INTERRUPT_ENABLE,
                AccessWidth::Word,
                C6_BLE_MODEM_RTC_INTERRUPT_BIT.into(),
                SimTime::ZERO,
            )
            .unwrap_err();
        assert!(enable_error.to_string().contains("illegal radio state"));

        let compare_error = modem
            .write(
                C6_BLE_MODEM_RTC_COMPARE,
                AccessWidth::Word,
                4,
                SimTime::from_ticks(5 * C6_BLE_MODEM_TICKS_PER_SLEEP_TICK),
            )
            .unwrap_err();
        assert!(compare_error.to_string().contains("illegal radio state"));

        modem
            .write(
                C6_BLE_MODEM_RTC_INTERRUPT_CLEAR,
                AccessWidth::Word,
                C6_BLE_MODEM_RTC_INTERRUPT_BIT.into(),
                SimTime::ZERO,
            )
            .unwrap();
        modem
            .write(
                C6_BLE_MODEM_RTC_COMPARE,
                AccessWidth::Word,
                10,
                SimTime::ZERO,
            )
            .unwrap();
        let clear_error = modem
            .write(
                C6_BLE_MODEM_RTC_INTERRUPT_CLEAR,
                AccessWidth::Word,
                C6_BLE_MODEM_RTC_INTERRUPT_BIT.into(),
                SimTime::ZERO,
            )
            .unwrap_err();
        assert!(clear_error.to_string().contains("illegal radio state"));

        modem
            .write(
                C6_BLE_MODEM_RTC_INTERRUPT_ENABLE,
                AccessWidth::Word,
                C6_BLE_MODEM_RTC_INTERRUPT_BIT.into(),
                SimTime::ZERO,
            )
            .unwrap();
        modem
            .write(
                C6_BLE_MODEM_RTC_INTERRUPT_CLEAR,
                AccessWidth::Word,
                C6_BLE_MODEM_RTC_INTERRUPT_BIT.into(),
                SimTime::from_ticks(9 * C6_BLE_MODEM_TICKS_PER_SLEEP_TICK),
            )
            .unwrap();
    }

    #[test]
    fn ble_baseband_scheduler_timer_starts_on_reset_release_at_one_mhz() {
        let (mut device, handle) = EspC6BleBaseband::new("ble-baseband");
        device
            .write(
                C6_BLE_BASEBAND_RESET,
                AccessWidth::Word,
                0,
                SimTime::from_ticks(1_000),
            )
            .unwrap();
        device
            .write(
                C6_BLE_BASEBAND_RESET,
                AccessWidth::Word,
                1,
                SimTime::from_ticks(1_016),
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    C6_BLE_BASEBAND_TIMER_CURRENT,
                    AccessWidth::Word,
                    SimTime::from_ticks(33_016),
                )
                .unwrap(),
            2_000
        );
        assert_eq!(
            handle.scheduler_timestamp(SimTime::from_ticks(33_016)),
            2_000
        );
    }

    #[test]
    fn ble_baseband_queues_native_schedule_and_publishes_w1c_event_end() {
        let (mut device, handle) = EspC6BleBaseband::new("ble-baseband");
        device
            .write(
                C6_BLE_BASEBAND_INTERRUPT_ENABLE0,
                AccessWidth::Word,
                u64::from(C6_BLE_BASEBAND_EVENT_END),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                C6_BLE_BASEBAND_SCHEDULER_HEAD,
                AccessWidth::Word,
                0x0007_ef84,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                C6_BLE_BASEBAND_SCHEDULER_KICK,
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(handle.take_schedule().unwrap().address, 0x4087_ef84);
        handle.schedule_event_end(SimTime::from_ticks(100), 0x4087_ef84, None);
        handle.advance_to(SimTime::from_ticks(99));
        assert!(!handle.interrupt_pending());
        handle.advance_to(SimTime::from_ticks(100));
        assert!(handle.interrupt_pending());
        assert_eq!(
            handle.take_completed_schedule().unwrap().address,
            0x4087_ef84
        );
        device
            .write(
                C6_BLE_BASEBAND_INTERRUPT_CLEAR0,
                AccessWidth::Word,
                u64::from(C6_BLE_BASEBAND_EVENT_END),
                SimTime::from_ticks(101),
            )
            .unwrap();
        assert!(!handle.interrupt_pending());
        assert_eq!(
            device
                .read(
                    C6_BLE_BASEBAND_SCHEDULER_CURRENT,
                    AccessWidth::Word,
                    SimTime::from_ticks(102),
                )
                .unwrap(),
            0xa007_ef84
        );
        assert_eq!(
            device
                .read(
                    C6_BLE_BASEBAND_SCHEDULER_CURRENT,
                    AccessWidth::Word,
                    SimTime::from_ticks(103),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn ble_baseband_publishes_loaded_native_buffer_cursors() {
        let (mut device, handle) = EspC6BleBaseband::new("ble-baseband");
        device
            .write(
                C6_BLE_BASEBAND_SCHEDULER_HEAD,
                AccessWidth::Word,
                0x0007_ef84,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                C6_BLE_BASEBAND_SCHEDULER_KICK,
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();

        handle.set_loaded_buffer_headers(
            0x4087_ef84,
            Some(0x4087_f4dc),
            Some(0x4087_f65c),
        );
        assert_eq!(
            device
                .read(
                    C6_BLE_BASEBAND_CURRENT_TX_BUFFER,
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x0007_f4e0
        );
        assert_eq!(
            device
                .read(
                    C6_BLE_BASEBAND_CURRENT_RX_BUFFER,
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x0007_f660
        );

        handle.set_loaded_buffer_headers(0x4081_0000, None, None);
        assert_eq!(
            device
                .read(
                    C6_BLE_BASEBAND_CURRENT_TX_BUFFER,
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x0007_f4e0
        );
    }

    #[test]
    fn ble_baseband_received_completion_replaces_same_schedule_timeout() {
        let (_, handle) = EspC6BleBaseband::new("ble-baseband");
        handle.schedule_event_end(SimTime::from_ticks(100), 0x4087_ef84, None);
        handle.schedule_received_event_end(
            SimTime::from_ticks(50),
            0x4087_ef84,
            Some(0x4081_de6c),
        );

        handle.advance_to(SimTime::from_ticks(50));
        assert_eq!(
            handle.take_completed_schedule().unwrap().address,
            0x4087_ef84
        );
        handle.advance_to(SimTime::from_ticks(100));
        assert!(handle.take_completed_schedule().is_none());
    }

    #[test]
    fn ble_baseband_advances_current_to_loaded_successor_after_acknowledgement() {
        let (mut device, handle) = EspC6BleBaseband::new("ble-baseband");
        device
            .write(
                C6_BLE_BASEBAND_SCHEDULER_HEAD,
                AccessWidth::Word,
                0x0007_ef84,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                C6_BLE_BASEBAND_SCHEDULER_KICK,
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        handle.set_loaded_schedule_successor(0x4087_ef84, Some(0x4081_de6c));
        handle.schedule_event_end(SimTime::from_ticks(100), 0x4087_ef84, Some(0x4081_de6c));
        handle.advance_to(SimTime::from_ticks(100));
        device
            .write(
                C6_BLE_BASEBAND_INTERRUPT_CLEAR0,
                AccessWidth::Word,
                u64::from(C6_BLE_BASEBAND_EVENT_END),
                SimTime::from_ticks(101),
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    C6_BLE_BASEBAND_SCHEDULER_CURRENT,
                    AccessWidth::Word,
                    SimTime::from_ticks(102),
                )
                .unwrap(),
            0xa007_ef84
        );
        assert_eq!(
            device
                .read(
                    C6_BLE_BASEBAND_SCHEDULER_CURRENT,
                    AccessWidth::Word,
                    SimTime::from_ticks(103),
                )
                .unwrap(),
            0xa001_de6c
        );
    }

    #[test]
    fn ble_baseband_stop_cancels_future_schedule_work() {
        let (mut device, handle) = EspC6BleBaseband::new("ble-baseband");
        device
            .write(
                C6_BLE_BASEBAND_SCHEDULER_HEAD,
                AccessWidth::Word,
                0x0007_ef84,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                C6_BLE_BASEBAND_SCHEDULER_KICK,
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        handle.schedule_event_end(SimTime::from_ticks(100), 0x4087_ef84, None);

        device
            .write(
                C6_BLE_BASEBAND_SCHEDULER_STOP,
                AccessWidth::Word,
                1,
                SimTime::from_ticks(50),
            )
            .unwrap();

        assert!(handle.take_stop_request());
        assert!(!handle.take_stop_request());
        assert!(handle.take_schedule().is_none());
        handle.advance_to(SimTime::from_ticks(100));
        assert!(handle.take_completed_schedule().is_none());
        assert_eq!(
            device
                .read(
                    C6_BLE_BASEBAND_SCHEDULER_CURRENT,
                    AccessWidth::Word,
                    SimTime::from_ticks(101),
                )
                .unwrap()
                & 0x8000_0000,
            0
        );
    }

    #[test]
    fn ble_modem_security_ecb_captures_native_dma_command_and_completes() {
        let (mut device, handle) = EspC6BleControl::new("ble-control");
        let key = [
            0x00_u8, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        for word in 0..4 {
            device
                .write(
                    C6_BLE_ECB_KEY_BASE + word as u64 * 4,
                    AccessWidth::Word,
                    u64::from(u32::from_le_bytes(
                        key[word * 4..word * 4 + 4].try_into().unwrap(),
                    )),
                    SimTime::ZERO,
                )
                .unwrap();
        }
        device
            .write(C6_BLE_ECB_LENGTH, AccessWidth::Word, 16, SimTime::ZERO)
            .unwrap();
        device
            .write(
                C6_BLE_ECB_INPUT_ADDRESS,
                AccessWidth::Word,
                0x4080_1000,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                C6_BLE_ECB_OUTPUT_ADDRESS,
                AccessWidth::Word,
                0x4080_2000,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(C6_BLE_ECB_START, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        let command = handle.take_ecb_command().unwrap();
        assert_eq!(command.input_address, 0x4080_1000);
        assert_eq!(command.output_address, 0x4080_2000);
        assert_eq!(command.length, 16);
        assert_eq!(
            command.encrypt_block([
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff,
            ]),
            [
                0x69, 0xc4, 0xe0, 0xd8, 0x6a, 0x7b, 0x04, 0x30, 0xd8, 0xcd, 0xb7, 0x80, 0x70, 0xb4,
                0xc5, 0x5a,
            ]
        );
        assert_eq!(
            device
                .read(C6_BLE_ECB_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        handle.complete_ecb();
        assert_eq!(
            device
                .read(C6_BLE_ECB_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
    }

    #[test]
    fn ble_modem_security_ccm_decodes_peripheral_rx_direction_and_completes() {
        let (mut device, handle) = EspC6BleControl::new("ble-control");
        let key = [
            0x75_u8, 0xc1, 0x90, 0x34, 0xa2, 0x8e, 0x97, 0xa8, 0x23, 0x03, 0x54, 0xbe, 0x03, 0x29,
            0x4d, 0xb5,
        ];
        for word in 0..4 {
            device
                .write(
                    C6_BLE_CCM_KEY_BASE + word as u64 * 4,
                    AccessWidth::Word,
                    u64::from(u32::from_le_bytes(
                        key[word * 4..word * 4 + 4].try_into().unwrap(),
                    )),
                    SimTime::ZERO,
                )
                .unwrap();
        }
        for (offset, value) in [
            (C6_BLE_CCM_CONFIG, 0x1013),
            (C6_BLE_CCM_INPUT_ADDRESS, 0x4082_1022),
            (C6_BLE_CCM_OUTPUT_ADDRESS, 0x4081_d960),
            (C6_BLE_CCM_COUNTER_LOW, 0),
            (C6_BLE_CCM_COUNTER_IV0, 0x3231_3080),
            (C6_BLE_CCM_IV1, 0xc063_0633),
            (C6_BLE_CCM_IV2, 0x23),
            (C6_BLE_CCM_AAD, 0x03),
        ] {
            device
                .write(offset, AccessWidth::Word, value, SimTime::ZERO)
                .unwrap();
        }
        device
            .write(C6_BLE_CCM_START, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();

        let command = handle.take_ccm_command().unwrap();
        assert!(command.decrypt);
        assert!(!command.peripheral_to_central);
        assert_eq!(command.payload_length, 1);
        assert_eq!(command.packet_counter, 0);
        assert_eq!(command.iv, [0x30, 0x31, 0x32, 0x33, 0x06, 0x63, 0xc0, 0x23]);
        assert_eq!(command.key, key);
        assert_eq!(command.aad_header, 0x03);
        assert_eq!(
            device
                .read(C6_BLE_CCM_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        handle.complete_ccm(true);
        assert_eq!(
            device
                .read(C6_BLE_CCM_RESULT, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
        assert_eq!(
            device
                .read(C6_BLE_CCM_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
    }

    #[test]
    fn modem_reset_values_masks_and_domains_are_visible() {
        let (mut syscon, mut lpcon, handle) = EspC6ModemControl::new_pair("syscon", "lpcon");
        assert_eq!(
            syscon.read(0x24, AccessWidth::Word, SimTime::ZERO).unwrap(),
            35_676_928
        );
        assert_eq!(
            lpcon.read(0x28, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x2_0015
        );
        syscon
            .write(
                0x14,
                AccessWidth::Word,
                (1 << 9) | (1 << 10) | (1 << 17) | (1 << 18),
                SimTime::ZERO,
            )
            .unwrap();
        syscon
            .write(
                0x04,
                AccessWidth::Word,
                (1 << 23) | (1 << 24),
                SimTime::ZERO,
            )
            .unwrap();
        lpcon
            .write(0x18, AccessWidth::Word, 1 << 1, SimTime::ZERO)
            .unwrap();
        assert!(handle.wifi_ready());
        assert!(handle.ble_ready());
        assert!(handle.ieee802154_ready());
        assert!(handle.coexistence_ready());
    }

    #[test]
    fn power_detector_start_completes_conversion_without_guest_hooks() {
        let mut detector = EspC6PowerDetector::new("power-detector");
        detector
            .write(
                C6_POWER_DETECTOR_CONVERSION,
                AccessWidth::Word,
                C6_POWER_DETECTOR_START as u64,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            detector
                .read(
                    C6_POWER_DETECTOR_CONVERSION,
                    AccessWidth::Word,
                    SimTime::ZERO
                )
                .unwrap(),
            u64::from(C6_POWER_DETECTOR_START | C6_POWER_DETECTOR_DONE)
        );
        detector
            .write(
                C6_POWER_DETECTOR_CONVERSION,
                AccessWidth::Word,
                0,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            detector
                .read(
                    C6_POWER_DETECTOR_CONVERSION,
                    AccessWidth::Word,
                    SimTime::ZERO
                )
                .unwrap(),
            0
        );
        detector
            .write(
                C6_POWER_DETECTOR_TONE_CONTROL,
                AccessWidth::Word,
                C6_POWER_DETECTOR_START as u64,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            detector
                .read(
                    C6_POWER_DETECTOR_TONE_STATUS,
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            u64::from(C6_POWER_DETECTOR_TONE_IDLE)
        );
        detector
            .write(
                C6_FREQUENCY_CONTROL,
                AccessWidth::Word,
                C6_FREQUENCY_CHANNEL_START as u64,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            detector
                .read(C6_FREQUENCY_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(C6_FREQUENCY_CHANNEL_DONE),
            u64::from(C6_FREQUENCY_CHANNEL_DONE)
        );
        detector
            .write(
                C6_IQ_ESTIMATE_CONTROL,
                AccessWidth::Word,
                C6_IQ_ESTIMATE_START as u64,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            detector
                .read(C6_IQ_ESTIMATE_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(C6_IQ_ESTIMATE_DONE),
            u64::from(C6_IQ_ESTIMATE_DONE)
        );
    }

    #[test]
    fn wifi_mac_reset_command_sets_ready_status() {
        let mut mac = EspC6WifiMacRegisters::new("wifi-mac");
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
}
