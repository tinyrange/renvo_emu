use super::*;

#[test]
fn wch_i2c_models_master_write_read_and_interrupts() {
    let (mut i2c, handle) = WchI2c::new("i2c1");
    let at = SimTime::ZERO;
    const PE: u64 = 1;
    const START: u64 = 1 << 8;
    const STOP: u64 = 1 << 9;
    const EVENT_IRQ: u64 = 1 << 9;
    const BUFFER_IRQ: u64 = 1 << 10;
    const SB: u64 = 1;
    const ADDR: u64 = 1 << 1;
    const RXNE: u64 = 1 << 6;
    const TXE: u64 = 1 << 7;

    i2c.write(0x04, AccessWidth::HalfWord, EVENT_IRQ | BUFFER_IRQ, at)
        .unwrap();
    i2c.write(0x00, AccessWidth::HalfWord, PE | START, at)
        .unwrap();
    assert_eq!(i2c.read(0x14, AccessWidth::HalfWord, at).unwrap() & SB, SB);
    assert_eq!(handle.interrupt_pending(), (true, false));

    i2c.write(0x10, AccessWidth::HalfWord, 0xa0, at).unwrap();
    assert_eq!(
        i2c.read(0x14, AccessWidth::HalfWord, at).unwrap() & (ADDR | TXE),
        ADDR
    );
    let _ = i2c.read(0x18, AccessWidth::HalfWord, at).unwrap();
    assert_ne!(i2c.read(0x14, AccessWidth::HalfWord, at).unwrap() & TXE, 0);
    i2c.write(0x10, AccessWidth::HalfWord, 0x12, at).unwrap();
    i2c.write(0x10, AccessWidth::HalfWord, 0x34, at).unwrap();
    assert_eq!(
        handle.take_transmitted(),
        vec![
            WchI2cWrite {
                address: 0x50,
                value: 0x12,
            },
            WchI2cWrite {
                address: 0x50,
                value: 0x34,
            },
        ]
    );

    handle.queue_read(0x50, &[0xab, 0xcd]);
    i2c.write(0x00, AccessWidth::HalfWord, PE | START, at)
        .unwrap();
    i2c.write(0x10, AccessWidth::HalfWord, 0xa1, at).unwrap();
    let _ = i2c.read(0x14, AccessWidth::HalfWord, at).unwrap();
    let _ = i2c.read(0x18, AccessWidth::HalfWord, at).unwrap();
    assert_eq!(
        i2c.read(0x14, AccessWidth::HalfWord, at).unwrap() & (ADDR | RXNE),
        RXNE
    );
    assert_eq!(i2c.read(0x10, AccessWidth::HalfWord, at).unwrap(), 0xab);
    assert_eq!(i2c.read(0x10, AccessWidth::HalfWord, at).unwrap(), 0xcd);
    assert_eq!(i2c.read(0x14, AccessWidth::HalfWord, at).unwrap() & RXNE, 0);
    i2c.write(0x00, AccessWidth::HalfWord, PE | STOP, at)
        .unwrap();
    assert_eq!(i2c.read(0x18, AccessWidth::HalfWord, at).unwrap(), 0);
}

#[test]
fn wch_i2c_nack_raises_error_interrupt_and_can_be_configured() {
    let (mut i2c, handle) = WchI2c::new("i2c1");
    let at = SimTime::ZERO;
    handle.set_address_ack(0x30, false);
    i2c.write(0x04, AccessWidth::HalfWord, 0, at).unwrap();
    i2c.write(0x00, AccessWidth::HalfWord, 1 | (1 << 8), at)
        .unwrap();
    i2c.write(0x10, AccessWidth::HalfWord, 0x60, at).unwrap();
    assert_eq!(handle.interrupt_pending(), (false, false));
    i2c.write(0x04, AccessWidth::HalfWord, (1 << 8) | (1 << 9), at)
        .unwrap();
    assert_eq!(handle.interrupt_pending(), (false, true));
    i2c.write(0x14, AccessWidth::HalfWord, !(1 << 10), at)
        .unwrap();
    assert_eq!(handle.interrupt_pending(), (false, false));
}

#[test]
fn wch_i2c_masks_registers_and_only_clears_rw0_errors() {
    let (mut i2c, handle) = WchI2c::new("i2c1");
    let at = SimTime::ZERO;

    i2c.write(0x00, AccessWidth::HalfWord, 0x7fff, at).unwrap();
    assert_eq!(
        i2c.read(0x00, AccessWidth::HalfWord, at).unwrap(),
        0x7fff & 0x9fe1 & !(1 << 8) & !(1 << 9)
    );
    i2c.write(0x04, AccessWidth::HalfWord, u16::MAX.into(), at)
        .unwrap();
    assert_eq!(i2c.read(0x04, AccessWidth::HalfWord, at).unwrap(), 0x1f3f);
    i2c.write(0x08, AccessWidth::HalfWord, u16::MAX.into(), at)
        .unwrap();
    assert_eq!(i2c.read(0x08, AccessWidth::HalfWord, at).unwrap(), 0x83ff);
    i2c.write(0x0c, AccessWidth::HalfWord, u16::MAX.into(), at)
        .unwrap();
    assert_eq!(i2c.read(0x0c, AccessWidth::HalfWord, at).unwrap(), 0x00ff);
    i2c.write(0x1c, AccessWidth::HalfWord, u16::MAX.into(), at)
        .unwrap();
    assert_eq!(i2c.read(0x1c, AccessWidth::HalfWord, at).unwrap(), 0xcfff);

    // SB is read-only and survives a status-register write; AF is RW0 and is
    // cleared by the vendor SDK's write-the-complement sequence.
    i2c.write(0x00, AccessWidth::HalfWord, 1 | (1 << 8), at)
        .unwrap();
    assert_ne!(i2c.read(0x14, AccessWidth::HalfWord, at).unwrap() & 1, 0);
    handle.set_address_ack(0x30, false);
    i2c.write(0x10, AccessWidth::HalfWord, 0x60, at).unwrap();
    assert_ne!(
        i2c.read(0x14, AccessWidth::HalfWord, at).unwrap() & (1 << 10),
        0
    );
    i2c.write(0x14, AccessWidth::HalfWord, !(1 << 10), at)
        .unwrap();
    assert_eq!(
        i2c.read(0x14, AccessWidth::HalfWord, at).unwrap() & (1 << 10),
        0
    );
}

