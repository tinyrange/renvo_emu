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
    assert!(RpPioRegister::try_from_offset_for_version(0x12c, version).is_err());
    assert_eq!(
        pio.read(0x044, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x1020_0404
    );
    assert_eq!(
        pio.read(0x004, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x0f00_0f00
    );

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

    // GPIO32..47 have registers but no electrical nets in the current machine.
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
fn esp32c6_spimem_reports_idle_axi_fifos_across_reset() {
    let mut spimem = EspSpiMem::new("spimem");
    assert_eq!(
        spimem
            .read(0x170, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0xfc00_0000
    );
    spimem.reset(ResetKind::Software);
    assert_eq!(
        spimem
            .read(0x170, AccessWidth::Word, SimTime::ZERO)
            .unwrap(),
        0xfc00_0000
    );
}

#[test]
fn esp32c6_spimem_exports_program_erase_and_read_transactions() {
    let (mut spimem, handle) = EspSpiMem::new_observed("spimem");

    spimem
        .write(0, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x20, AccessWidth::Word, 0x7000_0002, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x04, AccessWidth::Word, 0x9120, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x24, AccessWidth::Word, 31, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x58, AccessWidth::Word, 0x4433_2211, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0, AccessWidth::Word, 1 << 18, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        handle.take_flash_command(),
        Some(EspSpiFlashCommand::Program {
            address: 0x9120,
            data: vec![0x11, 0x22, 0x33, 0x44],
        })
    );

    spimem
        .write(0, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x20, AccessWidth::Word, 0x7000_0020, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x04, AccessWidth::Word, 0x9000, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0, AccessWidth::Word, 1 << 18, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        handle.take_flash_command(),
        Some(EspSpiFlashCommand::EraseSector { address: 0x9000 })
    );

    spimem
        .write(0x20, AccessWidth::Word, 0x7000_00bb, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x04, AccessWidth::Word, 0x9000, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0x28, AccessWidth::Word, 31, SimTime::ZERO)
        .unwrap();
    spimem
        .write(0, AccessWidth::Word, 1 << 18, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        handle.take_flash_command(),
        Some(EspSpiFlashCommand::Read {
            address: 0x9000,
            length: 4,
        })
    );
    handle.complete_flash_read(vec![0xaa, 0xbb, 0xcc, 0xdd]);
    assert_eq!(
        spimem.read(0x58, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0xddcc_bbaa
    );
}

#[test]
fn esp32c6_analog_i2c_completes_rfpll_charge_pump_calibration() {
    let mut i2c = EspAnalogI2c::new("analog-i2c");
    let write = (1_u64 << 24) | (0x20_u64 << 16) | (0x0f_u64 << 8) | 0x62;
    i2c.write(0x188, AccessWidth::Word, write, SimTime::ZERO)
        .unwrap();
    i2c.write(
        0x188,
        AccessWidth::Word,
        (0x0e_u64 << 8) | 0x62,
        SimTime::ZERO,
    )
    .unwrap();
    assert_ne!(
        i2c.read(0x188, AccessWidth::Word, SimTime::ZERO).unwrap() & (0x80 << 16),
        0
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
#[test]
fn pwm_advances_counter_and_reports_compare_outputs() {
    let (mut pwm, handle) = FunctionalPwm::new("pwm", 2);
    pwm.write(0x0c, AccessWidth::Word, 4, SimTime::ZERO)
        .unwrap();
    pwm.write(0x10, AccessWidth::Word, 9, SimTime::ZERO)
        .unwrap();
    pwm.write(0x00, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    pwm.write(
        RpPwmRegister::En.global_offset(2).unwrap(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    pwm.write(
        RpPwmRegister::Inte0.global_offset(2).unwrap(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.counter(0), Some(0));
    assert_eq!(handle.outputs(0), Some([true, false]));
    assert_eq!(
        pwm.read(0x08, AccessWidth::Word, SimTime::from_ticks(5))
            .unwrap(),
        5
    );
    assert_eq!(handle.outputs(0), Some([false, false]));
    assert_eq!(handle.pending_interrupts(), 0);
    assert_eq!(
        pwm.read(0x08, AccessWidth::Word, SimTime::from_ticks(12))
            .unwrap(),
        2
    );
    assert_ne!(handle.pending_interrupts(), 0);
}

#[test]
fn pwm_phase_commands_adjust_counter_and_self_clear() {
    let (mut pwm, handle) = FunctionalPwm::new("pwm", 1);
    pwm.write(0x10, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    pwm.write(0x08, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();

    pwm.write(0x00, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.counter(0), Some(0));
    assert_eq!(pwm.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);

    pwm.write(0x00, AccessWidth::Word, 1 << 6, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.counter(0), Some(3));
    assert_eq!(pwm.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
}

#[test]
fn i2c_executes_addressed_writes_and_queued_reads() {
    let (mut i2c, handle) = FunctionalI2c::new("i2c");
    handle.queue_read(0x58, &[0x12]);
    i2c.write(0x04, AccessWidth::Word, 0x58, SimTime::ZERO)
        .unwrap();
    i2c.write(0x6c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    i2c.write(0x10, AccessWidth::Word, 0xa0, SimTime::ZERO)
        .unwrap();
    i2c.write(0x10, AccessWidth::Word, 1 << 8, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        i2c.read(0x10, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x12
    );
    assert_eq!(
        handle.events(),
        [
            I2cEvent::Write {
                address: 0x58,
                value: 0xa0
            },
            I2cEvent::Read {
                address: 0x58,
                value: 0x12
            },
        ]
    );
}

#[test]
fn i2c_reports_fifo_and_stop_interrupt_state() {
    let (mut i2c, _handle) = FunctionalI2c::new("i2c");
    i2c.write(0x30, AccessWidth::Word, (1 << 4) | (1 << 9), SimTime::ZERO)
        .unwrap();
    i2c.write(0x04, AccessWidth::Word, 0x20, SimTime::ZERO)
        .unwrap();
    i2c.write(0x6c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    i2c.write(0x10, AccessWidth::Word, (1 << 9) | 0x5a, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        i2c.read(0x2c, AccessWidth::Word, SimTime::ZERO).unwrap(),
        (1 << 4) | (1 << 9)
    );
    assert_eq!(
        i2c.read(0x70, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x06
    );
}
