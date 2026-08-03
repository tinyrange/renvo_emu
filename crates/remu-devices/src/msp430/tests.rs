use super::*;
#[test]
fn eusci_b0_register_ids_match_ti_map() {
    assert_eq!(Msp430EusciB0Register::Ctlw0.address(), 0x0540);
    assert_eq!(Msp430EusciB0Register::TbCnt.address(), 0x054a);
    assert_eq!(Msp430EusciB0Register::I2cSa.address(), 0x0560);
    assert_eq!(
        Msp430EusciB0Register::from_address(0x056e),
        Some(Msp430EusciB0Register::Iv)
    );
    assert_eq!(Msp430EusciB0Register::from_address(0x0544), None);
}

#[test]
fn clock_system_has_fr2433_reset_values_and_masks() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    for (address, expected) in [
        (CSCTL0, CSCTL0_RESET),
        (CSCTL1, CSCTL1_RESET),
        (CSCTL2, CSCTL2_RESET),
        (CSCTL3, CSCTL3_RESET),
        (CSCTL4, CSCTL4_RESET),
        (CSCTL5, CSCTL5_RESET),
        (CSCTL6, CSCTL6_RESET),
        (CSCTL7, CSCTL7_RESET),
        (CSCTL8, CSCTL8_RESET),
    ] {
        assert_eq!(
            device.read(address as u64, AccessWidth::HalfWord, SimTime::ZERO),
            Ok(expected.into())
        );
    }
    assert_eq!((handle.mclk_divider(), handle.smclk_divider()), (1, 1));
    assert_eq!((handle.fll_multiplier(), handle.mclk_source()), (0x1f, 0));
    device
        .write(
            CSCTL5 as u64,
            AccessWidth::HalfWord,
            u16::MAX.into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        device.read(CSCTL5 as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0x10f7)
    );
    assert_eq!((handle.mclk_divider(), handle.smclk_divider()), (128, 8));
    device
        .write(CSCTL2 as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.fll_multiplier(), 1);
}
#[test]
fn gpio_is_locked_until_pm5ctl0_is_cleared() {
    let hub = SignalHub::new();
    let (mut device, _handle, gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    device
        .write(PADIR as u64, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    device
        .write(PAOUT as u64, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(gpio[0].direction(), 0);
    device
        .write(PM5CTL0 as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
        .unwrap();
    assert_eq!(gpio[0].direction(), 1);
    assert_eq!(gpio[0].resolved(0).unwrap(), Logic::One);
}

#[test]
fn pmm_registers_follow_reset_values_and_password_gate() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    assert_eq!(
        device.read(PMMCTL0 as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0x9640)
    );
    assert_eq!(
        device.read(PMMCTL1 as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0x9600)
    );
    assert_eq!(
        device.read(PM5CTL0 as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0x0011)
    );
    assert!(!handle.pmm_unlocked());

    device
        .write(PMMCTL2 as u64, AccessWidth::HalfWord, 0xffff, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.take_pmm_reset(), Some(ResetKind::Software));
    assert_eq!(
        device.read(PMMCTL2 as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0)
    );

    device
        .write(
            (PMMCTL0 + 1) as u64,
            AccessWidth::Byte,
            u64::from(PMM_UNLOCK),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(handle.pmm_unlocked());
    device
        .write(
            PMMCTL0 as u64,
            AccessWidth::Byte,
            u64::from(PMMCTL0_REG_OFF | PMMCTL0_SVSHE),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        device.read(PMMCTL0 as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0x9650)
    );
    device
        .write(PMMCTL2 as u64, AccessWidth::HalfWord, 0xffff, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        device.read(PMMCTL2 as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0x003b)
    );
    device
        .write(PMMIFG as u64, AccessWidth::HalfWord, 0xffff, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        device.read(PMMIFG as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(PMMIFG_VALUE_MASK.into())
    );
    device
        .write(PMMIE as u64, AccessWidth::HalfWord, 0xffff, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        device.read(PMMIE as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0)
    );

    device
        .write((PMMCTL0 + 1) as u64, AccessWidth::Byte, 0, SimTime::ZERO)
        .unwrap();
    assert!(!handle.pmm_unlocked());
    device
        .write(PMMCTL2 as u64, AccessWidth::HalfWord, 0x55, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.take_pmm_reset(), Some(ResetKind::Software));
}

#[test]
fn pmm_software_resets_self_clear_and_classify_low_power_modes() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    device
        .write(
            PMMCTL0 as u64,
            AccessWidth::HalfWord,
            u64::from((u16::from(PMM_UNLOCK) << 8) | PMMCTL0_SWPOR),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(handle.take_pmm_reset(), Some(ResetKind::Software));
    assert_eq!(
        device.read(PMMCTL0 as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0x9600)
    );

    assert_eq!(handle.low_power_mode(0), Msp430LowPowerMode::Active);
    assert_eq!(handle.low_power_mode(1 << 4), Msp430LowPowerMode::Lpm0);
    assert_eq!(
        handle.low_power_mode((1 << 4) | (1 << 5)),
        Msp430LowPowerMode::Lpm1
    );
    assert_eq!(
        handle.low_power_mode((1 << 4) | (1 << 6)),
        Msp430LowPowerMode::Lpm2
    );
    assert_eq!(
        handle.low_power_mode((1 << 4) | (1 << 5) | (1 << 6)),
        Msp430LowPowerMode::Lpm3
    );

    device
        .write(
            PMMCTL0 as u64,
            AccessWidth::HalfWord,
            u64::from((u16::from(PMM_UNLOCK) << 8) | PMMCTL0_REG_OFF),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        handle.low_power_mode((1 << 4) | (1 << 5) | (1 << 6)),
        Msp430LowPowerMode::Lpm3_5
    );
    assert_eq!(
        handle.low_power_mode((1 << 4) | (1 << 5) | (1 << 6) | (1 << 7)),
        Msp430LowPowerMode::Lpm4_5
    );
}
#[test]
fn eusci_captures_a_transmitted_byte() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    device
        .write(UCA0CTLW0 as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
        .unwrap();
    device
        .write(
            UCA0TXBUF as u64,
            AccessWidth::HalfWord,
            b'R'.into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(handle.uart_bytes(), b"R");
}

#[test]
fn eusci_a1_uart_transmit_and_loopback_are_observable() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    device
        .write(UCA1CTLW0 as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
        .unwrap();
    device
        .write(
            UCA1STATW as u64,
            AccessWidth::Byte,
            UCLISTEN.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            UCA1TXBUF as u64,
            AccessWidth::HalfWord,
            b'A'.into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(handle.uart1_bytes(), b"A");
    assert!(handle.poll(SimTime::from_ticks(7)).is_empty());
    assert!(handle.poll(SimTime::from_ticks(8)).is_empty());
    assert_eq!(
        device.read(
            UCA1RXBUF as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(8)
        ),
        Ok(u64::from(b'A'))
    );
}

#[test]
fn eusci_listen_mode_loops_tx_back_to_rx() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    device
        .write(UCA0CTLW0 as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
        .unwrap();
    device
        .write(
            UCA0STATW as u64,
            AccessWidth::Byte,
            UCLISTEN.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(UCA0TXBUF as u64, AccessWidth::HalfWord, 0x5a, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.uart_bytes(), [0x5a]);
    assert!(handle.poll(SimTime::from_ticks(7)).is_empty());
    assert!(handle.poll(SimTime::from_ticks(8)).is_empty());
    assert_eq!(
        device.read(
            UCA0RXBUF as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(8)
        ),
        Ok(0x5a)
    );
    assert_eq!(
        device.read(UCA0IFG as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(UCTXIFG.into())
    );
}

#[test]
fn crc16_registers_accumulate_normal_and_bit_reversed_data() {
    let hub = SignalHub::new();
    let (mut device, _handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    device
        .write(
            CRCINIRES as u64,
            AccessWidth::HalfWord,
            0xffff,
            SimTime::ZERO,
        )
        .unwrap();
    for byte in b"123456789" {
        device
            .write(
                CRC16DI as u64,
                AccessWidth::Byte,
                u64::from(*byte),
                SimTime::ZERO,
            )
            .unwrap();
    }
    assert_eq!(
        device.read(CRCINIRES as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0x29b1)
    );
    assert_eq!(
        device.read(CRCRESR as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0x8d94)
    );
    device
        .write(CRCINIRES as u64, AccessWidth::HalfWord, 0, SimTime::ZERO)
        .unwrap();
    device
        .write(
            CRCDIRB as u64,
            AccessWidth::Byte,
            u64::from(b'1'),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        device.read(CRC16DI as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(u64::from(b'1'.reverse_bits()))
    );
}

#[test]
fn timer_a_instances_route_compare_and_overflow_vectors() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    for timer in 0..TIMER_BASES.len() {
        device
            .write(
                timer_register(timer, TIMER_CTL_OFFSET) as u64,
                AccessWidth::HalfWord,
                u64::from(0x12_u16),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                timer_register(timer, TIMER_CCR0_OFFSET) as u64,
                AccessWidth::HalfWord,
                3,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                timer_register(timer, TIMER_CCTL0_OFFSET + 2) as u64,
                AccessWidth::HalfWord,
                u64::from(CCIE),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                timer_register(timer, TIMER_CCR0_OFFSET + 2) as u64,
                AccessWidth::HalfWord,
                2,
                SimTime::ZERO,
            )
            .unwrap();
    }
    let vectors = handle.poll(SimTime::from_ticks(2));
    for vector in MSP430_TIMER_A1_VECTORS {
        assert!(
            vectors.contains(&vector),
            "missing Timer_A vector {vector:#x}"
        );
    }
    for timer in 0..TIMER_BASES.len() {
        assert_eq!(
            device.read(
                timer_register(timer, TIMER_IV_OFFSET) as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(2),
            ),
            Ok(2)
        );
    }
    let vectors = handle.poll(SimTime::from_ticks(4));
    for vector in MSP430_TIMER_A1_VECTORS {
        assert!(
            vectors.contains(&vector),
            "missing overflow vector {vector:#x}"
        );
    }
    for timer in 0..TIMER_BASES.len() {
        assert_eq!(
            device.read(
                timer_register(timer, TIMER_IV_OFFSET) as u64,
                AccessWidth::HalfWord,
                SimTime::from_ticks(4),
            ),
            Ok(10)
        );
    }
}

#[test]
fn rtc_modulo_counter_sets_and_clears_overflow_interrupt() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    device
        .write(RTCMOD as u64, AccessWidth::HalfWord, 3, SimTime::ZERO)
        .unwrap();
    device
        .write(
            RTCCTL as u64,
            AccessWidth::HalfWord,
            u64::from(RTCSS_MASK | RTCIE | RTCSR),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(handle.poll(SimTime::from_ticks(3)).is_empty());
    assert_eq!(
        device.read(RTCCNT as u64, AccessWidth::HalfWord, SimTime::from_ticks(3)),
        Ok(3)
    );
    assert!(
        handle
            .poll(SimTime::from_ticks(4))
            .contains(&MSP430_RTC_VECTOR)
    );
    assert_eq!(
        device.read(RTCIV as u64, AccessWidth::HalfWord, SimTime::from_ticks(4)),
        Ok(2)
    );
    assert_eq!(
        device.read(RTCCTL as u64, AccessWidth::HalfWord, SimTime::from_ticks(4)),
        Ok((RTCSS_MASK | RTCIE).into())
    );
}

#[test]
fn eusci_b0_spi_transfer_exposes_miso_and_interrupt_state() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(UCSYNC | UCMST),
            SimTime::ZERO,
        )
        .unwrap();
    handle.inject_spi0_rx(0xa5);
    device
        .write(UCB0TXBUF as u64, AccessWidth::HalfWord, 0x3c, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.spi0_bytes(), [0x3c]);
    assert_eq!(
        device.read(UCB0RXBUF as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0xa5)
    );
    device
        .write(
            UCB0IE as u64,
            AccessWidth::HalfWord,
            u64::from(UCRXIFG | UCTXIFG),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(handle.poll(SimTime::ZERO), vec![MSP430_USCI_B0_VECTOR]);
    assert_eq!(
        device.read(UCB0IV as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(0x18)
    );
}

#[test]
fn eusci_b0_i2c_host_records_write_start_and_stop() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    let base = UCSYNC | UCMODE_I2C | UCMST;
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base | UCSWRST),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(UCB0I2CSA as u64, AccessWidth::HalfWord, 0x50, SimTime::ZERO)
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base | UCTR | UCTXSTT),
            SimTime::from_ticks(1),
        )
        .unwrap();
    device
        .write(
            UCB0TXBUF as u64,
            AccessWidth::HalfWord,
            0x10,
            SimTime::from_ticks(2),
        )
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base | UCTR | UCTXSTP),
            SimTime::from_ticks(3),
        )
        .unwrap();
    assert_eq!(
        handle.i2c_events(),
        [
            Msp430I2cEvent::Start,
            Msp430I2cEvent::Write {
                address: 0x50,
                value: 0x10,
            },
            Msp430I2cEvent::Stop,
        ]
    );
}

#[test]
fn adc10_single_conversion_uses_injected_channel_and_interrupt() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    handle.set_adc_input(3, 0x2aa);
    device
        .write(ADCMCTL0 as u64, AccessWidth::HalfWord, 3, SimTime::ZERO)
        .unwrap();
    device
        .write(
            ADCIE as u64,
            AccessWidth::HalfWord,
            u64::from(ADCIFG0),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            ADCCTL0 as u64,
            AccessWidth::HalfWord,
            u64::from(ADCON | ADCENC | ADCSC),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        device.read(ADCCTL1 as u64, AccessWidth::HalfWord, SimTime::ZERO),
        Ok(u64::from(ADCBUSY))
    );
    assert!(handle.poll(SimTime::from_ticks(3)).is_empty());
    assert_eq!(handle.poll(SimTime::from_ticks(4)), vec![MSP430_ADC_VECTOR]);
    assert_eq!(
        device.read(
            ADCMEM0 as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(4)
        ),
        Ok(0x2aa)
    );
    assert_eq!(
        device.read(ADCIV as u64, AccessWidth::HalfWord, SimTime::from_ticks(4)),
        Ok(2)
    );
    assert!(handle.poll(SimTime::from_ticks(5)).is_empty());
}

#[test]
fn eusci_b0_i2c_host_supplies_queued_read_and_interrupt() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    let base = UCSYNC | UCMODE_I2C | UCMST;
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base | UCSWRST),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(UCB0I2CSA as u64, AccessWidth::HalfWord, 0x44, SimTime::ZERO)
        .unwrap();
    handle.queue_i2c_read(0x44, [0x42, 0x43]);
    device
        .write(
            UCB0IE as u64,
            AccessWidth::HalfWord,
            u64::from(UCRXIFG),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base | UCTXSTT),
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert_eq!(handle.poll(SimTime::from_ticks(1)), [MSP430_USCI_B0_VECTOR]);
    assert_eq!(
        device.read(
            UCB0RXBUF as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(2),
        ),
        Ok(0x42)
    );
    assert_eq!(
        device.read(
            UCB0RXBUF as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(3),
        ),
        Ok(0x43)
    );
    assert_eq!(
        handle.i2c_events(),
        [
            Msp430I2cEvent::Start,
            Msp430I2cEvent::Read {
                address: 0x44,
                value: 0x42,
            },
            Msp430I2cEvent::Read {
                address: 0x44,
                value: 0x43,
            },
        ]
    );
}

#[test]
fn eusci_b0_honors_reset_write_protection_and_reserved_bits() {
    let hub = SignalHub::new();
    let (mut device, _handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    device
        .write(
            UCB0CTLW1 as u64,
            AccessWidth::HalfWord,
            0x01ff,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            UCB0TBCNT as u64,
            AccessWidth::HalfWord,
            0x01ff,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(UCB0BRW as u64, AccessWidth::HalfWord, 0x1234, SimTime::ZERO)
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(UCMODE_I2C | UCMST | UCSYNC),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            UCB0CTLW1 as u64,
            AccessWidth::HalfWord,
            0,
            SimTime::from_ticks(1),
        )
        .unwrap();
    device
        .write(
            UCB0TBCNT as u64,
            AccessWidth::HalfWord,
            0,
            SimTime::from_ticks(1),
        )
        .unwrap();
    device
        .write(
            UCB0BRW as u64,
            AccessWidth::HalfWord,
            0,
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert_eq!(
        device.read(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(2)
        ),
        Ok(u64::from(UCMODE_I2C | UCMST | UCSYNC))
    );
    assert_eq!(
        device.read(
            UCB0CTLW1 as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(2)
        ),
        Ok(0x01ff)
    );
    assert_eq!(
        device.read(
            UCB0TBCNT as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(2)
        ),
        Ok(0x00ff)
    );
    assert_eq!(
        device.read(
            UCB0BRW as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(2)
        ),
        Ok(0x1234)
    );
}

#[test]
fn eusci_b0_nack_sets_error_and_iv_clears_it() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    let base = UCSYNC | UCMODE_I2C | UCMST;
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base | UCSWRST),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(UCB0I2CSA as u64, AccessWidth::HalfWord, 0x52, SimTime::ZERO)
        .unwrap();
    handle.set_i2c_ack(0x52, false);
    device
        .write(
            UCB0IE as u64,
            AccessWidth::HalfWord,
            u64::from(UCNACKIFG),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base | UCTXSTT),
            SimTime::from_ticks(1),
        )
        .unwrap();
    assert_eq!(handle.poll(SimTime::from_ticks(1)), [MSP430_USCI_B0_VECTOR]);
    assert_eq!(
        device.read(UCB0IV as u64, AccessWidth::HalfWord, SimTime::from_ticks(2)),
        Ok(0x04)
    );
    assert_eq!(
        device.read(
            UCB0IFG as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(2)
        ),
        Ok(UCTXIFG.into())
    );
    assert_eq!(
        handle.i2c_events(),
        [
            Msp430I2cEvent::Start,
            Msp430I2cEvent::Nack { address: 0x52 },
        ]
    );
}

