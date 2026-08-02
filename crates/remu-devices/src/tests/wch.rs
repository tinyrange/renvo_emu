use super::*;

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
fn pfic_models_systick_reload_countflag_and_irq12() {
    let (mut pfic, handle) = WchPfic::new("pfic");
    pfic.write(0x014, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    pfic.write(0x018, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    pfic.write(0x010, AccessWidth::Word, 7, SimTime::ZERO)
        .unwrap();

    assert!(!handle.take_systick_pending(SimTime::from_ticks(2)));
    assert!(handle.take_systick_pending(SimTime::from_ticks(3)));
    assert_ne!(
        pfic.read(0x010, AccessWidth::Word, SimTime::from_ticks(3))
            .unwrap()
            & (1 << 16),
        0
    );
    assert_eq!(
        pfic.read(0x014, AccessWidth::Word, SimTime::from_ticks(3))
            .unwrap(),
        2
    );
    assert!(!handle.take_systick_pending(SimTime::from_ticks(3)));
}
