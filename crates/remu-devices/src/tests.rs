use super::*;

#[test]
fn gpio_emits_resolved_changes_and_contention() {
    let hub = SignalHub::new();
    let (mut gpio, handle) =
        FunctionalGpio::new("gpio", 2, "board.gpio", hub.clone(), 0, 4, 8).unwrap();
    gpio.write(0, AccessWidth::Word, 1, SimTime::ZERO).unwrap();
    gpio.write(4, AccessWidth::Word, 1, SimTime::from_ticks(1))
        .unwrap();
    handle
        .set_input(0, Logic::Zero, SimTime::from_ticks(2))
        .unwrap();
    let changes = hub.drain_changes();
    assert_eq!(changes.len(), 3);
    assert_eq!(changes[0].value.bit(0), Some(Logic::Zero));
    assert_eq!(changes[1].value.bit(0), Some(Logic::One));
    assert_eq!(changes[2].value.bit(0), Some(Logic::X));
}

#[test]
fn timer_latches_and_clears_interrupt() {
    let (mut timer, handle) = FunctionalTimer::new("timer");
    timer
        .write(
            FunctionalTimer::COMPARE,
            AccessWidth::DoubleWord,
            10,
            SimTime::ZERO,
        )
        .unwrap();
    timer
        .write(
            FunctionalTimer::CONTROL,
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
    assert!(!handle.poll(SimTime::from_ticks(9)));
    assert!(handle.poll(SimTime::from_ticks(10)));
    timer
        .write(
            FunctionalTimer::STATUS,
            AccessWidth::Word,
            1,
            SimTime::from_ticks(10),
        )
        .unwrap();
    assert!(!handle.pending());
}

#[test]
fn rp_timer_interrupt_aliases_accumulate_and_clear_bits() {
    let (mut timer, handle) = Rp2040Timer::new("timer", RpTimerLayout::Rp2040);
    timer
        .write(0x2038, AccessWidth::Word, 0x8, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x2038, AccessWidth::Word, 0x4, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        timer.read(0x38, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xc
    );

    timer
        .write(0x203c, AccessWidth::Word, 0x8, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.pending(SimTime::ZERO), 0x8);
    timer
        .write(0x303c, AccessWidth::Word, 0x8, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.pending(SimTime::ZERO), 0);
}

#[test]
fn rp2350_timer_uses_shifted_interrupt_registers() {
    let (mut timer, handle) = Rp2040Timer::new("timer", RpTimerLayout::Rp2350);
    timer
        .write(0x2040, AccessWidth::Word, 0x8, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x2040, AccessWidth::Word, 0x4, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        timer.read(0x40, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xc
    );

    timer
        .write(0x1c, AccessWidth::Word, 10, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(10)), 0x8);
    timer
        .write(0x3c, AccessWidth::Word, 0x8, SimTime::from_ticks(10))
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(10)), 0);

    timer
        .write(0x2044, AccessWidth::Word, 0x8, SimTime::from_ticks(10))
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(10)), 0x8);
    timer
        .write(0x3044, AccessWidth::Word, 0x8, SimTime::from_ticks(10))
        .unwrap();
    assert_eq!(handle.pending(SimTime::from_ticks(10)), 0);
}

