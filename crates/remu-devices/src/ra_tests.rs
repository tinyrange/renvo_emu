
use super::*;

#[test]
fn kint_register_ids_are_named_and_native() {
    assert_eq!(RaKintRegister::ALL.len(), 3);
    assert_eq!(RaKintRegister::Krctl.offset(), 0x00);
    assert_eq!(RaKintRegister::Krf.name(), "krf");
    assert_eq!(
        RaKintRegister::from_offset(RaKintRegister::Krm.offset()),
        Some(RaKintRegister::Krm)
    );
    assert_eq!(RaKintRegister::from_offset(0x0c), None);
}

#[test]
fn ioport_atomic_output_and_pfs_direction_are_visible() {
    let hub = SignalHub::new();
    let (mut port, handle) = RaIoPort::new("port1", "board.ra.port1", hub).unwrap();
    port.write(0, AccessWidth::Word, 1 << 11, SimTime::ZERO)
        .unwrap();
    port.write(8, AccessWidth::Word, 1 << 11, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.output(), 1 << 11);
    assert_eq!(handle.direction(), 1 << 11);
}

#[test]
fn gpt_and_sci_events_route_through_ielsr() {
    let (mut icu, handle) = RaIcu::new("icu");
    icu.write(
        0x300 + 7 * 4,
        AccessWidth::Word,
        u64::from(RA4M1_EVENT_GPT0_OVERFLOW),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.route_event(RA4M1_EVENT_GPT0_OVERFLOW), vec![7]);
    icu.write(
        0x300 + 8 * 4,
        AccessWidth::Word,
        u64::from(super::super::RA4M1_EVENT_ADC0_SCAN_END),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        handle.route_event(super::super::RA4M1_EVENT_ADC0_SCAN_END),
        vec![8]
    );

    let (mut gpt, gpt_handle) = RaGpt::new("gpt0");
    gpt.write(0x64, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    gpt.write(0x38, AccessWidth::Word, 1 << 6, SimTime::ZERO)
        .unwrap();
    gpt.write(0x2c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert!(gpt_handle.poll(SimTime::from_ticks(4)));

    let (mut sci, sci_handle) = RaSci::new("sci9");
    sci.write(3, AccessWidth::Byte, b'R'.into(), SimTime::ZERO)
        .unwrap();
    assert_eq!(sci_handle.bytes(), b"R");
}

#[test]
fn agt_channels_count_down_and_emit_underflow_events() {
    let (mut agt0, handle0) = RaAgt::new("agt0");
    agt0.write(0x00, AccessWidth::HalfWord, 3, SimTime::ZERO)
        .unwrap();
    agt0.write(0x08, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    assert!(!handle0.poll(SimTime::from_ticks(3)));
    assert!(handle0.poll(SimTime::from_ticks(4)));
    assert!(!handle0.poll(SimTime::from_ticks(4)));
    assert_eq!(
        agt0.read(0x08, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x23
    );
    agt0.write(0x08, AccessWidth::Byte, 1, SimTime::from_ticks(4))
        .unwrap();
    assert_eq!(
        agt0.read(0x08, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x03
    );

    let (mut agt1, handle1) = RaAgt::new("agt1");
    agt1.write(0x00, AccessWidth::HalfWord, 1, SimTime::ZERO)
        .unwrap();
    agt1.write(0x08, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    assert!(handle1.poll(SimTime::from_ticks(2)));
}

#[test]
fn spi_transfer_sets_status_and_exposes_host_bytes() {
    let (mut spi, handle) = RaSpi::new("spi0");
    spi.write(0, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
        .unwrap();
    spi.write(0x0b, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
        .unwrap();
    handle.queue_rx(0xa5);
    spi.write(4, AccessWidth::Byte, 0x5a, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.take_tx(), vec![0x5a]);
    assert_eq!(
        spi.read(3, AccessWidth::Byte, SimTime::ZERO).unwrap() & 0xa0,
        0xa0
    );
    assert_eq!(spi.read(4, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0xa5);
    assert_eq!(
        spi.read(3, AccessWidth::Byte, SimTime::ZERO).unwrap() & 0x80,
        0
    );
}

#[test]
fn spi_reset_defaults_and_width_selection_follow_ra4m1_registers() {
    let (mut spi, _) = RaSpi::new("spi0");
    assert_eq!(spi.read(0, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0);
    assert_eq!(spi.read(3, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0x20);
    assert_eq!(
        spi.read(0x0a, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0xff
    );
    assert_eq!(
        spi.read(0x10, AccessWidth::HalfWord, SimTime::ZERO)
            .unwrap(),
        0x0401
    );
    assert!(
        spi.write(4, AccessWidth::Byte, 0x12, SimTime::ZERO)
            .is_err()
    );

    spi.write(0x0b, AccessWidth::Byte, 0x40, SimTime::ZERO)
        .unwrap();
    spi.write(0, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
        .unwrap();
    spi.write(4, AccessWidth::Byte, 0x12, SimTime::ZERO)
        .unwrap();
    assert_eq!(spi.read(4, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0);

    spi.reset(ResetKind::PowerOn);
    spi.write(0, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
        .unwrap();
    assert!(
        spi.write(4, AccessWidth::Byte, 0x12, SimTime::ZERO)
            .is_err()
    );
    spi.write(4, AccessWidth::HalfWord, 0x1234, SimTime::ZERO)
        .unwrap();

    spi.reset(ResetKind::PowerOn);
    spi.write(0x0b, AccessWidth::Byte, 1 << 5, SimTime::ZERO)
        .unwrap();
    spi.write(0, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
        .unwrap();
    spi.write(4, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
        .unwrap();
    assert_eq!(spi.read(4, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
}

#[test]
fn iic_start_transmit_receive_and_stop_are_deterministic() {
    let (mut iic, handle) = RaIic::new("iic0");
    iic.write(0x00, AccessWidth::Byte, IicState::ICE.into(), SimTime::ZERO)
        .unwrap();
    iic.write(
        0x01,
        AccessWidth::Byte,
        IicState::START_REQUEST.into(),
        SimTime::ZERO,
    )
    .unwrap();
    iic.write(0x12, AccessWidth::Byte, 0x6e, SimTime::ZERO)
        .unwrap();
    iic.write(0x12, AccessWidth::Byte, 0xa5, SimTime::ZERO)
        .unwrap();
    iic.write(
        0x07,
        AccessWidth::Byte,
        IicState::TDRE.into(),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.bus_busy());
    assert_eq!(handle.transmitted(), [0x6e, 0xa5]);
    assert!(handle.interrupt_pending());

    handle.enqueue_receive(0x5a);
    assert_eq!(
        iic.read(0x13, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x5a
    );
    handle.set_nack();
    assert_ne!(
        iic.read(0x09, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8 & IicState::NACKF,
        0
    );
    iic.write(
        0x01,
        AccessWidth::Byte,
        IicState::STOP_REQUEST.into(),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(!handle.bus_busy());
    assert_eq!(
        iic.read(0x09, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8 & IicState::STOP_DETECTED,
        IicState::STOP_DETECTED
    );
}

#[test]
fn iic_reset_defaults_and_zero_to_clear_status_match_native_registers() {
    let (mut iic, _) = RaIic::new("iic0");
    assert_eq!(
        iic.read(0x00, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x1f
    );
    assert_eq!(
        iic.read(0x02, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x08
    );
    assert_eq!(
        iic.read(0x03, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x06
    );
    assert_eq!(
        iic.read(0x05, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x72
    );
    assert_eq!(
        iic.read(0x06, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x09
    );
    assert_eq!(
        iic.read(0x10, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0xff
    );
    assert_eq!(
        iic.read(0x11, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0xff
    );
    assert_eq!(
        iic.read(0x12, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0xff
    );
    assert_eq!(iic.read(0x13, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0);

    iic.write(0x00, AccessWidth::Byte, IicState::ICE.into(), SimTime::ZERO)
        .unwrap();
    iic.write(
        0x01,
        AccessWidth::Byte,
        IicState::START_REQUEST.into(),
        SimTime::ZERO,
    )
    .unwrap();
    assert_ne!(
        iic.read(0x09, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8 & IicState::START_DETECTED,
        0
    );
    // ICSR2 flags are cleared by writing zero, while writing one leaves a
    // latched flag set.
    iic.write(0x09, AccessWidth::Byte, 0xff, SimTime::ZERO)
        .unwrap();
    assert_ne!(
        iic.read(0x09, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8 & IicState::START_DETECTED,
        0
    );
    iic.write(0x09, AccessWidth::Byte, 0, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        iic.read(0x09, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8 & IicState::START_DETECTED,
        0
    );
}
