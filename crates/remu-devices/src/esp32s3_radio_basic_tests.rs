mod basic_tests {
    use super::*;

    #[test]
    fn ble_time_latch_acknowledges_and_exposes_deterministic_native_time() {
        let mut ble = Esp32S3BleExchangeMemoryRegisters::new("ble");
        let at = SimTime::from_ticks(BLE_HALF_SLOT_TICKS * 7 + 25);
        ble.write(
            BLE_TIME_LATCH,
            AccessWidth::Word,
            u64::from(BLE_TIME_LATCH_REQUEST),
            at,
        )
        .unwrap();

        assert_eq!(ble.read(BLE_TIME_LATCH, AccessWidth::Word, at).unwrap(), 7);
        assert_eq!(
            ble.read(BLE_FINE_TIME, AccessWidth::Word, at).unwrap(),
            BLE_FINE_POSITIONS_PER_HALF_SLOT - 1 - 25 / BLE_FINE_POSITION_TICKS
        );
    }

    #[test]
    fn ble_core_version_is_native_read_only_reset_state() {
        let mut ble = Esp32S3BleExchangeMemoryRegisters::new("ble");
        assert_eq!(
            ble.read(BLE_CORE_VERSION, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(BLE_CORE_VERSION_ESP32S3)
        );
        ble.write(BLE_CORE_VERSION, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            ble.read(BLE_CORE_VERSION, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(BLE_CORE_VERSION_ESP32S3)
        );
        ble.reset(ResetKind::Software);
        assert_eq!(
            ble.read(BLE_CORE_VERSION, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(BLE_CORE_VERSION_ESP32S3)
        );
    }

    #[test]
    fn ble_core_soft_reset_strobe_self_clears() {
        let mut ble = Esp32S3BleExchangeMemoryRegisters::new("ble");
        assert_eq!(
            ble.read(BLE_RX_BUFFER_CURRENT, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(BLE_RX_BUFFER_RING_BASE)
        );
        ble.write(
            BLE_RX_BUFFER_CURRENT,
            AccessWidth::Word,
            0x1140,
            SimTime::ZERO,
        )
        .unwrap();
        ble.write(
            BLE_CORE_CONTROL,
            AccessWidth::Word,
            u64::from(BLE_CORE_SOFT_RESET | 0x100),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            ble.read(BLE_CORE_CONTROL, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x100
        );
        assert_eq!(
            ble.read(BLE_RX_BUFFER_CURRENT, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(BLE_RX_BUFFER_RING_BASE)
        );
    }

    #[test]
    fn ble_core_software_interrupt_strobe_raises_rom_scheduler_cause() {
        let mut ble = Esp32S3BleExchangeMemoryRegisters::new("ble");
        let handle = ble.handle();
        ble.write(
            BLE_CORE_CONTROL,
            AccessWidth::Word,
            u64::from(BLE_CORE_SW_INTERRUPT_REQUEST | 0x070f),
            SimTime::ZERO,
        )
        .unwrap();

        assert_eq!(
            ble.read(BLE_CORE_CONTROL, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x070f
        );
        assert!(handle.interrupt_pending());
        assert_eq!(
            ble.read(BLE_INTERRUPT_STATUS, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(BLE_SOFTWARE_INTERRUPT)
        );
        ble.write(
            BLE_INTERRUPT_CLEAR,
            AccessWidth::Word,
            u64::from(BLE_SOFTWARE_INTERRUPT),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.interrupt_pending());
    }
}
