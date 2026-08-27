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
fn spi_captures_transmit_and_provides_deterministic_loopback() {
    let (mut spi, handle) = FunctionalSpi::new("spi");
    spi.write(
        Rp2040SpiRegister::SsiEnr.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    spi.write(
        Rp2040SpiRegister::Ser.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Sr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x06
    );
    spi.write(
        Rp2040SpiRegister::Data(0).offset(),
        AccessWidth::Word,
        0xa5,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.transmitted(), [0xa5]);
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Sr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x0e
    );
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Data(0).offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0xa5
    );
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Sr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x06
    );
}

#[test]
fn spi_consumes_queued_host_bytes_before_loopback() {
    let (mut spi, handle) = FunctionalSpi::new("spi");
    spi.write(
        Rp2040SpiRegister::SsiEnr.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    spi.write(
        Rp2040SpiRegister::Ser.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    handle.queue_received(&[0x5a]);
    spi.write(
        Rp2040SpiRegister::Data(0).offset(),
        AccessWidth::Word,
        0x33,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Data(0).offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x5a
    );
}

#[test]
fn rp2040_spi_uses_native_dw_ssi_offsets_masks_and_identifiers() {
    assert_eq!(
        Rp2040SpiRegister::from_offset(0x28),
        Some(Rp2040SpiRegister::Sr)
    );
    assert_eq!(
        Rp2040SpiRegister::from_offset(0x60),
        Some(Rp2040SpiRegister::Data(0))
    );
    assert_eq!(
        Rp2040SpiRegister::from_offset(0xec),
        Some(Rp2040SpiRegister::Data(35))
    );
    assert_eq!(Rp2040SpiRegister::from_offset(0x08 + 1), None);
    assert_eq!(Rp2040SpiRegister::Idr.offset(), 0x58);
    let (mut spi, _) = FunctionalSpi::new("spi");
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Idr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x5153_5049
    );
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::SsiVersionId.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x3430_312a
    );
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::SpiCtrlr0.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0x0300_0000
    );
}

#[test]
fn rp2040_spi_latches_fifo_faults_and_clears_fifos_when_disabled() {
    let (mut spi, _) = FunctionalSpi::new("spi");
    // Keep the slave deselected so writes remain in the native sixteen-entry TX FIFO.
    spi.write(
        Rp2040SpiRegister::SsiEnr.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    for value in 0..16 {
        spi.write(
            Rp2040SpiRegister::Data(0).offset(),
            AccessWidth::Word,
            value,
            SimTime::ZERO,
        )
        .unwrap();
    }
    spi.write(
        Rp2040SpiRegister::Data(0).offset(),
        AccessWidth::Word,
        0xff,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Txflr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        16
    );
    assert_ne!(
        spi.read(
            Rp2040SpiRegister::Risr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap()
            & (1 << 1),
        0
    );
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Txoicr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        1
    );
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Data(0).offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0
    );
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Rxuicr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        1
    );

    spi.write(
        Rp2040SpiRegister::SsiEnr.offset(),
        AccessWidth::Word,
        0,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Txflr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0
    );
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Rxflr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0
    );
}

