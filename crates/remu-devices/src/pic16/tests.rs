use super::*;

#[test]
fn timer2_register_ids_are_named_and_native() {
    assert_eq!(Pic16Timer2Register::ALL.len(), 6);
    assert_eq!(Pic16Timer2Register::T2Con.offset(), 0x28e);
    assert_eq!(Pic16Timer2Register::T2Con.index(), 0x28e);
    assert_eq!(Pic16Timer2Register::T2Con.name(), "t2con");
    assert_eq!(
        Pic16Timer2Register::from_data_address(0x71a),
        Some(Pic16Timer2Register::Pie4)
    );
    assert_eq!(Pic16Timer2Register::from_data_address(0x28f), None);
}

#[test]
fn dac_register_ids_are_named_and_stable() {
    assert_eq!(Pic16DacRegister::ALL.len(), 2);
    assert_eq!(Pic16DacRegister::Dac1Con0.offset(), 0x90e);
    assert_eq!(Pic16DacRegister::Dac1Con0.index(), 0x90e);
    assert_eq!(Pic16DacRegister::Dac1Con0.name(), "dac1con0");
    assert_eq!(
        Pic16DacRegister::from_data_address(0x90f),
        Some(Pic16DacRegister::Dac1Con1)
    );
    assert_eq!(Pic16DacRegister::from_data_address(0x90d), None);
}

#[test]
fn comparator_register_ids_are_named_and_stable() {
    assert_eq!(Pic16ComparatorRegister::ALL.len(), 7);
    assert_eq!(Pic16ComparatorRegister::Pir2.offset(), 0x70e);
    assert_eq!(Pic16ComparatorRegister::Cm1Con0.index(), 0x990);
    assert_eq!(Pic16ComparatorRegister::Cm1Con0.name(), "cm1con0");
    assert_eq!(
        Pic16ComparatorRegister::from_data_address(0x993),
        Some(Pic16ComparatorRegister::Cm1Pch)
    );
    assert_eq!(Pic16ComparatorRegister::from_data_address(0x994), None);
}

