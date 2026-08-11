#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(handle.interrupt_pending());
        assert_eq!(
            mac.read(C6_WIFI_MAC_TX_QUEUE_STATE, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
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