#[test]
fn eusci_b0_byte_counter_can_generate_an_automatic_stop() {
    let hub = SignalHub::new();
    let (mut device, handle, _gpio) =
        Msp430Peripherals::new("fr2433", hub).expect("signals should construct");
    let base = UCSYNC | UCMODE_I2C | UCMST;
    device
        .write(
            UCB0CTLW1 as u64,
            AccessWidth::HalfWord,
            u64::from(UCASTP_STOP),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(UCB0TBCNT as u64, AccessWidth::HalfWord, 2, SimTime::ZERO)
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base | UCSWRST),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(UCB0I2CSA as u64, AccessWidth::HalfWord, 0x50, SimTime::ZERO)
        .unwrap();
    device
        .write(
            UCB0CTLW0 as u64,
            AccessWidth::HalfWord,
            u64::from(base | UCTR | UCTXSTT),
            SimTime::from_ticks(1),
        )
        .unwrap();
    device
        .write(
            UCB0TXBUF as u64,
            AccessWidth::HalfWord,
            0x10,
            SimTime::from_ticks(2),
        )
        .unwrap();
    device
        .write(
            UCB0TXBUF as u64,
            AccessWidth::HalfWord,
            0x20,
            SimTime::from_ticks(3),
        )
        .unwrap();
    assert_eq!(
        device.read(
            UCB0STATW as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(4)
        ),
        Ok(0x0200)
    );
    assert_eq!(
        device.read(
            UCB0IFG as u64,
            AccessWidth::HalfWord,
            SimTime::from_ticks(4)
        ),
        Ok((UCBCNTIFG | UCSTPIFG | UCTXIFG).into())
    );
    assert_eq!(
        handle.i2c_events(),
        [
            Msp430I2cEvent::Start,
            Msp430I2cEvent::Write {
                address: 0x50,
                value: 0x10,
            },
            Msp430I2cEvent::Write {
                address: 0x50,
                value: 0x20,
            },
            Msp430I2cEvent::Stop,
        ]
    );
}