#[test]
fn nco_registers_are_named_and_match_the_documented_map() {
    assert_eq!(Pic16NcoRegister::ALL.len(), 10);
    for (index, register) in Pic16NcoRegister::ALL.into_iter().enumerate() {
        assert_eq!(register.index(), index);
        assert_eq!(
            Pic16NcoRegister::from_data_address(register.offset()),
            Some(register)
        );
        assert!(!register.name().is_empty());
    }

    let hub = SignalHub::new();
    let (mut device, _handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    assert_eq!(
        device
            .read(
                Pic16NcoRegister::Nco1Incl.offset() as u64,
                AccessWidth::Byte,
                SimTime::ZERO,
            )
            .unwrap(),
        1
    );
    device
        .write(
            Pic16NcoRegister::Nco1Con.offset() as u64,
            AccessWidth::Byte,
            u64::from(NCO1EN | NCO1POL | NCO1OUT | 0x0e),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        device
            .read(
                Pic16NcoRegister::Nco1Con.offset() as u64,
                AccessWidth::Byte,
                SimTime::ZERO,
            )
            .unwrap(),
        u64::from(NCO1EN | NCO1OUT | NCO1POL)
    );
}

#[test]
fn mssp1_register_ids_cover_the_native_window() {
    assert_eq!(
        Pic16Mssp1Register::from_offset(0x18c),
        Some(Pic16Mssp1Register::Buffer)
    );
    assert_eq!(
        Pic16Mssp1Register::Control3.offset(),
        0x192,
        "the typed ID must retain the PIC16 native offset"
    );
    assert_eq!(Pic16Mssp1Register::from_offset(0x193), None);
}

#[test]
fn gpio_uart_timer_and_watchdog_slice_is_functional() {
    let hub = SignalHub::new();
    let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    device
        .write(ANSEL[0] as u64, AccessWidth::Byte, 0, SimTime::ZERO)
        .unwrap();
    device
        .write(TRIS_BASE as u64, AccessWidth::Byte, 0xfe, SimTime::ZERO)
        .unwrap();
    device
        .write(LAT_BASE as u64, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(ports[0].output() & 1, 1);

    device
        .write(RC1STA as u64, AccessWidth::Byte, SPEN.into(), SimTime::ZERO)
        .unwrap();
    device
        .write(TX1STA as u64, AccessWidth::Byte, TXEN.into(), SimTime::ZERO)
        .unwrap();
    device
        .write(TX1REG as u64, AccessWidth::Byte, b'P'.into(), SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.uart_bytes(), b"P");

    device
        .write(TMR0H as u64, AccessWidth::Byte, 3, SimTime::ZERO)
        .unwrap();
    device
        .write(PIE0 as u64, AccessWidth::Byte, TMR0IF.into(), SimTime::ZERO)
        .unwrap();
    device
        .write(
            INTCON as u64,
            AccessWidth::Byte,
            (INTCON_GIE | INTCON_PEIE).into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(T0CON0 as u64, AccessWidth::Byte, 0x80, SimTime::ZERO)
        .unwrap();
    assert!(handle.poll(SimTime::from_ticks(4)));
}

#[test]
fn mssp1_spi_master_transfer_exposes_loopback_and_interrupt_state() {
    let hub = SignalHub::new();
    let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();

    device
        .write(
            SSP1CON1 as u64,
            AccessWidth::Byte,
            SSP1CON1_SSPEN.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(PIE3 as u64, AccessWidth::Byte, SSP1IE.into(), SimTime::ZERO)
        .unwrap();
    device
        .write(
            INTCON as u64,
            AccessWidth::Byte,
            (INTCON_GIE | INTCON_PEIE).into(),
            SimTime::ZERO,
        )
        .unwrap();
    handle.inject_spi_rx(0xa5, SimTime::ZERO);
    device
        .write(SSP1BUF as u64, AccessWidth::Byte, 0x3c, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.spi_bytes(), vec![0x3c]);
    assert_eq!(
        device
            .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0xa5
    );
    assert_eq!(
        device
            .read(SSP1STAT as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap()
            & u64::from(SSP1STAT_BF),
        0
    );
    assert!(handle.poll(SimTime::from_ticks(1)));

    handle.inject_spi_rx(0x5a, SimTime::from_ticks(1));
    device
        .write(
            SSP1BUF as u64,
            AccessWidth::Byte,
            0xc3,
            SimTime::from_ticks(1),
        )
        .unwrap();
    device
        .write(
            SSP1BUF as u64,
            AccessWidth::Byte,
            0xff,
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert_eq!(
        device
            .read(SSP1CON1 as u64, AccessWidth::Byte, SimTime::from_ticks(1))
            .unwrap()
            & u64::from(SSP1CON1_WCOL),
        u64::from(SSP1CON1_WCOL)
    );
    assert_eq!(handle.spi_bytes(), vec![0x3c, 0xc3]);
}

#[test]
fn adc_conversion_formats_result_and_sets_interrupt() {
    let hub = SignalHub::new();
    let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    handle.set_adc_input(3, 0x2a5);
    device
        .write(PIE1 as u64, AccessWidth::Byte, ADIE.into(), SimTime::ZERO)
        .unwrap();
    device
        .write(
            INTCON as u64,
            AccessWidth::Byte,
            (INTCON_GIE | INTCON_PEIE).into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(ADCON1 as u64, AccessWidth::Byte, 1 << 7, SimTime::ZERO)
        .unwrap();
    device
        .write(
            ADCON0 as u64,
            AccessWidth::Byte,
            ((3 << 2) | ADCON0_GO | ADCON0_ADON).into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(!handle.poll(SimTime::ZERO));
    assert!(handle.poll(SimTime::from_ticks(1)));
    assert_eq!(
        device
            .read(ADRESL as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0xa5
    );
    assert_eq!(
        device
            .read(ADRESH as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x02
    );
    assert_eq!(
        device
            .read(ADCON0 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap()
            & u64::from(ADCON0_GO),
        0
    );
}

#[test]
fn timer2_period_match_honors_prescaler_and_postscaler() {
    let hub = SignalHub::new();
    let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    device
        .write(
            Pic16Timer2Register::T2Pr.offset() as u64,
            AccessWidth::Byte,
            2,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Pic16Timer2Register::Pie4.offset() as u64,
            AccessWidth::Byte,
            TMR2IF.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            INTCON as u64,
            AccessWidth::Byte,
            (INTCON_GIE | INTCON_PEIE).into(),
            SimTime::ZERO,
        )
        .unwrap();
    // CKPS=1:2 and OUTPS=1:2. A T2TMR-to-T2PR match occurs every
    // (2 + 1) * 2 ticks, and the interrupt is raised on the second match.
    device
        .write(
            Pic16Timer2Register::T2Con.offset() as u64,
            AccessWidth::Byte,
            (T2ON | (1 << 4) | 1).into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(!handle.poll(SimTime::from_ticks(5)));
    assert!(!handle.poll(SimTime::from_ticks(11)));
    assert!(handle.poll(SimTime::from_ticks(12)));
    device
        .write(
            Pic16Timer2Register::Pir4.offset() as u64,
            AccessWidth::Byte,
            0,
            SimTime::from_ticks(12),
        )
        .unwrap();
    assert!(!handle.poll(SimTime::from_ticks(13)));
}

#[test]
fn dac1_exposes_a_masked_code_and_enable_state() {
    let hub = SignalHub::new();
    let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    assert!(!handle.dac1_enabled());
    assert_eq!(handle.dac1_code(), 0);
    device
        .write(
            Pic16DacRegister::Dac1Con1.offset() as u64,
            AccessWidth::Byte,
            0xb5,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Pic16DacRegister::Dac1Con0.offset() as u64,
            AccessWidth::Byte,
            u64::from(DAC1EN | (1 << 5) | (1 << 2) | 1),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        device
            .read(
                Pic16DacRegister::Dac1Con0.offset() as u64,
                AccessWidth::Byte,
                SimTime::ZERO,
            )
            .unwrap(),
        u64::from(DAC1EN | (1 << 5) | (1 << 2))
    );
    assert!(handle.dac1_enabled());
    assert_eq!(handle.dac1_code(), 0x15);
    device
        .write(
            Pic16DacRegister::Dac1Con0.offset() as u64,
            AccessWidth::Byte,
            0,
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert!(!handle.dac1_enabled());
    assert_eq!(handle.dac1_code(), 0);
}

#[test]
fn comparator1_selects_gpio_inputs_and_latches_edge_interrupts() {
    let hub = SignalHub::new();
    let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    ports[0].set_input(0, Logic::Zero, SimTime::ZERO).unwrap(); // C1IN0-
    ports[0].set_input(2, Logic::One, SimTime::ZERO).unwrap(); // C1IN0+
    device
        .write(
            Pic16ComparatorRegister::Pie2.offset() as u64,
            AccessWidth::Byte,
            C1IF.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Pic16ComparatorRegister::Cm1Con1.offset() as u64,
            AccessWidth::Byte,
            0x02,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            INTCON as u64,
            AccessWidth::Byte,
            (INTCON_GIE | INTCON_PEIE).into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Pic16ComparatorRegister::Cm1Con0.offset() as u64,
            AccessWidth::Byte,
            C1ON.into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(handle.comparator1_output());
    assert!(handle.poll(SimTime::from_ticks(1)));
    assert_eq!(
        device
            .read(
                Pic16ComparatorRegister::Cm1Con0.offset() as u64,
                AccessWidth::Byte,
                SimTime::from_ticks(1),
            )
            .unwrap(),
        u64::from(C1ON | CM1CON0_OUT)
    );
    assert_eq!(
        device
            .read(
                Pic16ComparatorRegister::Cmout.offset() as u64,
                AccessWidth::Byte,
                SimTime::from_ticks(1),
            )
            .unwrap(),
        u64::from(CMOUT_C1OUT)
    );
    device
        .write(
            Pic16ComparatorRegister::Cmout.offset() as u64,
            AccessWidth::Byte,
            u8::MAX.into(),
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert_eq!(
        device
            .read(
                Pic16ComparatorRegister::Cmout.offset() as u64,
                AccessWidth::Byte,
                SimTime::from_ticks(1),
            )
            .unwrap(),
        u64::from(CMOUT_C1OUT)
    );

    device
        .write(
            Pic16ComparatorRegister::Pir2.offset() as u64,
            AccessWidth::Byte,
            0,
            SimTime::from_ticks(1),
        )
        .unwrap();
    ports[0]
        .set_input(2, Logic::Zero, SimTime::from_ticks(2))
        .unwrap();
    assert!(!handle.poll(SimTime::from_ticks(2)));
    assert!(!handle.comparator1_output());
}

#[test]
fn comparator1_stays_low_when_disabled_even_if_polarity_is_inverted() {
    let hub = SignalHub::new();
    let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    ports[0].set_input(0, Logic::Zero, SimTime::ZERO).unwrap();
    ports[0].set_input(2, Logic::One, SimTime::ZERO).unwrap();
    device
        .write(
            Pic16ComparatorRegister::Cm1Con0.offset() as u64,
            AccessWidth::Byte,
            C1POL.into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(!handle.comparator1_output());
    device
        .write(
            Pic16ComparatorRegister::Cm1Con0.offset() as u64,
            AccessWidth::Byte,
            u64::from(C1ON | C1POL),
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert!(!handle.comparator1_output());
    device
        .write(
            Pic16ComparatorRegister::Cm1Con0.offset() as u64,
            AccessWidth::Byte,
            C1ON.into(),
            SimTime::from_ticks(2),
        )
        .unwrap();
    assert!(handle.comparator1_output());
}

#[test]
fn pps_routes_timer0_and_eusart_strobes_to_gpio_outputs() {
    let hub = SignalHub::new();
    let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    device
        .write(ANSEL[0] as u64, AccessWidth::Byte, 0, SimTime::ZERO)
        .unwrap();
    device
        .write(TRIS_BASE as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
        .unwrap();
    device
        .write(
            Pic16PpsRegister::Ra0Pps.offset() as u64,
            AccessWidth::Byte,
            PPS_OUTPUT_TMR0.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(TMR0H as u64, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    device
        .write(T0CON0 as u64, AccessWidth::Byte, 0x80, SimTime::ZERO)
        .unwrap();
    assert_eq!(ports[0].output() & 1, 0);
    handle.poll(SimTime::from_ticks(2));
    assert_eq!(ports[0].output() & 1, 1);

    device
        .write(
            Pic16PpsRegister::Ra0Pps.offset() as u64,
            AccessWidth::Byte,
            PPS_OUTPUT_TX1.into(),
            SimTime::from_ticks(2),
        )
        .unwrap();
    device
        .write(
            RC1STA as u64,
            AccessWidth::Byte,
            SPEN.into(),
            SimTime::from_ticks(2),
        )
        .unwrap();
    device
        .write(
            TX1STA as u64,
            AccessWidth::Byte,
            TXEN.into(),
            SimTime::from_ticks(2),
        )
        .unwrap();
    device
        .write(
            TX1REG as u64,
            AccessWidth::Byte,
            b'P'.into(),
            SimTime::from_ticks(2),
        )
        .unwrap();
    handle.poll(SimTime::from_ticks(2));
    assert_eq!(ports[0].output() & 1, 1);
}

#[test]
fn pps_registers_are_named_cover_all_pins_and_honor_the_lock() {
    assert_eq!(Pic16PpsRegister::ALL.len(), 37);
    for (index, register) in Pic16PpsRegister::ALL.iter().copied().enumerate() {
        assert_eq!(register.index(), index);
        assert_eq!(
            Pic16PpsRegister::from_data_address(register.offset()),
            Some(register)
        );
    }
    assert_eq!(Pic16PpsRegister::Ra7Pps.port_pin(), Some((0, 7)));
    assert_eq!(Pic16PpsRegister::Re3Pps.port_pin(), Some((4, 3)));
    assert_eq!(
        Pic16PpsRegister::output(3, 7),
        Some(Pic16PpsRegister::Rd7Pps)
    );

    let hub = SignalHub::new();
    let (mut device, handle, ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    let at = SimTime::ZERO;
    device
        .write(ANSEL[0] as u64, AccessWidth::Byte, 0, at)
        .unwrap();
    device
        .write(TRIS_BASE as u64, AccessWidth::Byte, 0x7f, at)
        .unwrap();
    device
        .write(
            Pic16PpsRegister::Ra7Pps.offset() as u64,
            AccessWidth::Byte,
            PPS_OUTPUT_TMR0.into(),
            at,
        )
        .unwrap();
    device
        .write(TMR0H as u64, AccessWidth::Byte, 1, at)
        .unwrap();
    device
        .write(T0CON0 as u64, AccessWidth::Byte, 0x80, at)
        .unwrap();
    handle.poll(SimTime::from_ticks(2));
    assert_eq!(ports[0].output() & 0x80, 0x80);

    device
        .write(
            Pic16PpsRegister::Ppslock.offset() as u64,
            AccessWidth::Byte,
            PPSLOCKED.into(),
            at,
        )
        .unwrap();
    device
        .write(
            Pic16PpsRegister::Ra7Pps.offset() as u64,
            AccessWidth::Byte,
            0,
            at,
        )
        .unwrap();
    assert_eq!(
        device.read(
            Pic16PpsRegister::Ra7Pps.offset() as u64,
            AccessWidth::Byte,
            at
        ),
        Ok(u64::from(PPS_OUTPUT_TMR0))
    );
}

#[test]
fn nco1_accumulates_and_routes_overflow_interrupt() {
    let hub = SignalHub::new();
    let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    device
        .write(
            Pic16NcoRegister::Nco1Incu.offset() as u64,
            AccessWidth::Byte,
            0x0f,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Pic16NcoRegister::Nco1Inch.offset() as u64,
            AccessWidth::Byte,
            0xff,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Pic16NcoRegister::Nco1Incl.offset() as u64,
            AccessWidth::Byte,
            0xff,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Pic16NcoRegister::Pie7.offset() as u64,
            AccessWidth::Byte,
            NCO1IE.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            INTCON as u64,
            AccessWidth::Byte,
            (INTCON_GIE | INTCON_PEIE).into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Pic16NcoRegister::Nco1Con.offset() as u64,
            AccessWidth::Byte,
            NCO1EN.into(),
            SimTime::ZERO,
        )
        .unwrap();

    assert!(!handle.nco1_output());
    assert!(handle.poll(SimTime::from_ticks(2)));
    assert!(handle.nco1_output());
    assert_eq!(
        device
            .read(
                Pic16NcoRegister::Pir7.offset() as u64,
                AccessWidth::Byte,
                SimTime::from_ticks(2),
            )
            .unwrap() as u8
            & NCO1IF,
        NCO1IF
    );
}

#[test]
fn nco_fixed_duty_polarity_and_pulse_mode_are_observable() {
    let hub = SignalHub::new();
    let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    for (register, value) in [
        (Pic16NcoRegister::Nco1Incu, 0x04_u64),
        (Pic16NcoRegister::Nco1Inch, 0),
        (Pic16NcoRegister::Nco1Incl, 0),
    ] {
        device
            .write(
                register.offset() as u64,
                AccessWidth::Byte,
                value,
                SimTime::ZERO,
            )
            .unwrap();
    }
    device
        .write(
            Pic16NcoRegister::Nco1Con.offset() as u64,
            AccessWidth::Byte,
            NCO1EN.into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(!handle.nco1_output());
    assert!(!handle.poll(SimTime::from_ticks(4)));
    assert!(handle.nco1_output());

    device
        .write(
            Pic16NcoRegister::Nco1Con.offset() as u64,
            AccessWidth::Byte,
            u64::from(NCO1EN | NCO1POL),
            SimTime::from_ticks(4),
        )
        .unwrap();
    assert!(!handle.nco1_output());

    // A 1/4-scale increment overflows every four abstract input clocks.
    device
        .write(
            Pic16NcoRegister::Nco1Con.offset() as u64,
            AccessWidth::Byte,
            u64::from(NCO1EN | NCO1PFM),
            SimTime::from_ticks(4),
        )
        .unwrap();
    assert!(!handle.poll(SimTime::from_ticks(8)));
    assert!(handle.nco1_output());
    assert!(!handle.poll(SimTime::from_ticks(9)));
    assert!(!handle.nco1_output());
}

#[test]
fn mssp1_i2c_host_records_write_start_and_stop() {
    let hub = SignalHub::new();
    let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    device
        .write(
            SSP1CON1 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(PIE3 as u64, AccessWidth::Byte, SSP1IE.into(), SimTime::ZERO)
        .unwrap();
    device
        .write(
            INTCON as u64,
            AccessWidth::Byte,
            u64::from(INTCON_GIE | INTCON_PEIE),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            SSP1CON2_SEN.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            SSP1BUF as u64,
            AccessWidth::Byte,
            0xa0,
            SimTime::from_ticks(1),
        )
        .unwrap();
    device
        .write(
            SSP1BUF as u64,
            AccessWidth::Byte,
            0x10,
            SimTime::from_ticks(2),
        )
        .unwrap();
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            SSP1CON2_PEN.into(),
            SimTime::from_ticks(3),
        )
        .unwrap();
    assert_eq!(
        handle.i2c_events(),
        vec![
            Pic16I2cEvent::Start,
            Pic16I2cEvent::Write {
                address: 0x50,
                value: 0x10
            },
            Pic16I2cEvent::Stop,
        ]
    );
    assert!(handle.poll(SimTime::from_ticks(3)));
}

#[test]
fn mssp1_i2c_host_reads_queued_response_and_clears_bf() {
    let hub = SignalHub::new();
    let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    device
        .write(
            SSP1CON1 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
            SimTime::ZERO,
        )
        .unwrap();
    handle.queue_i2c_read(0x50, [0x42]);
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            SSP1CON2_SEN.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            SSP1BUF as u64,
            AccessWidth::Byte,
            0xa1,
            SimTime::from_ticks(1),
        )
        .unwrap();
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            SSP1CON2_RCEN.into(),
            SimTime::from_ticks(2),
        )
        .unwrap();
    assert_ne!(
        device
            .read(SSP1STAT as u64, AccessWidth::Byte, SimTime::from_ticks(2))
            .unwrap()
            & u64::from(SSP1STAT_BF),
        0
    );
    assert_eq!(
        device
            .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::from_ticks(3))
            .unwrap(),
        0x42
    );
    assert_eq!(
        device
            .read(SSP1STAT as u64, AccessWidth::Byte, SimTime::from_ticks(3))
            .unwrap()
            & u64::from(SSP1STAT_BF),
        0
    );
    assert_eq!(
        handle.i2c_events(),
        vec![
            Pic16I2cEvent::Start,
            Pic16I2cEvent::Read {
                address: 0x50,
                value: 0x42
            }
        ]
    );
}

#[test]
fn mssp1_i2c_master_reports_ackstat_and_ack_sequence() {
    let hub = SignalHub::new();
    let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    device
        .write(
            SSP1CON1 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
            SimTime::ZERO,
        )
        .unwrap();
    handle.set_i2c_ack(0x50, false);
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON2_SEN),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            SSP1BUF as u64,
            AccessWidth::Byte,
            0xa0,
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert_ne!(
        device
            .read(SSP1CON2 as u64, AccessWidth::Byte, SimTime::from_ticks(1))
            .unwrap()
            & u64::from(SSP1CON2_ACKSTAT),
        0,
        "a configured NACK must be visible through ACKSTAT"
    );

    handle.set_i2c_ack(0x50, true);
    handle.queue_i2c_read(0x50, [0x42]);
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON2_RSEN),
            SimTime::from_ticks(2),
        )
        .unwrap();
    device
        .write(
            SSP1BUF as u64,
            AccessWidth::Byte,
            0xa1,
            SimTime::from_ticks(3),
        )
        .unwrap();
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON2_RCEN),
            SimTime::from_ticks(4),
        )
        .unwrap();
    assert_eq!(
        device
            .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::from_ticks(5))
            .unwrap(),
        0x42
    );
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON2_ACKDT | SSP1CON2_ACKEN),
            SimTime::from_ticks(6),
        )
        .unwrap();
    assert_eq!(
        handle.i2c_events().last(),
        Some(&Pic16I2cEvent::Ack { acknowledge: false })
    );
    assert_eq!(
        device
            .read(SSP1CON2 as u64, AccessWidth::Byte, SimTime::from_ticks(6))
            .unwrap()
            & u64::from(SSP1CON2_ACKEN),
        0
    );
}

#[test]
fn mssp1_i2c_master_rejects_queued_commands_and_preserves_receive_buffer() {
    let hub = SignalHub::new();
    let (mut device, handle, _ports) = Pic16Peripherals::new("pic16f15376.data", hub).unwrap();
    device
        .write(
            SSP1CON1 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON1_SSPEN | SSP1_I2C_MASTER_7BIT),
            SimTime::ZERO,
        )
        .unwrap();
    handle.queue_i2c_read(0x50, [0x10, 0x20]);
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON2_SEN),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            SSP1BUF as u64,
            AccessWidth::Byte,
            0xa1,
            SimTime::from_ticks(1),
        )
        .unwrap();
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON2_RCEN),
            SimTime::from_ticks(2),
        )
        .unwrap();
    device
        .write(
            SSP1CON2 as u64,
            AccessWidth::Byte,
            u64::from(SSP1CON2_SEN | SSP1CON2_PEN),
            SimTime::from_ticks(3),
        )
        .unwrap();
    assert_ne!(
        device
            .read(SSP1CON1 as u64, AccessWidth::Byte, SimTime::from_ticks(3))
            .unwrap()
            & u64::from(SSP1CON1_WCOL),
        0
    );
    assert_eq!(
        device
            .read(SSP1BUF as u64, AccessWidth::Byte, SimTime::from_ticks(4))
            .unwrap(),
        0x10
    );
}

#[test]
fn clock_reference_masks_registers_and_emits_deterministic_output() {
    let hub = SignalHub::new();
    let (mut device, handle, _) = Pic16Peripherals::new("pic16f15376.data", hub.clone()).unwrap();
    let clkr = hub
        .with_registry(|registry| registry.find("board.pic16f15376.clkr"))
        .expect("CLKR signal is declared");

    assert_eq!(
        device
            .read(CLKRCON as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x08
    );
    assert_eq!(
        device
            .read(CLKRCLK as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0
    );
    device
        .write(CLKRCON as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
        .unwrap();
    device
        .write(CLKRCLK as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        device
            .read(CLKRCON as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        CLKRCON_WRITABLE_MASK.into()
    );
    assert_eq!(
        device
            .read(CLKRCLK as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        CLKRCLK_WRITABLE_MASK.into()
    );

    handle.poll(SimTime::ZERO);
    assert_eq!(
        hub.with_registry(|registry| registry.value(clkr).and_then(|value| value.bit(0))),
        Some(Logic::Zero)
    );
    hub.drain_changes();

    // FOSC, /2, 50% duty: an eight-tick functional period.
    device
        .write(CLKRCLK as u64, AccessWidth::Byte, 0, SimTime::ZERO)
        .unwrap();
    device
        .write(CLKRCON as u64, AccessWidth::Byte, 0x91, SimTime::ZERO)
        .unwrap();
    let changes = hub.drain_changes();
    assert_eq!(changes.last().map(|change| change.signal), Some(clkr));
    assert_eq!(
        changes.last().and_then(|change| change.value.bit(0)),
        Some(Logic::One)
    );

    handle.poll(SimTime::from_ticks(3));
    assert!(hub.drain_changes().is_empty());
    handle.poll(SimTime::from_ticks(4));
    let falling = hub.drain_changes();
    assert_eq!(
        falling.last().and_then(|change| change.value.bit(0)),
        Some(Logic::Zero)
    );
    handle.poll(SimTime::from_ticks(8));
    let rising = hub.drain_changes();
    assert_eq!(
        rising.last().and_then(|change| change.value.bit(0)),
        Some(Logic::One)
    );

    device
        .write(CLKRCON as u64, AccessWidth::Byte, 0, SimTime::from_ticks(9))
        .unwrap();
    let disabled = hub.drain_changes();
    assert_eq!(
        disabled.last().and_then(|change| change.value.bit(0)),
        Some(Logic::Zero)
    );
}
