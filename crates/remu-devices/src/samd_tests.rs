use super::*;
use crate::Samd21Wdt;

#[test]
fn port_aliases_drive_vcd_backed_pins() {
    let hub = SignalHub::new();
    let (mut port, handle) = Samd21Port::new("port", 26, "board.atsamd21e18.gpio", hub).unwrap();
    port.write(0x08, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    port.write(0x18, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.direction(), 1 << 7);
    assert_eq!(handle.output(), 1 << 7);
    assert_eq!(handle.resolved(7).unwrap(), Logic::One);
}

#[test]
fn tc_match_sets_and_clears_mc0_interrupt() {
    let (mut tc, handle) = Samd21Tc::new("tc3");
    tc.write(0x18, AccessWidth::HalfWord, 4, SimTime::ZERO)
        .unwrap();
    tc.write(0x0d, AccessWidth::Byte, 0x10, SimTime::ZERO)
        .unwrap();
    tc.write(0x00, AccessWidth::HalfWord, 2, SimTime::ZERO)
        .unwrap();
    assert!(!handle.poll(SimTime::from_ticks(3)));
    assert!(handle.poll(SimTime::from_ticks(4)));
    tc.write(0x0e, AccessWidth::Byte, 0x10, SimTime::from_ticks(4))
        .unwrap();
    assert!(!handle.poll(SimTime::from_ticks(4)));
}

#[test]
fn sercom_data_collects_transmit_bytes() {
    let (mut usart, handle) = Samd21Usart::new("sercom0");
    usart
        .write(0x28, AccessWidth::HalfWord, u64::from(b'A'), SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.bytes(), b"A");
}

#[test]
fn sercom_named_registers_and_vendor_mode_masks_match_cmsis() {
    assert_eq!(
        Samd21SercomRegister::from_offset(0x00),
        Some(Samd21SercomRegister::Ctrla)
    );
    assert_eq!(
        Samd21SercomRegister::from_offset(0x28),
        Some(Samd21SercomRegister::Data)
    );
    assert_eq!(Samd21SercomRegister::from_offset(0x20), None);
    assert_eq!(Samd21SercomRegister::Dbgctrl.offset(), 0x30);

    let (mut sercom, handle) = Samd21Usart::new("sercom0");
    let spi_ctrla = (0x7f33_019e_u32 & !0x1e) | (3 << 2);
    sercom
        .write(0x00, AccessWidth::Word, u64::from(spi_ctrla), SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.mode(), Samd21SercomMode::SpiMaster);
    assert_eq!(
        sercom.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
        u64::from(spi_ctrla)
    );
    sercom
        .write(0x04, AccessWidth::Word, u64::MAX, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        sercom.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
        0x0002_e247
    );

    sercom
        .write(0x00, AccessWidth::Word, (5_u64 << 2) | 2, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.mode(), Samd21SercomMode::I2cMaster);
    sercom
        .write(0x16, AccessWidth::Byte, u64::MAX, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        sercom.read(0x16, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x83
    );
    assert_eq!(
        sercom.read(0x00, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x16
    );
}

#[test]
fn sercom_mode_values_follow_samd21_shared_mode_encoding() {
    let (mut sercom, handle) = Samd21Usart::new("sercom0");
    for (mode, expected) in [
        (0, Samd21SercomMode::Usart),
        (1, Samd21SercomMode::Usart),
        (2, Samd21SercomMode::SpiSlave),
        (3, Samd21SercomMode::SpiMaster),
        (4, Samd21SercomMode::I2cSlave),
        (5, Samd21SercomMode::I2cMaster),
        (6, Samd21SercomMode::Other(6)),
    ] {
        sercom
            .write(0x00, AccessWidth::Word, mode << 2, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.mode(), expected);
    }
}

#[test]
fn sercom_spi_master_exposes_loopback_and_injected_receive_data() {
    let (mut sercom, handle) = Samd21Usart::new("sercom0");
    let ctrla = 3_u64 << 2;
    sercom
        .write(0x00, AccessWidth::Word, ctrla, SimTime::ZERO)
        .unwrap();
    sercom
        .write(0x04, AccessWidth::Word, 1 << 17, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        sercom.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 17
    );
    sercom
        .write(0x00, AccessWidth::Word, ctrla | 2, SimTime::ZERO)
        .unwrap();
    sercom
        .write(0x16, AccessWidth::Byte, 1 << 2, SimTime::ZERO)
        .unwrap();
    handle.queue_spi_rx([0xa5]);
    sercom
        .write(0x28, AccessWidth::Byte, 0x3c, SimTime::ZERO)
        .unwrap();

    assert_eq!(handle.mode(), Samd21SercomMode::SpiMaster);
    assert_eq!(handle.spi_bytes(), [0x3c]);
    assert_eq!(
        sercom.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
        1 << 17
    );
    assert_eq!(
        sercom.read(0x16, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        4
    );
    assert_eq!(
        sercom.read(0x18, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        7
    );
    assert!(handle.interrupt_pending());
    assert_eq!(
        sercom.read(0x18, AccessWidth::Byte, SimTime::ZERO).unwrap() & 0x04,
        0x04
    );
    assert_eq!(
        sercom.read(0x28, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0xa5
    );
    assert_eq!(
        sercom.read(0x18, AccessWidth::Byte, SimTime::ZERO).unwrap() & 0x04,
        0
    );

    // With no external response queued, the functional SPI path is deterministic loopback.
    sercom
        .write(0x28, AccessWidth::Byte, 0x12, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        sercom.read(0x28, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x12
    );
}

#[test]
fn sercom_i2c_master_models_address_data_read_and_stop_flags() {
    let (mut sercom, handle) = Samd21Usart::new("sercom0");
    let ctrla = 5_u64 << 2;
    sercom
        .write(0x00, AccessWidth::Word, ctrla, SimTime::ZERO)
        .unwrap();
    sercom
        .write(0x16, AccessWidth::Byte, 0x03, SimTime::ZERO)
        .unwrap();
    sercom
        .write(0x00, AccessWidth::Word, ctrla | 2, SimTime::ZERO)
        .unwrap();
    sercom
        .write(0x24, AccessWidth::Byte, 0xa0, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.mode(), Samd21SercomMode::I2cMaster);
    assert_eq!(handle.i2c_address(), Some(0xa0));
    assert_eq!(
        sercom.read(0x18, AccessWidth::Byte, SimTime::ZERO).unwrap() & 1,
        1
    );
    sercom
        .write(0x28, AccessWidth::Byte, 0x10, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.i2c_bytes(), [0x10]);

    handle.queue_i2c_rx([0x42]);
    sercom
        .write(0x24, AccessWidth::Byte, 0xa1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        sercom.read(0x1a, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        (2 << 4) | (1 << 7)
    );
    assert_eq!(
        sercom.read(0x18, AccessWidth::Byte, SimTime::ZERO).unwrap() & 2,
        2
    );
    assert_eq!(
        sercom.read(0x28, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x42
    );
    assert_eq!(
        sercom.read(0x18, AccessWidth::Byte, SimTime::ZERO).unwrap() & 2,
        0
    );

    sercom
        .write(0x04, AccessWidth::Word, 2 << 16, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        sercom.read(0x1a, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        1 << 4
    );
    assert_eq!(
        sercom.read(0x18, AccessWidth::Byte, SimTime::ZERO).unwrap() & 3,
        0
    );
}

#[test]
fn dac_models_native_control_data_buffer_and_interrupts() {
    let (mut dac, handle) = Samd21Dac::new("dac");
    assert!(!handle.enabled());
    dac.write(0x01, AccessWidth::Byte, 0x41, SimTime::ZERO)
        .unwrap();
    dac.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
        .unwrap();
    dac.write(0x08, AccessWidth::HalfWord, 0xffff, SimTime::ZERO)
        .unwrap();
    dac.write(0x0c, AccessWidth::HalfWord, 0x0555, SimTime::ZERO)
        .unwrap();
    assert!(handle.enabled());
    assert_eq!(handle.control_b(), 0x41);
    assert_eq!(handle.data(), 0x03ff);
    assert_eq!(handle.data_buffer(), 0x0155);
    assert!(handle.data_buffer_full());
    dac.write(0x02, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    dac.write(0x05, AccessWidth::Byte, 0x02, SimTime::ZERO)
        .unwrap();
    handle.start_conversion(SimTime::from_ticks(3)).unwrap();
    assert_eq!(handle.data(), 0x0155);
    assert!(!handle.data_buffer_full());
    assert!(handle.interrupt_pending());
    assert_eq!(
        dac.read(0x06, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x02
    );
    dac.write(0x06, AccessWidth::Byte, 0x02, SimTime::from_ticks(3))
        .unwrap();
    assert!(!handle.interrupt_pending());
    dac.write(0x00, AccessWidth::Byte, 0x01, SimTime::from_ticks(3))
        .unwrap();
    assert!(!handle.enabled());
    assert_eq!(handle.data(), 0);
}

#[test]
fn dac_decodes_left_adjusted_data_and_reports_underrun() {
    let (mut dac, handle) = Samd21Dac::new("dac");
    dac.write(0x01, AccessWidth::Byte, 0x04, SimTime::ZERO)
        .unwrap();
    dac.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
        .unwrap();
    dac.write(0x02, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    dac.write(0x08, AccessWidth::HalfWord, 0x03fc, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.data(), 0x000f);
    handle.start_conversion(SimTime::from_ticks(1)).unwrap();
    assert_eq!(dac.read(0x06, AccessWidth::Byte, SimTime::ZERO).unwrap(), 1);
}

#[test]
fn dac_honors_native_byte_lanes_and_write_only_data_registers() {
    let (mut dac, handle) = Samd21Dac::new("dac");
    dac.write(0x08, AccessWidth::Byte, 0x02, SimTime::ZERO)
        .unwrap();
    dac.write(0x09, AccessWidth::Byte, 0x03, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.data(), 0x0302);
    assert_eq!(
        dac.read(0x08, AccessWidth::HalfWord, SimTime::ZERO)
            .unwrap(),
        0
    );
    dac.write(0x0c, AccessWidth::Byte, 0x05, SimTime::ZERO)
        .unwrap();
    dac.write(0x0d, AccessWidth::Byte, 0x02, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.data_buffer(), 0x0205);
    assert_eq!(
        dac.read(0x0c, AccessWidth::HalfWord, SimTime::ZERO)
            .unwrap(),
        0
    );
}

#[test]
fn dac_control_b_is_enable_protected() {
    let (mut dac, handle) = Samd21Dac::new("dac");
    dac.write(0x01, AccessWidth::Byte, 0x04, SimTime::ZERO)
        .unwrap();
    dac.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
        .unwrap();
    dac.write(0x01, AccessWidth::Byte, 0, SimTime::from_ticks(1))
        .unwrap();
    assert_eq!(handle.control_b(), 0x04);
}

#[test]
fn eic_latches_a_configured_rising_edge_until_write_one_to_clear() {
    let (mut eic, handle) = Samd21Eic::new("eic");
    eic.write(0x18, AccessWidth::Word, 1 << (3 * 4), SimTime::ZERO)
        .unwrap();
    eic.write(0x0c, AccessWidth::Word, 1 << 3, SimTime::ZERO)
        .unwrap();
    eic.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
        .unwrap();
    assert!(!handle.poll(0));
    assert!(handle.poll(1 << 3));
    assert_eq!(handle.flags(), 1 << 3);
    eic.write(0x10, AccessWidth::Word, 1 << 3, SimTime::ZERO)
        .unwrap();
    assert!(!handle.poll(1 << 3));
}

#[test]
fn watchdog_clear_restarts_the_functional_timeout() {
    let (mut wdt, handle) = Samd21Wdt::new("wdt");
    wdt.write(0x01, AccessWidth::Byte, 0, SimTime::ZERO)
        .unwrap();
    wdt.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
        .unwrap();
    assert!(!handle.take_reset(SimTime::from_ticks(7)));
    wdt.write(0x08, AccessWidth::Byte, 0xa5, SimTime::from_ticks(7))
        .unwrap();
    assert!(!handle.take_reset(SimTime::from_ticks(14)));
    assert!(handle.take_reset(SimTime::from_ticks(15)));
}
