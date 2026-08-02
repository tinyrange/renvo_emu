use super::*;

#[test]
fn esp_timer_group_schedules_and_clears_alarm_interrupts() {
    let (mut group, handle) = EspTimerGroup::new("timer-group", EspTimerGroupKind::Esp32C6);
    group
        .write(0x18, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    group
        .write(0x1c, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    group
        .write(0x20, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    group
        .write(0x10, AccessWidth::Word, 100, SimTime::ZERO)
        .unwrap();
    group
        .write(0x14, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    group
        .write(0x70, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    let config = (1 << 31) | (1 << 30) | (1 << 29) | (1 << 10) | (8 << 13);
    group
        .write(0, AccessWidth::Word, config, SimTime::ZERO)
        .unwrap();

    assert_eq!(handle.pending(SimTime::from_ticks(99)), [false, false]);
    assert_eq!(handle.pending(SimTime::from_ticks(100)), [true, false]);
    group
        .write(0x7c, AccessWidth::Word, 1, SimTime::from_ticks(100))
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(100)), [false, false]);

    group
        .write(0, AccessWidth::Word, config, SimTime::from_ticks(100))
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(200)), [true, false]);
}

#[test]
fn esp32c6_systimer_publishes_native_raw_and_status_registers() {
    let (mut timer, handle) = EspSystimer::new_esp32c6("systimer");
    timer
        .write(0x00, AccessWidth::Word, 1 << 24, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x1c, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x20, AccessWidth::Word, 10, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x64, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(9)), [false; 3]);
    assert_eq!(
        handle.pending(SimTime::from_ticks(10)),
        [true, false, false]
    );
    assert_eq!(
        timer
            .read(0x68, AccessWidth::Word, SimTime::from_ticks(10))
            .unwrap(),
        1
    );
    assert_eq!(
        timer
            .read(0x70, AccessWidth::Word, SimTime::from_ticks(10))
            .unwrap(),
        1
    );
    timer
        .write(0x6c, AccessWidth::Word, 1, SimTime::from_ticks(10))
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(10)), [false; 3]);
}

#[test]
fn esp_systimer_comparator_load_distinguishes_oneshot_and_periodic_targets() {
    let (mut timer, handle) = EspSystimer::new("systimer");

    // Target 2 mirrors the one-shot sequence used by the ESP32-S3 esp_timer HAL:
    // write an absolute deadline, apply it, then enable the comparator and interrupt.
    timer
        .write(0x2c, AccessWidth::Word, 0, SimTime::from_ticks(100))
        .unwrap();
    timer
        .write(0x30, AccessWidth::Word, 1_000, SimTime::from_ticks(100))
        .unwrap();
    timer
        .write(0x58, AccessWidth::Word, 1, SimTime::from_ticks(100))
        .unwrap();
    timer
        .write(0x00, AccessWidth::Word, 1 << 22, SimTime::from_ticks(100))
        .unwrap();
    timer
        .write(0x64, AccessWidth::Word, 1 << 2, SimTime::from_ticks(100))
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(999)), [false; 3]);
    assert_eq!(
        handle.pending(SimTime::from_ticks(1_000)),
        [false, false, true]
    );

    // A periodic target is seeded relative to the load strobe and advances by its period.
    timer
        .write(0x6c, AccessWidth::Word, 1 << 2, SimTime::from_ticks(1_000))
        .unwrap();
    timer
        .write(
            0x34,
            AccessWidth::Word,
            (1 << 30) | 25,
            SimTime::from_ticks(2_000),
        )
        .unwrap();
    timer
        .write(0x50, AccessWidth::Word, 1, SimTime::from_ticks(2_000))
        .unwrap();
    timer
        .write(0x00, AccessWidth::Word, 1 << 24, SimTime::from_ticks(2_000))
        .unwrap();
    timer
        .write(0x64, AccessWidth::Word, 1, SimTime::from_ticks(2_000))
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(2_024)), [false; 3]);
    assert_eq!(
        handle.pending(SimTime::from_ticks(2_025)),
        [true, false, false]
    );
    timer
        .write(0x6c, AccessWidth::Word, 1, SimTime::from_ticks(2_025))
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(2_049)), [false; 3]);
    assert_eq!(
        handle.pending(SimTime::from_ticks(2_050)),
        [true, false, false]
    );

    // ESP-IDF also uses load-before-mode ordering. The zero reset target must
    // not fire in the gap; setting PERIOD_MODE completes the configuration
    // and seeds the deadline from that write.
    timer
        .write(0x6c, AccessWidth::Word, 1, SimTime::from_ticks(3_000))
        .unwrap();
    timer
        .write(0x00, AccessWidth::Word, 0, SimTime::from_ticks(3_000))
        .unwrap();
    timer
        .write(0x34, AccessWidth::Word, 25, SimTime::from_ticks(3_000))
        .unwrap();
    timer
        .write(0x50, AccessWidth::Word, 1, SimTime::from_ticks(3_001))
        .unwrap();
    timer
        .write(0x00, AccessWidth::Word, 1 << 24, SimTime::from_ticks(3_002))
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(3_002)), [false; 3]);
    timer
        .write(
            0x34,
            AccessWidth::Word,
            (1 << 30) | 25,
            SimTime::from_ticks(3_010),
        )
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(3_034)), [false; 3]);
    assert_eq!(
        handle.pending(SimTime::from_ticks(3_035)),
        [true, false, false]
    );
}