#[test]
fn rp_pio_executes_set_pin_program_on_abstract_ticks() {
    let hub = SignalHub::new();
    let (mut pio, handle) = RpPio::new("pio0", 32, "board.rp.pio0.gpio", hub.clone()).unwrap();
    pio.write(
        0x0dc,
        AccessWidth::Word,
        (1_u64 << 26) | (25_u64 << 5),
        SimTime::ZERO,
    )
    .unwrap();
    pio.write(0x0cc, AccessWidth::Word, 1 << 12, SimTime::ZERO)
        .unwrap();
    pio.write(0x048, AccessWidth::Word, 0xe001, SimTime::ZERO)
        .unwrap();
    pio.write(0x04c, AccessWidth::Word, 0xe000, SimTime::ZERO)
        .unwrap();
    pio.write(0x000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();

    assert!(handle.poll(SimTime::from_ticks(1)).unwrap());
    assert!(handle.poll(SimTime::from_ticks(2)).unwrap());
    let changes = hub.drain_changes();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].value.bit(25), Some(Logic::One));
    assert_eq!(changes[1].value.bit(25), Some(Logic::Zero));
}

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
fn esp_i2c_fifo_exposes_master_transaction_and_completion_status() {
    let (mut i2c, handle) = EspI2c::new("i2c0");
    handle.queue_rx(&[0x34, 0x12]);
    i2c.write(0x1c, AccessWidth::Word, 0xa0, SimTime::ZERO)
        .unwrap();
    i2c.write(0x1c, AccessWidth::Word, 0x00, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.take_tx(), [0xa0, 0x00]);
    assert_eq!(
        i2c.read(0x1c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x34
    );
    assert_eq!(
        i2c.read(0x1c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x12
    );
    // Restart, write two bytes, stop, and end. Command fields use the native
    // ESP32-C6 byte-count/opcode encoding.
    i2c.write(0x58, AccessWidth::Word, 6 << 11, SimTime::ZERO)
        .unwrap();
    i2c.write(0x5c, AccessWidth::Word, 2 | (1 << 11), SimTime::ZERO)
        .unwrap();
    i2c.write(0x60, AccessWidth::Word, 2 << 11, SimTime::ZERO)
        .unwrap();
    i2c.write(0x64, AccessWidth::Word, 4 << 11, SimTime::ZERO)
        .unwrap();
    i2c.write(0x04, AccessWidth::Word, (1 << 4) | (1 << 5), SimTime::ZERO)
        .unwrap();
    assert!(!handle.busy());
    i2c.write(0x28, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        i2c.read(0x2c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 7
    );
    i2c.write(0x24, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    assert_eq!(i2c.read(0x2c, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
}

#[test]
fn uart_captures_low_byte() {
    let (mut uart, handle) = FunctionalUart::new("uart", 0, 4, 1);
    uart.write(0, AccessWidth::Word, b'A'.into(), SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.text_lossy(), "A");
}

#[test]
fn wch_usart_requires_enable_and_preserves_configuration() {
    let (mut usart, handle) = WchUsart::new("usart1");
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
fn rp2350_sio_uses_interleaved_low_and_high_gpio_registers() {
    let hub = SignalHub::new();
    let (mut sio, handle) =
        RpSioGpio::new_rp2350("sio", 4, "board.rp2350.gpio", hub.clone()).unwrap();
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
}

#[test]
fn rp_sio_echoes_bootrom_launch_and_routes_live_fifo_words() {
    let hub = SignalHub::new();
    let (mut sio, _, multicore) =
        RpSioGpio::new_with_multicore("sio", 4, "board.rp.gpio", hub).unwrap();

    // Initial core-1 ROM ready acknowledgement.
    assert_eq!(
        sio.read(0x050, AccessWidth::Word, SimTime::ZERO).unwrap() & 1,
        1
    );
    assert_eq!(
        sio.read(0x058, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );

    let sequence: [u32; 6] = [0, 0, 1, 0x1000_0000, 0x2004_0000, 0x1000_0101];
    for word in sequence {
        sio.write(0x054, AccessWidth::Word, u64::from(word), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            sio.read(0x058, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(word)
        );
    }
    assert_eq!(
        multicore.take_core1_launch(),
        Some(RpCoreLaunch {
            vector_table: 0x1000_0000,
            stack_pointer: 0x2004_0000,
            entry: 0x1000_0101,
        })
    );

    multicore.select_core(1);
    assert_eq!(sio.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
    sio.write(0x054, AccessWidth::Word, 0xfeed_beef, SimTime::ZERO)
        .unwrap();
    multicore.select_core(0);
    assert_eq!(
        sio.read(0x058, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xfeed_beef
    );
}

#[test]
fn rp2040_resets_supports_set_and_clear_aliases() {
    let mut resets = Rp2040Resets::new("resets");
    resets
        .write(0x2000, AccessWidth::Word, 0x21, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        resets.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x21
    );
    assert_eq!(
        resets.read(8, AccessWidth::Word, SimTime::ZERO).unwrap(),
        u64::from(Rp2040Resets::VALID_MASK & !0x21)
    );
    resets
        .write(0x3000, AccessWidth::Word, 0x20, SimTime::ZERO)
        .unwrap();
    assert_eq!(resets.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
}

#[test]
fn deterministic_rng_changes_words_and_restarts_from_its_seed() {
    let mut rng = DeterministicRng::new("rng", 0x7c, 0x1234_5678);
    let first = rng.read(0x7c, AccessWidth::Word, SimTime::ZERO).unwrap();
    let second = rng.read(0x7c, AccessWidth::Word, SimTime::ZERO).unwrap();
    assert_ne!(first, second);
    rng.reset(ResetKind::PowerOn);
    assert_eq!(
        rng.read(0x7c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        first
    );
}

#[test]
fn rp2040_usb_exposes_vbus_and_atomic_aliases() {
    let mut usb = Rp2040UsbController::new("usb");
    assert_eq!(
        usb.read(0x50, AccessWidth::Word, SimTime::ZERO).unwrap() & 1,
        1
    );
    usb.write(0x204c, AccessWidth::Word, 0x10, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        usb.read(0x4c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x10
    );
}

#[test]
fn rp_sio_spinlocks_claim_on_read_and_release_on_write() {
    let hub = SignalHub::new();
    let (mut sio, _) = RpSioGpio::new("sio", 4, "board.rp.gpio", hub).unwrap();
    assert_eq!(
        sio.read(0x12c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 11
    );
    assert_eq!(
        sio.read(0x12c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
    sio.write(0x12c, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        sio.read(0x12c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 11
    );
}

#[test]
fn arm_ppb_models_all_nvic_banks_and_software_trigger() {
    let (mut ppb, handle) = ArmPrivatePeripheralBus::new("ppb", 0x410f_d210);
    ppb.write(0x11c, AccessWidth::Word, 1 << 15, SimTime::ZERO)
        .unwrap();
    ppb.write(0xf00, AccessWidth::Word, 239, SimTime::ZERO)
        .unwrap();

    assert!(handle.interrupt_enabled(239));
    assert_eq!(handle.take_pending_interrupts(), vec![239]);
    assert!(handle.take_pending_interrupts().is_empty());
}

#[test]
fn arm_ppb_systick_latches_a_deterministic_exception() {
    let (mut ppb, handle) = ArmPrivatePeripheralBus::new("ppb", 0x410c_c601);
    ppb.write(0x014, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    ppb.write(0x018, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    ppb.write(0x010, AccessWidth::Word, 7, SimTime::ZERO)
        .unwrap();

    assert!(!handle.take_systick_pending(SimTime::from_ticks(2)));
    assert!(handle.take_systick_pending(SimTime::from_ticks(3)));
    assert!(!handle.take_systick_pending(SimTime::from_ticks(3)));
    assert_ne!(
        ppb.read(0x010, AccessWidth::Word, SimTime::from_ticks(3))
            .unwrap()
            & (1 << 16),
        0
    );
}