#[test]
fn exti_routes_afio_selected_edges_and_clears_flags() {
    let (mut exti, handle, mut afio) = WchExti::new("exti", "afio");
    // EXTICR line 2 selects PC (the WCH encoding is PA=0, PB=1, PC=2).
    afio.write(0x08, AccessWidth::Word, 2 << (2 * 2), SimTime::ZERO)
        .unwrap();
    exti.write(0x00, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();
    exti.write(0x08, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();

    assert!(!handle.pending([0, 0, 0]));
    assert!(handle.pending([0, 1 << 2, 0]));
    assert_eq!(
        exti.read(0x14, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 2
    );
    exti.write(0x14, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        exti.read(0x14, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0
    );
}

#[test]
fn afio_uses_named_registers_and_masks_reserved_bits() {
    let (mut _exti, _handle, mut afio) = WchExti::new("exti", "afio");
    afio.write(
        WchAfioRegister::Pcfr1.offset(),
        AccessWidth::Word,
        u64::from(u32::MAX),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        afio.read(
            WchAfioRegister::Pcfr1.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        u64::from(WCH_AFIO_PCFR1_MASK)
    );

    afio.write(
        WchAfioRegister::Exticr.offset(),
        AccessWidth::Word,
        u64::from(u32::MAX),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        afio.read(
            WchAfioRegister::Exticr.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0xffff
    );
}

#[test]
fn exti_reserved_port_selection_does_not_alias_pa() {
    let (mut exti, handle, mut afio) = WchExti::new("exti", "afio");
    afio.write(
        WchAfioRegister::Exticr.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    exti.write(
        WchExtiRegister::InterruptEnable.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    exti.write(
        WchExtiRegister::RisingTrigger.offset(),
        AccessWidth::Word,
        1,
        SimTime::ZERO,
    )
    .unwrap();

    // AFIO encoding 01 is reserved on CH32V00x. A high PA0 must not look
    // like an edge on a nonexistent PB0 input.
    assert!(!handle.pending([1, 0, 0]));
    assert_eq!(
        exti.read(
            WchExtiRegister::InterruptFlag.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        )
        .unwrap(),
        0
    );
}

#[test]
fn advanced_timer_honors_update_disable_and_native_capture_width() {
    let (mut timer, handle) = WchTimer::new("tim1");
    timer
        .write(0x2c, AccessWidth::HalfWord, 2, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x28, AccessWidth::HalfWord, 0, SimTime::ZERO)
        .unwrap();
    timer
        .write(0x0c, AccessWidth::HalfWord, 1, SimTime::ZERO)
        .unwrap();
    // UDIS suppresses the update event while the counter continues to run.
    timer
        .write(0x00, AccessWidth::HalfWord, 1 | (1 << 1), SimTime::ZERO)
        .unwrap();
    assert!(!handle.pending(SimTime::from_ticks(3)));
    assert_eq!(
        timer
            .read(0x10, AccessWidth::HalfWord, SimTime::from_ticks(3))
            .unwrap(),
        0
    );

    timer
        .write(0x00, AccessWidth::HalfWord, 1, SimTime::from_ticks(3))
        .unwrap();
    assert!(handle.pending(SimTime::from_ticks(6)));

    // CHxCVR is a 32-bit register, but only the low 16-bit value is writable.
    timer
        .write(0x34, AccessWidth::Word, 0xabcd_1234, SimTime::from_ticks(6))
        .unwrap();
    assert_eq!(
        timer
            .read(0x34, AccessWidth::Word, SimTime::from_ticks(6))
            .unwrap(),
        0x1234
    );
}

#[test]
fn pfic_models_stk_counter_compare_clock_and_irq12() {
    let (mut pfic, handle) = WchPfic::new("pfic");
    pfic.write(0x1010, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    pfic.write(0x1008, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    pfic.write(0x1000, AccessWidth::Word, 0x0f, SimTime::ZERO)
        .unwrap();

    assert!(!handle.take_systick_pending(SimTime::from_ticks(1)));
    assert!(handle.take_systick_pending(SimTime::from_ticks(2)));
    assert_ne!(
        pfic.read(0x1004, AccessWidth::Word, SimTime::from_ticks(2))
            .unwrap()
            & 1,
        0
    );
    assert_eq!(
        pfic.read(0x1008, AccessWidth::Word, SimTime::from_ticks(2))
            .unwrap(),
        2
    );
    assert!(handle.take_systick_pending(SimTime::from_ticks(2)));
    pfic.write(0x1004, AccessWidth::Word, 0, SimTime::from_ticks(2))
        .unwrap();
    assert!(!handle.take_systick_pending(SimTime::from_ticks(2)));

    let (mut prescaled, prescaled_handle) = WchPfic::new("prescaled-pfic");
    prescaled
        .write(0x1010, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    prescaled
        .write(0x1000, AccessWidth::Word, 0x0b, SimTime::ZERO)
        .unwrap();
    assert!(!prescaled_handle.take_systick_pending(SimTime::from_ticks(7)));
    assert!(prescaled_handle.take_systick_pending(SimTime::from_ticks(8)));
}