#[test]
fn esp_timer_group_main_watchdog_advances_interrupt_then_reset_stage() {
    let (mut group, handle) = EspTimerGroup::new("timer-group", EspTimerGroupKind::Esp32C6);
    group
        .write(0x64, AccessWidth::Word, 0x50d8_3aa1, SimTime::ZERO)
        .unwrap();
    group
        .write(0x50, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    group
        .write(0x54, AccessWidth::Word, 4, SimTime::ZERO)
        .unwrap();
    group
        .write(0x70, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    group
        .write(
            0x48,
            AccessWidth::Word,
            (1 << 31) | (1 << 29) | (3 << 27),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        handle.take_watchdog_action(SimTime::from_ticks(3)),
        Some(EspWatchdogAction::Interrupt)
    );
    assert_eq!(handle.pending(SimTime::from_ticks(3)), [false, true]);
    assert_eq!(
        handle.take_watchdog_action(SimTime::from_ticks(7)),
        Some(EspWatchdogAction::ResetSystem)
    );
}

#[test]
fn esp32c6_main_watchdog_applies_its_source_clock_prescaler() {
    let (mut group, handle) = EspTimerGroup::new("timer-group", EspTimerGroupKind::Esp32C6);
    group
        .write(0x64, AccessWidth::Word, 0x50d8_3aa1, SimTime::ZERO)
        .unwrap();
    group
        .write(0x4c, AccessWidth::Word, 4 << 16, SimTime::ZERO)
        .unwrap();
    group
        .write(0x50, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    group
        .write(
            0x48,
            AccessWidth::Word,
            (1 << 31) | (2 << 29),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(handle.take_watchdog_action(SimTime::from_ticks(11)), None);
    assert_eq!(
        handle.take_watchdog_action(SimTime::from_ticks(12)),
        Some(EspWatchdogAction::ResetCpu)
    );
}

#[test]
fn esp32s3_timer_group_exposes_second_timer_interrupt() {
    let (mut group, handle) = EspTimerGroup::new("timer-group", EspTimerGroupKind::Esp32S3);
    group
        .write(0x3c, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    group
        .write(0x40, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    group
        .write(0x44, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    group
        .write(0x34, AccessWidth::Word, 20, SimTime::ZERO)
        .unwrap();
    group
        .write(0x38, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    group
        .write(0x70, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    group
        .write(
            0x24,
            AccessWidth::Word,
            (1 << 31) | (1 << 30) | (1 << 10) | (8 << 13),
            SimTime::ZERO,
        )
        .unwrap();

    assert_eq!(handle.pending(SimTime::from_ticks(20)), [false, true]);
}

#[test]
fn uart_captures_low_byte() {
    let (mut uart, handle) = FunctionalUart::new("uart", 0, 4, 1);
    uart.write(0, AccessWidth::Word, b'A'.into(), SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.text_lossy(), "A");
}

#[test]
fn rp_pl011_uses_named_registers_and_reset_values() {
    let (mut uart, _handle) = RpPl011Uart::new("rp.uart1");
    assert_eq!(
        RpPl011Register::from_offset(0x30),
        Some(RpPl011Register::Control)
    );
    assert_eq!(RpPl011Register::Control.offset(), 0x30);
    assert_eq!(
        uart.read(
            RpPl011Register::Control.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x300
    );
    assert_eq!(
        uart.read(
            RpPl011Register::Flags.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x90
    );
    assert_eq!(
        uart.read(
            RpPl011Register::InterruptFifoLevel.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x12
    );
    assert_eq!(
        uart.read(
            RpPl011Register::PeripheralId0.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x11
    );
    assert_eq!(
        uart.read(
            RpPl011Register::CellId3.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0xb1
    );
    assert!(RpPl011Register::from_offset(0x01c).is_none());
}

#[test]
fn rp_pl011_transmit_requires_uart_and_transmitter_enable() {
    let (mut uart, handle) = RpPl011Uart::new("rp.uart1");
    uart.write(
        RpPl011Register::Data.offset(),
        AccessWidth::Word,
        b'X'.into(),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.bytes().is_empty());

    uart.write(
        RpPl011Register::Control.offset(),
        AccessWidth::Word,
        0x301,
        SimTime::ZERO,
    )
    .unwrap();
    uart.write(
        RpPl011Register::Data.offset(),
        AccessWidth::Word,
        0x0000_0041,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.bytes(), b"A");
}

#[test]
fn rp_pl011_applies_register_masks_and_clear_semantics() {
    let (mut uart, _handle) = RpPl011Uart::new("rp.uart1");
    uart.write(
        RpPl011Register::IntegerBaud.offset(),
        AccessWidth::Word,
        u64::MAX,
        SimTime::ZERO,
    )
    .unwrap();
    uart.write(
        RpPl011Register::FractionalBaud.offset(),
        AccessWidth::Word,
        u64::MAX,
        SimTime::ZERO,
    )
    .unwrap();
    uart.write(
        RpPl011Register::InterruptMask.offset(),
        AccessWidth::Word,
        u64::MAX,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        uart.read(
            RpPl011Register::IntegerBaud.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0xffff
    );
    assert_eq!(
        uart.read(
            RpPl011Register::FractionalBaud.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x3f
    );
    assert_eq!(
        uart.read(
            RpPl011Register::InterruptMask.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x7ff
    );
    uart.write(
        RpPl011Register::Control.offset(),
        AccessWidth::Word,
        u64::MAX,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        uart.read(
            RpPl011Register::Control.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0xff87
    );
    assert!(
        uart.read(
            RpPl011Register::Data.offset(),
            AccessWidth::Byte,
            SimTime::ZERO,
        )
        .is_err()
    );
}

#[test]
fn rp_pl011_rejects_reserved_fifo_level_encodings() {
    let (mut uart, _handle) = RpPl011Uart::new("rp.uart1");
    assert!(
        uart.write(
            RpPl011Register::InterruptFifoLevel.offset(),
            AccessWidth::Word,
            0x25,
            SimTime::ZERO,
        )
        .is_err()
    );
    assert_eq!(
        uart.read(
            RpPl011Register::InterruptFifoLevel.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x12
    );

    uart.write(
        RpPl011Register::InterruptFifoLevel.offset(),
        AccessWidth::Word,
        0x24,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        uart.read(
            RpPl011Register::InterruptFifoLevel.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x24
    );
}

#[test]
fn wch_usart_requires_enable_and_preserves_configuration() {
    let (mut usart, handle) = WchUsart::new("usart1");
    assert_eq!(
        WchUsartRegister::from_offset(0x0c),
        Some(WchUsartRegister::Control1)
    );
    assert_eq!(WchUsartRegister::GuardPrescaler.offset(), 0x18);
    assert_eq!(
        usart.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xc0
    );
    usart
        .write(0x04, AccessWidth::Word, b'X'.into(), SimTime::ZERO)
        .unwrap();
    assert!(handle.bytes().is_empty());

    usart
        .write(0x08, AccessWidth::Word, 0x01a1, SimTime::ZERO)
        .unwrap();
    usart
        .write(
            0x0c,
            AccessWidth::Word,
            (1_u64 << 13) | (1_u64 << 3),
            SimTime::ZERO,
        )
        .unwrap();
    usart
        .write(0x04, AccessWidth::Word, b'A'.into(), SimTime::ZERO)
        .unwrap();

    assert_eq!(handle.bytes(), b"A");
    assert_eq!(
        usart.read(0x08, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x01a1
    );
}

#[test]
fn wch_timer_raises_and_vendor_clear_sequence_lowers_update_interrupt() {
    let (mut timer, handle) = WchTimer::new("tim2");
    timer
        .write(0x2c, AccessWidth::HalfWord, 4, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x28, AccessWidth::HalfWord, 1, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x10, AccessWidth::HalfWord, 0, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x0c, AccessWidth::HalfWord, 1, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x00, AccessWidth::HalfWord, 1, SimTime::ZERO)
        .unwrap();

    assert!(!handle.pending(SimTime::from_ticks(9)));
    assert!(handle.pending(SimTime::from_ticks(10)));
    timer
        .write(
            0x10,
            AccessWidth::HalfWord,
            u64::from(!1_u16),
            SimTime::from_ticks(10),
        )
        .unwrap();
    assert!(!handle.pending(SimTime::from_ticks(10)));
}

#[test]
fn wch_pfic_gates_pending_source_with_vendor_enable_register() {
    let (mut pfic, handle) = WchPfic::new("pfic");
    handle.set_pending(38, true);
    assert_eq!(handle.next_pending(), None);

    pfic.write(0x104, AccessWidth::Word, 1 << 6, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.next_pending(), Some(38));
    pfic.write(0x184, AccessWidth::Word, 1 << 6, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.next_pending(), None);
}

#[test]
fn esp_usb_serial_jtag_moves_deterministic_host_packets() {
    let (mut usb, handle) = EspUsbSerialJtag::new("usb-serial-jtag");
    handle.queue_input(b"x\x04");
    usb.write(0x10, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();
    assert!(handle.interrupt_pending());
    assert_eq!(
        usb.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0b110
    );
    assert_eq!(
        usb.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
        u64::from(b'x')
    );
    assert_eq!(
        usb.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
        u64::from(0x04_u8)
    );
    assert_eq!(
        usb.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0b010
    );

    for byte in b"hello" {
        usb.write(0, AccessWidth::Word, u64::from(*byte), SimTime::ZERO)
            .unwrap();
    }
    assert!(handle.output().is_empty());
    usb.write(4, AccessWidth::Word, 1, SimTime::ZERO).unwrap();
    assert_eq!(handle.output(), b"hello");
    assert!(!handle.input_complete());

    for byte in b"__REMU_HOST_SCRIPT_COMPLETE__\r\n\x04\x04>" {
        usb.write(0, AccessWidth::Word, u64::from(*byte), SimTime::ZERO)
            .unwrap();
    }
    usb.write(4, AccessWidth::Word, 1, SimTime::ZERO).unwrap();
    assert!(handle.input_complete());
}

#[test]
fn esp_usb_serial_jtag_models_host_connection_and_sof() {
    let (mut usb, handle) = EspUsbSerialJtag::new("usb-serial-jtag");
    let sof = 1_u64 << 1;
    let enabled = 0x10;
    let clear = 0x14;
    let before_first_sof = SimTime::from_ticks(EspUsbSerialJtag::SOF_PERIOD_TICKS - 1);
    let first_sof = SimTime::from_ticks(EspUsbSerialJtag::SOF_PERIOD_TICKS);

    assert!(handle.host_connected());
    usb.write(enabled, AccessWidth::Word, sof, SimTime::ZERO)
        .unwrap();
    assert!(!handle.poll(before_first_sof));
    assert_eq!(
        usb.read(0x08, AccessWidth::Word, before_first_sof).unwrap() & sof,
        0
    );
    assert!(handle.poll(first_sof));
    assert_eq!(
        usb.read(0x08, AccessWidth::Word, first_sof).unwrap() & sof,
        sof
    );
    assert!(handle.interrupt_pending());

    // SOF is a latched raw status bit and clear-on-write registers acknowledge
    // it. The next frame asserts it again after another fixed period.
    usb.write(clear, AccessWidth::Word, sof, first_sof).unwrap();
    assert!(!handle.interrupt_pending());
    assert!(!handle.poll(SimTime::from_ticks(first_sof.ticks() + 1)));
    assert!(handle.poll(SimTime::from_ticks(
        first_sof.ticks() + EspUsbSerialJtag::SOF_PERIOD_TICKS,
    )));

    handle.set_host_connected(false, first_sof);
    assert!(!handle.host_connected());
    assert_eq!(
        usb.read(0x08, AccessWidth::Word, first_sof).unwrap() & sof,
        0
    );
    assert!(!handle.poll(SimTime::from_ticks(10 * EspUsbSerialJtag::SOF_PERIOD_TICKS)));

    let reconnected = SimTime::from_ticks(20 * EspUsbSerialJtag::SOF_PERIOD_TICKS);
    handle.set_host_connected(true, reconnected);
    assert!(handle.host_connected());
    assert!(!handle.poll(SimTime::from_ticks(
        reconnected.ticks() + EspUsbSerialJtag::SOF_PERIOD_TICKS - 1,
    )));
    assert!(handle.poll(SimTime::from_ticks(
        reconnected.ticks() + EspUsbSerialJtag::SOF_PERIOD_TICKS,
    )));
}

#[test]
fn vendor_gpio_set_clear_registers_drive_signals() {
    let hub = SignalHub::new();
    let (mut sio, handle) = RpSioGpio::new("sio", 4, "board.rp.gpio", hub.clone()).unwrap();
    sio.write(0x024, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    sio.write(0x014, AccessWidth::Word, 1, SimTime::from_ticks(1))
        .unwrap();
    assert_eq!(handle.direction(), 1);
    assert_eq!(handle.output(), 1);
    let changes = hub.drain_changes();
    assert_eq!(changes.last().unwrap().value.bit(0), Some(Logic::One));
}

#[test]
fn esp32s3_gpio_bank_one_exposes_and_drives_pin_38() {
    let hub = SignalHub::new();
    let (mut gpio, handle) =
        EspGpio::new("esp32s3.gpio", 49, "board.esp32s3.chip_gpio", hub).unwrap();

    assert_eq!(handle.pin_count(), 49);
    gpio.write(0x30, AccessWidth::Word, 1 << 6, SimTime::ZERO)
        .unwrap();
    gpio.write(0x14, AccessWidth::Word, 1 << 6, SimTime::from_ticks(1))
        .unwrap();

    assert_eq!(
        gpio.read(0x2c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 6
    );
    assert_eq!(
        gpio.read(0x10, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 6
    );
    assert_eq!(handle.resolved(38).unwrap(), Logic::One);
    assert_eq!(
        gpio.read(0x40, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 6
    );

    gpio.write(0x18, AccessWidth::Word, 1 << 6, SimTime::from_ticks(2))
        .unwrap();
    assert_eq!(handle.resolved(38).unwrap(), Logic::Zero);
}

#[test]
fn rp2350_sio_uses_interleaved_low_and_high_gpio_registers() {
    let hub = SignalHub::new();
    let (mut sio, handle) =
        RpSioGpio::new_rp2350("sio", 48, "board.rp2350.gpio", hub.clone()).unwrap();
    sio.write(0x038, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    sio.write(0x018, AccessWidth::Word, 1, SimTime::from_ticks(1))
        .unwrap();
    assert_eq!(handle.direction(), 1);
    assert_eq!(handle.output(), 1);
    assert_eq!(
        sio.read(0x030, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1
    );
    assert_eq!(
        sio.read(0x010, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1
    );
    sio.write(0x020, AccessWidth::Word, 1, SimTime::from_ticks(2))
        .unwrap();
    assert_eq!(handle.output(), 0);
    assert_eq!(
        hub.drain_changes().last().unwrap().value.bit(0),
        Some(Logic::Zero)
    );

    sio.write(0x03c, AccessWidth::Word, 1 << 6, SimTime::from_ticks(3))
        .unwrap();
    sio.write(0x01c, AccessWidth::Word, 1 << 6, SimTime::from_ticks(4))
        .unwrap();
    assert_eq!(handle.direction_high(), 1 << 6);
    assert_eq!(handle.output_high(), 1 << 6);
    assert_eq!(handle.resolved(38).unwrap(), Logic::One);
    assert_eq!(
        sio.read(0x034, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 6
    );
    assert_eq!(
        sio.read(0x014, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 6
    );
}

#[test]
fn rp2350_io_bank_connects_gpio32_through_gpio47() {
    let hub = SignalHub::new();
    let (mut sio, gpio) = RpSioGpio::new_rp2350("sio", 48, "board.rp2350.io.high", hub).unwrap();
    let (mut io_bank, handle) = RpIoBank::new("io-bank0", gpio.clone(), 48);

    sio.write(0x03c, AccessWidth::Word, 1 << 15, SimTime::ZERO)
        .unwrap();
    sio.write(0x01c, AccessWidth::Word, 1 << 15, SimTime::ZERO)
        .unwrap();
    assert_ne!(
        io_bank
            .read(
                RpIoBankRegister::GpioStatus(47).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & ((1 << 9) | (1 << 13) | (1 << 17)),
        0
    );

    gpio.set_input(40, Logic::One, SimTime::from_ticks(1))
        .unwrap();
    handle.poll(SimTime::from_ticks(1)).unwrap();
    assert_ne!(
        io_bank
            .read(
                RpIoBankRegister::RawInterrupt(5).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 3),
        0
    );
}

#[test]
fn rp2350_io_bank_models_overrides_and_proc0_edge_events() {
    assert_eq!(
        RpIoBankRegister::from_offset(0x204),
        Some(RpIoBankRegister::IrqSummary {
            kind: RpIoBankSummary::Proc0Secure,
            bank: 1,
        })
    );
    assert_eq!(
        RpIoBankRegister::from_offset(0x2d4),
        Some(RpIoBankRegister::Proc1Status(5))
    );
    let hub = SignalHub::new();
    let (mut sio, gpio) =
        RpSioGpio::new_rp2350("sio", 4, "board.rp2350.io.gpio", hub.clone()).unwrap();
    let (mut io_bank, handle) = RpIoBank::new("io-bank0", gpio.clone(), 48);

    sio.write(0x038, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    sio.write(0x018, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    io_bank
        .write(
            RpIoBankRegister::GpioControl(0).offset(),
            AccessWidth::Word,
            0x3003_f01f,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::GpioControl(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
        0x3003_f01f
    );
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::GpioStatus(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & ((1 << 9) | (1 << 13)),
        (1 << 9) | (1 << 13)
    );

    gpio.set_input(2, Logic::One, SimTime::from_ticks(1))
        .unwrap();
    io_bank
        .write(
            RpIoBankRegister::Proc0Enable(0).offset(),
            AccessWidth::Word,
            1 << 11,
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert!(handle.poll(SimTime::from_ticks(1)).unwrap());
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::RawInterrupt(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 11),
        1 << 11
    );
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::Proc0Status(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 11),
        1 << 11
    );
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::IrqSummary {
                    kind: RpIoBankSummary::Proc0Secure,
                    bank: 0,
                }
                .offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 2),
        1 << 2
    );
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::RawInterrupt(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 9),
        1 << 9
    );
    io_bank
        .write(
            RpIoBankRegister::RawInterrupt(0).offset(),
            AccessWidth::Word,
            1 << 11,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::RawInterrupt(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 11),
        0
    );
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::Proc0Status(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 11),
        0
    );
}

#[test]
fn rp2350_io_bank_matches_narrow_access_and_register_reset_contract() {
    let hub = SignalHub::new();
    let (sio, gpio) = RpSioGpio::new_rp2350("sio", 4, "board.rp2350.io.contract", hub).unwrap();
    let (mut io_bank, _) = RpIoBank::new("io-bank0", gpio, 48);

    // Unbonded pins retain a register surface without manufactured levels.
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::GpioControl(47).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
        0x1f
    );
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::GpioStatus(47).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::RawInterrupt(4).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );

    // Narrow writes replicate across the register, as specified by RP2350's APB bridge.
    let enable = RpIoBankRegister::Proc0Enable(0).offset();
    io_bank
        .write(enable + 1, AccessWidth::Byte, 0xa5, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        io_bank
            .read(enable, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0xa5a5_a5a5
    );
    assert_eq!(
        io_bank
            .read(enable + 2, AccessWidth::HalfWord, SimTime::ZERO)
            .unwrap(),
        0xa5a5
    );

    // Atomic SET and CLEAR aliases apply to the native enable registers.
    io_bank
        .write(enable, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    io_bank
        .write(enable + 0x2000, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        io_bank
            .read(enable, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        3
    );
    io_bank
        .write(enable + 0x3000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        io_bank
            .read(enable, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        2
    );

    // A forced event contributes directly to INTS and IRQSUMMARY.
    io_bank
        .write(
            RpIoBankRegister::Proc0Force(0).offset(),
            AccessWidth::Word,
            1 << 3,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::Proc0Status(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & (1 << 3),
        1 << 3
    );
    assert_eq!(
        io_bank
            .read(
                RpIoBankRegister::IrqSummary {
                    kind: RpIoBankSummary::Proc0Secure,
                    bank: 0,
                }
                .offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap()
            & 1,
        1
    );

    drop(sio);
}

#[test]
fn rp2350_spi_models_primecell_fifo_loopback_and_interrupts() {
    let (mut spi, handle) = Rp2350Spi::new("spi0");
    assert_eq!(
        Rp2350SpiRegister::from_offset(0x00c),
        Some(Rp2350SpiRegister::Sr)
    );
    assert_eq!(Rp2350SpiRegister::Cr0.offset(), 0x000);
    assert_eq!(Rp2350SpiRegister::from_offset(0x028), None);
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Sr.offset(),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap(),
        0x03
    );
    spi.write(
        Rp2350SpiRegister::Cr0.offset(),
        AccessWidth::Word,
        0x07,
        SimTime::ZERO,
    )
    .unwrap();
    spi.write(
        Rp2350SpiRegister::Cr1.offset(),
        AccessWidth::Word,
        0x03,
        SimTime::ZERO,
    )
    .unwrap();
    spi.write(
        Rp2350SpiRegister::Imsc.offset(),
        AccessWidth::Word,
        1 << 3,
        SimTime::ZERO,
    )
    .unwrap();
    spi.write(
        Rp2350SpiRegister::Dr.offset(),
        AccessWidth::Word,
        0x5a,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.take_output(), vec![0x5a]);
    assert!(handle.interrupt_pending());
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Dr.offset(),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap(),
        0x5a
    );
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Sr.offset(),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap(),
        0x03
    );

    spi.write(
        Rp2350SpiRegister::Cr1.offset(),
        AccessWidth::Word,
        0x02,
        SimTime::ZERO,
    )
    .unwrap();
    handle.queue_input(&[0xa5]);
    spi.write(
        Rp2350SpiRegister::Dr.offset(),
        AccessWidth::Word,
        0x11,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Dr.offset(),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap(),
        0xa5
    );
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Mis.offset(),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap(),
        0x08
    );
}

#[test]
fn rp2350_spi_matches_apb_width_alias_and_reset_contract() {
    let (mut spi, _) = Rp2350Spi::new("spi0");

    // PrimeCell reset values and identification bytes are part of the published register
    // contract, not implementation details of the functional FIFO model.
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Sr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x03
    );
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Ris.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x08
    );
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::PeriphId0.offset(),
            AccessWidth::Byte,
            SimTime::ZERO,
        )
        .unwrap(),
        0x22
    );
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::CellId3.offset() + 3,
            AccessWidth::Byte,
            SimTime::ZERO,
        )
        .unwrap(),
        0
    );

    // RP2350 narrow writes replicate across the APB data bus. The model also preserves the
    // legal even CPSDVSR range while retaining zero as its reset value.
    spi.write(
        Rp2350SpiRegister::Cpsr.offset(),
        AccessWidth::Byte,
        3,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Cpsr.offset(),
            AccessWidth::HalfWord,
            SimTime::ZERO,
        )
        .unwrap(),
        2
    );
    spi.write(
        Rp2350SpiRegister::Cpsr.offset(),
        AccessWidth::Word,
        0xff,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Cpsr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0xfe
    );

    // The RP atomic aliases are available on the mapped SPI APB window. IMSC is an ordinary
    // read/write mask, so SET and CLEAR aliases operate on its current value.
    spi.write(
        Rp2350SpiRegister::Imsc.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    spi.write(
        0x2000 + Rp2350SpiRegister::Imsc.offset(),
        AccessWidth::Byte,
        2,
        SimTime::ZERO,
    )
    .unwrap();
    spi.write(
        0x3000 + Rp2350SpiRegister::Imsc.offset(),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Imsc.offset(),
            AccessWidth::Byte,
            SimTime::ZERO,
        )
        .unwrap(),
        2
    );
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Imsc.offset() + 1,
            AccessWidth::Byte,
            SimTime::ZERO,
        )
        .unwrap(),
        0
    );

    // MS is only writable while the SSP is disabled. Atomic writes retain that restriction.
    spi.write(
        Rp2350SpiRegister::Cr1.offset(),
        AccessWidth::Word,
        0x02,
        SimTime::ZERO,
    )
    .unwrap();
    spi.write(
        0x2000 + Rp2350SpiRegister::Cr1.offset(),
        AccessWidth::Word,
        0x04,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Cr1.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x02
    );
    spi.write(
        Rp2350SpiRegister::Cr1.offset(),
        AccessWidth::Word,
        0,
        SimTime::ZERO,
    )
    .unwrap();
    spi.write(
        Rp2350SpiRegister::Cr1.offset(),
        AccessWidth::Word,
        0x04,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        spi.read(
            Rp2350SpiRegister::Cr1.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x04
    );
}