#[test]
fn rp2040_spi_requires_enable_and_reports_fifo_interrupts() {
    let (mut spi, handle) = FunctionalSpi::new("spi");
    spi.write(
        Rp2040SpiRegister::Data(0).offset(),
        AccessWidth::Word,
        0x12,
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.transmitted().is_empty());
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Txflr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        1
    );
    spi.write(
        Rp2040SpiRegister::Ser.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    spi.write(
        Rp2040SpiRegister::SsiEnr.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.transmitted(), [0x12]);
    spi.write(
        Rp2040SpiRegister::Imr.offset(),
        AccessWidth::Word,
        1 << 4,
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.interrupt_pending());
    assert_eq!(
        spi.read(
            Rp2040SpiRegister::Rxflr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        1
    );
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
fn rp2350_trng_generates_deterministic_words_and_interrupts() {
    let (mut trng, handle) = Rp2350Trng::new("trng");
    assert_eq!(
        trng.read(0x100, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xf
    );
    assert_eq!(
        trng.read(0x130, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xffff
    );
    trng.write(0x12c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert!(handle.result_ready());
    assert!(!handle.interrupt_pending());
    trng.write(0x100, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    assert!(handle.interrupt_pending());
    let first = trng.read(0x114, AccessWidth::Word, SimTime::ZERO).unwrap();
    let last = trng.read(0x128, AccessWidth::Word, SimTime::ZERO).unwrap();
    assert_ne!(first, last);
    assert!(!handle.result_ready());
    trng.write(0x12c, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    trng.write(0x12c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    let repeated = trng.read(0x114, AccessWidth::Word, SimTime::ZERO).unwrap();
    assert_ne!(first, repeated);
    assert!(
        trng.write(0x1e0, AccessWidth::Word, 1, SimTime::ZERO)
            .is_err()
    );

    trng.write(0x12c, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    trng.write(0x12c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    trng.write(0x108, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        trng.read(0x110, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
    assert_eq!(
        trng.read(0x114, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
}

#[test]
fn rp2350_sha256_accepts_padded_byte_stream_and_matches_fips_digest() {
    let mut sha = Rp2350Sha256::new("sha256");
    assert_eq!(
        sha.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x1206
    );
    assert_eq!(sha.read(0x08, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
    assert!(
        sha.write(0x08, AccessWidth::Word, 1, SimTime::ZERO)
            .is_err()
    );
    sha.write(0x00, AccessWidth::Word, 0x1201, SimTime::ZERO)
        .unwrap();
    let mut padded = [0_u8; 64];
    padded[..3].copy_from_slice(b"abc");
    padded[3] = 0x80;
    padded[63] = 24;
    for byte in padded {
        sha.write(0x04, AccessWidth::Byte, u64::from(byte), SimTime::ZERO)
            .unwrap();
    }
    let expected: [u32; 8] = [
        0xba78_16bf,
        0x8f01_cfea,
        0x4141_40de,
        0x5dae_2223,
        0xb003_61a3,
        0x9617_7a9c,
        0xb410_ff61,
        0xf200_15ad,
    ];
    for (index, word) in expected.into_iter().enumerate() {
        assert_eq!(
            sha.read(
                0x08 + u64::try_from(index * 4).expect("SHA result offset fits"),
                AccessWidth::Word,
                SimTime::ZERO,
            )
            .unwrap(),
            u64::from(word)
        );
    }
    assert_ne!(
        sha.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 2),
        0
    );
}

#[test]
fn rp2350_hstx_serializes_fifo_words_and_reports_overflow() {
    let hub = SignalHub::new();
    let (mut ctrl, mut fifo, handle) =
        new_rp2350_hstx("rp2350.hstx", "board.rp2350.hstx", hub.clone()).unwrap();
    assert_eq!(
        Rp2350HstxControlRegister::try_from(0x20).unwrap(),
        Rp2350HstxControlRegister::Bit7
    );
    assert!(Rp2350HstxControlRegister::try_from(0x2c).is_err());
    assert_eq!(
        Rp2350HstxFifoRegister::try_from(0x04).unwrap(),
        Rp2350HstxFifoRegister::Fifo
    );
    assert_eq!(
        ctrl.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x1005_0600
    );
    ctrl.write(0x04, AccessWidth::Word, 1 | (1 << 8), SimTime::ZERO)
        .unwrap();
    for lane in 1..8 {
        ctrl.write(
            0x04 + lane * 4,
            AccessWidth::Word,
            2 | (2 << 8),
            SimTime::ZERO,
        )
        .unwrap();
    }
    ctrl.write(0, AccessWidth::Word, 0x1005_0601, SimTime::ZERO)
        .unwrap();
    fifo.write(0x04, AccessWidth::Word, 0b11, SimTime::from_ticks(3))
        .unwrap();
    assert_eq!(
        handle.samples(),
        vec![HstxSample {
            word: 0b11,
            positive: [true, false, false, false, false, false, false, false],
            negative: [true, false, false, false, false, false, false, false],
            clock: false,
        }]
    );
    assert_eq!(
        fifo.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 9
    );
    assert!(fifo.read(0x04, AccessWidth::Word, SimTime::ZERO).is_err());

    ctrl.write(0, AccessWidth::Word, 0, SimTime::ZERO).unwrap();
    for word in 0..9 {
        fifo.write(0x04, AccessWidth::Word, word, SimTime::ZERO)
            .unwrap();
    }
    assert_eq!(
        fifo.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
        8 | (1 << 8) | (1 << 10)
    );
    fifo.write(0, AccessWidth::Word, 1 << 10, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        fifo.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
        8 | (1 << 8)
    );
}

#[test]
fn rp2350_otp_exposes_read_aliases_and_monotonic_locks() {
    let mut otp = Rp2350Otp::with_words("otp", &[0x00c0_ffee, 0x0012_3456]);
    assert_eq!(
        Rp2350OtpRegister::try_from(0x100).unwrap(),
        Rp2350OtpRegister::SbpiInstr
    );
    assert!(Rp2350OtpRegister::try_from(0x130).is_err());
    for offset in [0x10000, 0x14000, 0x1c000] {
        assert_eq!(
            otp.read(offset, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x00c0_ffee
        );
    }
    otp.write(0x000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    otp.write(0x3000, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    assert_eq!(otp.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 3);
    otp.write(0x3000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(otp.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 3);

    otp.write(0x128, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    assert!(otp.read(0x14000, AccessWidth::Word, SimTime::ZERO).is_err());
    otp.write(0x2128, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        otp.read(0x1c000, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x00c0_ffee
    );
    otp.write(0x158, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        otp.read(0x15c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        2
    );
    otp.write(0x154, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    otp.write(0x3154, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        otp.read(0x154, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1
    );
    otp.write(0x100, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        otp.read(0x100, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 30),
        0
    );
    assert_eq!(
        otp.read(0x124, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 4),
        1 << 4
    );
    otp.write(0x124, AccessWidth::Word, 1 << 4, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        otp.read(0x124, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 4),
        0
    );
}

#[test]
fn rp2350_accessctrl_tracks_masks_locks_and_configuration_reset() {
    let (mut access, handle) = Rp2350AccessCtrl::new_with_handle("accessctrl");
    assert_eq!(
        Rp2350AccessCtrlRegister::try_from(0x14).unwrap(),
        Rp2350AccessCtrlRegister::Peripheral(0)
    );
    assert!(Rp2350AccessCtrlRegister::try_from(0x11).is_err());
    assert_eq!(access.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 4);
    assert_eq!(access.permission(0x14), Some(0xff));
    assert_eq!(access.gpio_nonsecure_masks(), (0, 0));
    access
        .write(0x14, AccessWidth::Word, 0x55, SimTime::ZERO)
        .unwrap();
    access
        .write(0x2014, AccessWidth::Word, 0x0f, SimTime::ZERO)
        .unwrap();
    access
        .write(0x3014, AccessWidth::Word, 0x0f, SimTime::ZERO)
        .unwrap();
    assert_eq!(access.permission(0x14), Some(0x50));
    access
        .write(0x0c, AccessWidth::Word, 0xa5, SimTime::ZERO)
        .unwrap();
    assert_eq!(access.gpio_nonsecure_masks().0, 0xa5);
    access
        .write(0x04, AccessWidth::Word, 0x03, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        access.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
        2
    );
    access
        .write(0, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    access
        .write(0x3000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(access.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 5);
    handle.set_context(Rp2350AccessMaster::Debugger, true, true);
    access
        .write(8, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(access.permission(0x14), Some(0xff));
    assert_eq!(access.gpio_nonsecure_masks(), (0, 0));
    assert_eq!(
        access.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
        2
    );
    assert_eq!(access.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 5);
}

#[test]
fn rp2350_accessctrl_enforces_master_security_and_privilege() {
    let (mut access, handle) = Rp2350AccessCtrl::new_with_handle("accessctrl");
    // UART0: core 0, Secure Privileged only.
    access
        .write(0xa0, AccessWidth::Word, 0x18, SimTime::ZERO)
        .unwrap();
    assert!(handle.check_address(0x4007_0000).is_ok());
    handle.set_context(Rp2350AccessMaster::Core0, false, true);
    assert!(handle.check_address(0x4007_0000).is_err());
    handle.set_context(Rp2350AccessMaster::Core1, true, true);
    assert!(handle.check_address(0x4007_0000).is_err());

    // Secure privileged firmware may delegate NSP, after which NSP may only
    // grant/revoke the subordinate NSU bit.
    handle.set_context(Rp2350AccessMaster::Debugger, true, true);
    access
        .write(0xa0, AccessWidth::Word, 0x1a, SimTime::ZERO)
        .unwrap();
    handle.set_context(Rp2350AccessMaster::Core0, false, true);
    access
        .write(0x20a0, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(access.permission(0xa0), Some(0x1b));
    handle.set_context(Rp2350AccessMaster::Core0, false, false);
    assert!(handle.check_address(0x4007_0000).is_ok());

    handle.set_context(Rp2350AccessMaster::Debugger, true, true);
    access
        .write(0x0c, AccessWidth::Word, 1 << 12, SimTime::ZERO)
        .unwrap();
    assert!(handle.gpio_is_nonsecure(12));
    assert!(!handle.gpio_is_nonsecure(13));
}

#[test]
fn rp2350_ticks_expose_independent_running_and_countdown_state() {
    let mut ticks = Rp2350Ticks::new("ticks");
    assert_eq!(
        Rp2350TicksRegister::try_from(0x44).unwrap(),
        Rp2350TicksRegister::RiscvCount
    );
    assert!(Rp2350TicksRegister::try_from(0x48).is_err());
    assert_eq!(ticks.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
    ticks
        .write(4, AccessWidth::Word, 4, SimTime::from_ticks(10))
        .unwrap();
    ticks
        .write(
            0,
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::from_ticks(10),
        )
        .unwrap();
    assert_eq!(
        ticks
            .read(0, AccessWidth::Word, SimTime::from_ticks(10))
            .unwrap(),
        3
    );
    ticks
        .write(0, AccessWidth::Word, 1, SimTime::from_ticks(10))
        .unwrap();
    assert_eq!(
        ticks
            .read(0, AccessWidth::Word, SimTime::from_ticks(10))
            .unwrap(),
        3
    );
    assert_eq!(
        ticks
            .read(8, AccessWidth::Word, SimTime::from_ticks(12))
            .unwrap(),
        2
    );
    assert_eq!(ticks.countdown(0, SimTime::from_ticks(14)), Some(4));
    ticks
        .write(0x3000, AccessWidth::Word, 1, SimTime::from_ticks(15))
        .unwrap();
    assert_eq!(
        ticks
            .read(0, AccessWidth::Word, SimTime::from_ticks(15))
            .unwrap(),
        0
    );
    ticks
        .write(0x40, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    ticks
        .write(0x3c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(ticks.is_running(5, SimTime::ZERO), Some(true));
    assert!(
        ticks
            .write(0x08, AccessWidth::Word, 1, SimTime::ZERO)
            .is_err()
    );
}

#[test]
fn rp2350_powman_keeps_scratch_and_runs_aon_timer_and_wake_configuration() {
    let mut powman = Rp2350Powman::new("powman");
    assert_eq!(
        Rp2350PowmanRegister::try_from(0xec).unwrap(),
        Rp2350PowmanRegister::Ints
    );
    assert!(Rp2350PowmanRegister::try_from(0xee).is_err());
    assert_eq!(
        powman.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x8050
    );
    assert_eq!(
        powman.read(0x38, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xf
    );
    powman
        .write(0xb0, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
        .unwrap();
    powman
        .write(0xd0, AccessWidth::Word, 0xfeed_cafe, SimTime::ZERO)
        .unwrap();
    powman
        .write(0x6c, AccessWidth::Word, 100, SimTime::ZERO)
        .unwrap();
    powman
        .write(0x84, AccessWidth::Word, 110, SimTime::ZERO)
        .unwrap();
    powman
        .write(
            0x88,
            AccessWidth::Word,
            (1 << 8) | (1 << 4) | (1 << 1),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(powman.aon_time(SimTime::from_ticks(7)), 107);
    assert!(!powman.alarm_pending(SimTime::from_ticks(7)));
    assert!(powman.alarm_pending(SimTime::from_ticks(10)));
    powman
        .write(0xe4, AccessWidth::Word, 1 << 1, SimTime::from_ticks(10))
        .unwrap();
    assert_ne!(
        powman
            .read(0xec, AccessWidth::Word, SimTime::from_ticks(10))
            .unwrap()
            & (1 << 1),
        0
    );
    powman.reset(ResetKind::Watchdog);
    assert_eq!(
        powman.read(0xb0, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x1234_5678
    );
    assert_eq!(
        powman.read(0xd0, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xfeed_cafe
    );
}

#[test]
fn rp2350_powman_timer_commands_are_self_clearing_and_set_time_is_gated() {
    let mut powman = Rp2350Powman::new("powman");
    powman
        .write(0x6c, AccessWidth::Word, 25, SimTime::ZERO)
        .unwrap();
    powman
        .write(0x88, AccessWidth::Word, 1 << 1, SimTime::ZERO)
        .unwrap();
    powman
        .write(0x6c, AccessWidth::Word, 99, SimTime::from_ticks(5))
        .unwrap();
    assert_eq!(powman.aon_time(SimTime::from_ticks(5)), 30);
    powman
        .write(0x88, AccessWidth::Word, 1 << 2, SimTime::from_ticks(5))
        .unwrap();
    assert_eq!(powman.aon_time(SimTime::from_ticks(6)), 1);
    assert_eq!(
        powman
            .read(0x88, AccessWidth::Word, SimTime::from_ticks(6))
            .unwrap()
            & (1 << 2),
        0
    );
}

#[test]
fn rp2350_powman_writes_respect_read_only_and_write_clear_fields() {
    let mut powman = Rp2350Powman::new("powman");
    for (offset, expected) in [(0x0c, 0x1f2), (0x8c, 0x1ff), (0x38, 0xff), (0xa8, 2)] {
        powman
            .write(
                offset,
                AccessWidth::Word,
                u64::from(u32::MAX),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            powman
                .read(offset, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            expected
        );
    }
    powman
        .write(0xe0, AccessWidth::Word, u64::from(u32::MAX), SimTime::ZERO)
        .unwrap();
    assert_eq!(
        powman.read(0xe0, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
}

#[test]
fn rp2040_psm_exposes_power_state_masks_and_atomic_aliases() {
    let mut psm = Rp2040Psm::new("psm");
    assert_eq!(
        psm.read(0x0c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x1ffff
    );
    psm.write(0x04, AccessWidth::Word, 1 << 3, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        psm.read(0x0c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x1fff7
    );
    psm.write(0x2000, AccessWidth::Word, 1 << 3, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        psm.read(0x0c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x1ffff
    );
    assert_eq!(
        psm.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 3
    );
    assert!(
        psm.write(0x0c, AccessWidth::Word, 0, SimTime::ZERO)
            .is_err()
    );
}

#[test]
fn rp2040_rosc_models_protected_controls_dormant_and_count() {
    let mut rosc = Rp2040Rosc::new("rosc");
    assert_eq!(
        rosc.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xaa0
    );
    assert_eq!(
        rosc.read(0x18, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x8000_1000
    );
    assert_eq!(
        rosc.read(0x10, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xab0
    );
    assert_eq!(
        rosc.read(0x1c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1
    );
    rosc.write(
        0x00,
        AccessWidth::Word,
        (0xd1e_u64 << 12) | 0xfa4,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        rosc.read(0x18, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
    rosc.write(
        0x00,
        AccessWidth::Word,
        (0xfab_u64 << 12) | 0xfa5,
        SimTime::ZERO,
    )
    .unwrap();
    rosc.write(0x20, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        rosc.read(0x20, AccessWidth::Word, SimTime::from_ticks(2))
            .unwrap(),
        1
    );
    rosc.write(0x0c, AccessWidth::Word, 0x636f_6d61, SimTime::from_ticks(2))
        .unwrap();
    assert_eq!(
        rosc.read(0x1c, AccessWidth::Word, SimTime::from_ticks(3))
            .unwrap(),
        1
    );
    rosc.write(0x0c, AccessWidth::Word, 0x7761_6b65, SimTime::from_ticks(3))
        .unwrap();
    assert_eq!(
        rosc.read(0x18, AccessWidth::Word, SimTime::from_ticks(3))
            .unwrap()
            & 0x8000_1000,
        0x8000_1000
    );
    rosc.write(0x10, AccessWidth::Word, 0xaa0, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        rosc.read(0x10, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xaa0
    );
    rosc.write(0x10, AccessWidth::Word, 0xabf, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        rosc.read(0x10, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xabf
    );
    rosc.write(0x10, AccessWidth::Word, 0x123, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        rosc.read(0x10, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xabf
    );
    assert_ne!(
        rosc.read(0x18, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 24),
        0
    );
    rosc.write(0x18, AccessWidth::Word, 1 << 24, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        rosc.read(0x18, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 24),
        0
    );
}

#[test]
fn rp2040_vreg_reports_regulation_and_bod_configuration() {
    let mut power = Rp2040VregAndChipReset::new("vreg");
    assert_eq!(
        power.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x10b1
    );
    power
        .write(0x2000, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        power.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xb3
    );
    power
        .write(0x3000, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        power.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x10b1
    );
    power
        .write(0x04, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        power.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
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
fn rp_pio_named_registers_model_fifo_levels_and_irq_masks() {
    let hub = SignalHub::new();
    let (mut pio, handle) =
        RpPio::new_with_version("pio1", 32, "board.rp.pio1.gpio", hub, RpPioVersion::Rp2350)
            .unwrap();
    let version = RpPioVersion::Rp2350;
    let offset = |register: RpPioRegister| register.offset_for_version(version);

    assert_eq!(RpPioRegister::Txf(2).offset(), 0x18);
    assert_eq!(
        RpPioRegister::try_from_offset_for_version(0x0d4, version),
        Ok(RpPioRegister::StateMachine {
            machine: 0,
            register: RpPioStateMachineRegister::Addr,
        })
    );
    assert_eq!(offset(RpPioRegister::Intr), 0x16c);
    assert_eq!(offset(RpPioRegister::Irq0Inte), 0x170);
    assert_eq!(offset(RpPioRegister::Irq1Ints), 0x184);
    assert_eq!(
        RpPioRegister::try_from_offset_for_version(0x12c, version),
        Ok(RpPioRegister::RxfPutGet {
            machine: 0,
            entry: 1
        })
    );
    assert_eq!(
        pio.read(0x044, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x1020_0404
    );
    assert_eq!(
        pio.read(0x004, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x0f00_0f00
    );

    // FJOIN_RX_GET exposes the four random-access RX words as processor writes.
    pio.write(0x0d0, AccessWidth::Word, 1 << 14, SimTime::ZERO)
        .unwrap();
    pio.write(0x128, AccessWidth::Word, 0xa5a5_5a5a, SimTime::ZERO)
        .unwrap();
    assert!(pio.read(0x128, AccessWidth::Word, SimTime::ZERO).is_err());
    // FJOIN_RX_PUT reverses the processor direction and resets the storage.
    pio.write(0x0d0, AccessWidth::Word, 1 << 15, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        pio.read(0x128, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
    assert!(
        pio.write(0x128, AccessWidth::Word, 1, SimTime::ZERO)
            .is_err()
    );
    pio.write(0x0d0, AccessWidth::Word, 0x000c_0000, SimTime::ZERO)
        .unwrap();

    for value in 0..4 {
        pio.write(0x010, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    assert_eq!(
        pio.read(0x00c, AccessWidth::Word, SimTime::ZERO).unwrap() & 0xf,
        4
    );
    assert_eq!(
        pio.read(0x004, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 16),
        1 << 16
    );
    pio.write(0x010, AccessWidth::Word, 4, SimTime::ZERO)
        .unwrap();
    assert_ne!(
        pio.read(0x008, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 16),
        0
    );
    pio.write(0x008, AccessWidth::Word, 1 << 16, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        pio.read(0x008, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 16),
        0
    );

    assert!(handle.inject_rx(0, 0xdead_beef));
    assert_eq!(
        pio.read(0x020, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xdead_beef
    );
    assert!(handle.inject_rx(0, 0x1234));
    pio.write(
        offset(RpPioRegister::Irq0Inte),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_ne!(
        pio.read(
            offset(RpPioRegister::Irq0Ints),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap()
            & 1,
        0
    );
    pio.write(
        offset(RpPioRegister::Irq0Intf),
        AccessWidth::Word,
        2,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        pio.read(
            offset(RpPioRegister::Irq0Ints),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap()
            & 3,
        3
    );
    pio.write(
        offset(RpPioRegister::Irq1Inte),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        pio.read(
            offset(RpPioRegister::Irq1Ints),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap()
            & 1,
        1
    );
    pio.write(
        offset(RpPioRegister::GpioBase),
        AccessWidth::Word,
        0x10,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        pio.read(
            offset(RpPioRegister::GpioBase),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap(),
        0x10
    );
    pio.write(
        offset(RpPioRegister::StateMachine {
            machine: 0,
            register: RpPioStateMachineRegister::ExecCtrl,
        }),
        AccessWidth::Word,
        0x60,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        pio.read(
            offset(RpPioRegister::StateMachine {
                machine: 0,
                register: RpPioStateMachineRegister::ExecCtrl,
            }),
            AccessWidth::Word,
            SimTime::ZERO
        )
        .unwrap()
            & 0x60,
        0x60
    );
    assert!(pio.read(0x048, AccessWidth::Word, SimTime::ZERO).is_err());

    // Empty RX reads set the RXUNDER W1C field (bits 8..11), not the
    // RXSTALL low nibble. EXEC_STALLED is a read-only status bit.
    assert_eq!(
        pio.read(0x024, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
    assert_eq!(
        pio.read(0x008, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 9),
        1 << 9
    );
    pio.write(0x008, AccessWidth::Word, 1 << 9, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        pio.read(0x008, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 9),
        0
    );
    pio.write(
        offset(RpPioRegister::StateMachine {
            machine: 0,
            register: RpPioStateMachineRegister::ExecCtrl,
        }),
        AccessWidth::Word,
        1 << 31,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        pio.read(
            offset(RpPioRegister::StateMachine {
                machine: 0,
                register: RpPioStateMachineRegister::ExecCtrl,
            }),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap()
            & (1 << 31),
        0
    );
}

#[test]
fn rp_pio_pull_out_in_push_and_dreq_follow_fifo_state() {
    let hub = SignalHub::new();
    let (mut pio, handle) = RpPio::new("pio0", 32, "board.rp.pio.shift", hub).unwrap();
    // OUT_COUNT=8, OUT_BASE=0, IN_BASE=0.
    pio.write(0x0dc, AccessWidth::Word, 8 << 20, SimTime::ZERO)
        .unwrap();
    // PULL BLOCK; OUT PINS, 8; IN PINS, 8; PUSH BLOCK.
    for (offset, instruction) in [
        (0x048, 0x80a0),
        (0x04c, 0x6008),
        (0x050, 0x4008),
        (0x054, 0x8020),
    ] {
        pio.write(offset, AccessWidth::Word, instruction, SimTime::ZERO)
            .unwrap();
    }
    pio.write(0x0cc, AccessWidth::Word, 3 << 12, SimTime::ZERO)
        .unwrap();
    assert!(handle.tx_dreq(0));
    pio.write(0x010, AccessWidth::Word, 0xa5, SimTime::ZERO)
        .unwrap();
    assert!(handle.rx_dreq(0) == false);
    handle.set_inputs(0x5a);
    pio.write(0x000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();

    assert!(!handle.poll(SimTime::from_ticks(1)).unwrap());
    assert!(handle.poll(SimTime::from_ticks(2)).unwrap());
    assert!(handle.poll(SimTime::from_ticks(3)).unwrap() == false);
    assert!(!handle.rx_dreq(0));
    assert!(!handle.poll(SimTime::from_ticks(4)).unwrap());
    assert!(handle.rx_dreq(0));
    assert_eq!(
        pio.read(0x020, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x5a00_0000
    );
}

#[test]
fn rp_pio_wait_stalls_until_input_matches() {
    let hub = SignalHub::new();
    let (mut pio, handle) = RpPio::new("pio0", 32, "board.rp.pio.wait", hub).unwrap();
    // WAIT 1 GPIO 3 followed by SET X, 7.
    pio.write(0x048, AccessWidth::Word, 0x2083, SimTime::ZERO)
        .unwrap();
    pio.write(0x04c, AccessWidth::Word, 0xe027, SimTime::ZERO)
        .unwrap();
    pio.write(0x0cc, AccessWidth::Word, 1 << 12, SimTime::ZERO)
        .unwrap();
    pio.write(0x000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();

    handle.poll(SimTime::from_ticks(1)).unwrap();
    assert_eq!(
        pio.read(0x0d4, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
    assert_ne!(
        pio.read(0x0cc, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 31),
        0
    );
    handle.set_inputs(1 << 3);
    handle.poll(SimTime::from_ticks(2)).unwrap();
    assert_eq!(
        pio.read(0x0d4, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1
    );
    assert_eq!(
        pio.read(0x0cc, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 31),
        0
    );
}

#[test]
fn rp_pio_clock_divider_and_side_set_gate_execution() {
    let hub = SignalHub::new();
    let (mut pio, handle) = RpPio::new("pio0", 32, "board.rp.pio.sideset", hub.clone()).unwrap();
    // One mandatory side-set pin at GPIO5 and a 2.0 divider.
    pio.write(
        0x0dc,
        AccessWidth::Word,
        (1 << 29) | (5 << 10),
        SimTime::ZERO,
    )
    .unwrap();
    pio.write(0x0c8, AccessWidth::Word, 2 << 16, SimTime::ZERO)
        .unwrap();
    pio.write(0x048, AccessWidth::Word, 0xf020, SimTime::ZERO)
        .unwrap();
    pio.write(0x0cc, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    pio.write(0x000, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();

    assert!(!handle.poll(SimTime::from_ticks(1)).unwrap());
    assert!(handle.poll(SimTime::from_ticks(2)).unwrap());
    let changes = hub.drain_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].value.bit(5), Some(Logic::One));
}

#[test]
fn rp2350_pad_controls_expose_pulls_input_enable_and_output_disable() {
    let (mut pads, handle) = RpPadsBank::new("pads", 48, RpPadsVariant::Rp2350);
    let gpio33 = 4 + 33 * 4;
    assert_eq!(
        pads.read(gpio33, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x116
    );
    assert!(!handle.input_enabled(33));
    assert_eq!(handle.pull(33), Some(Logic::Zero));
    pads.write(
        gpio33,
        AccessWidth::Word,
        (1 << 7) | (1 << 6) | (1 << 3),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.input_enabled(33));
    assert!(handle.output_disabled(33));
    assert_eq!(handle.pull(33), Some(Logic::One));
    pads.write(gpio33 + 0x3000, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    assert!(!handle.output_disabled(33));
}
