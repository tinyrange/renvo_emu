use super::*;

const DDRB: u16 = 0x24;

#[test]
fn comparator_register_ids_are_named_and_native() {
    assert_eq!(AtmegaComparatorRegister::ALL.len(), 1);
    assert_eq!(AtmegaComparatorRegister::Acsr.offset(), 0x50);
    assert_eq!(AtmegaComparatorRegister::Acsr.io_offset(), 0x30);
    assert_eq!(AtmegaComparatorRegister::Acsr.name(), "acsr");
    assert_eq!(
        AtmegaComparatorRegister::from_data_address(0x50),
        Some(AtmegaComparatorRegister::Acsr)
    );
    assert_eq!(AtmegaComparatorRegister::from_data_address(0x51), None);
}

#[test]
fn timer_register_ids_are_named_and_native() {
    assert_eq!(AtmegaTimerRegister::ALL.len(), 14);
    assert_eq!(AtmegaTimerRegister::Tccr3b.offset(), 0x91);
    assert_eq!(AtmegaTimerRegister::Tccr3b.io_offset(), 0x71);
    assert_eq!(AtmegaTimerRegister::Tccr3b.name(), "tccr3b");
    assert_eq!(
        AtmegaTimerRegister::from_data_address(0xa9),
        Some(AtmegaTimerRegister::Ocr4ah)
    );
    assert_eq!(AtmegaTimerRegister::from_data_address(0x93), None);
}

#[test]
fn pb_ports_uart_timer_and_persistent_eeprom_are_functional() {
    let hub = SignalHub::new();
    let (mut io, handle, ports) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    io.write(
        u64::from(DDRB - IO_BASE),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(PORTB - IO_BASE),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(ports[0].output(), 1);
    io.write(
        u64::from(UDR0 - IO_BASE),
        AccessWidth::Byte,
        b'A'.into(),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.uart_bytes(), b"A");
    io.write(
        u64::from(OCR0A - IO_BASE),
        AccessWidth::Byte,
        3,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(TIMSK0 - IO_BASE),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(TCCR0B - IO_BASE),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.poll(SimTime::from_ticks(4)), vec![15]);
    io.write(
        u64::from(AtmegaTimerRegister::Ocr3al.io_offset()),
        AccessWidth::Byte,
        3,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Timsk3.io_offset()),
        AccessWidth::Byte,
        1 << 1,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Tccr3b.io_offset()),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.poll(SimTime::from_ticks(4)), vec![15, 32]);
    io.write(
        u64::from(AtmegaTimerRegister::Tifr3.io_offset()),
        AccessWidth::Byte,
        1 << 1,
        SimTime::from_ticks(4),
    )
    .unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Ocr4al.io_offset()),
        AccessWidth::Byte,
        2,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Timsk4.io_offset()),
        AccessWidth::Byte,
        1 << 1,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Tccr4b.io_offset()),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.poll(SimTime::from_ticks(4)).contains(&41));
}

#[test]
fn pin_change_groups_and_int1_report_distinct_interrupt_lines() {
    let hub = SignalHub::new();
    let (mut io, handle, ports) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    io.write(
        u64::from(PCICR - IO_BASE),
        AccessWidth::Byte,
        (1 << 1) | (1 << 2),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(PCMSK1 - IO_BASE),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(PCMSK2 - IO_BASE),
        AccessWidth::Byte,
        1 << 4,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(EICRA - IO_BASE),
        AccessWidth::Byte,
        3 << 4,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(EIMSK - IO_BASE),
        AccessWidth::Byte,
        1 << 1,
        SimTime::ZERO,
    )
    .unwrap();
    ports[1].set_input(0, Logic::Zero, SimTime::ZERO).unwrap();
    ports[2].set_input(3, Logic::Zero, SimTime::ZERO).unwrap();
    ports[2].set_input(4, Logic::Zero, SimTime::ZERO).unwrap();
    assert!(handle.poll(SimTime::ZERO).is_empty());
    ports[1]
        .set_input(0, Logic::One, SimTime::from_ticks(1))
        .unwrap();
    ports[2]
        .set_input(3, Logic::One, SimTime::from_ticks(1))
        .unwrap();
    ports[2]
        .set_input(4, Logic::One, SimTime::from_ticks(1))
        .unwrap();
    let lines = handle.poll(SimTime::from_ticks(1));
    assert!(lines.contains(&1));
    assert!(lines.contains(&3));
    assert!(lines.contains(&4));
    io.write(
        u64::from(PCIFR - IO_BASE),
        AccessWidth::Byte,
        (1 << 1) | (1 << 2),
        SimTime::from_ticks(1),
    )
    .unwrap();
    io.write(
        u64::from(EIFR - IO_BASE),
        AccessWidth::Byte,
        1 << 1,
        SimTime::from_ticks(1),
    )
    .unwrap();
    assert_eq!(
        io.read(
            u64::from(PCIFR - IO_BASE),
            AccessWidth::Byte,
            SimTime::from_ticks(1),
        )
        .unwrap(),
        0
    );
    assert_eq!(
        io.read(
            u64::from(EIFR - IO_BASE),
            AccessWidth::Byte,
            SimTime::from_ticks(1),
        )
        .unwrap(),
        0
    );
}

#[test]
fn timer2_ctc_sets_and_clears_its_compare_interrupt() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub.clone()).unwrap();
    let timer2_irq = hub
        .with_registry(|registry| registry.find("board.atmega328pb.timer2.irq"))
        .expect("Timer2 IRQ signal is declared");
    io.write(
        u64::from(OCR2A - IO_BASE),
        AccessWidth::Byte,
        3,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(TCCR2A - IO_BASE),
        AccessWidth::Byte,
        2,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(TIMSK2 - IO_BASE),
        AccessWidth::Byte,
        1 << 1,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(TCCR2B - IO_BASE),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();

    assert!(handle.poll(SimTime::from_ticks(3)).is_empty());
    assert_eq!(
        handle.poll(SimTime::from_ticks(4)),
        vec![6],
        "TIMER2_COMPA is AVR vector 8 / emulator interrupt line 6"
    );
    assert!(
        hub.drain_changes()
            .iter()
            .any(|change| change.signal == timer2_irq && change.value.bit(0) == Some(Logic::One))
    );
    assert_eq!(
        io.read(
            u64::from(TIFR2 - IO_BASE),
            AccessWidth::Byte,
            SimTime::from_ticks(4)
        )
        .unwrap(),
        1 << 1
    );

    io.write(
        u64::from(TIFR2 - IO_BASE),
        AccessWidth::Byte,
        1 << 1,
        SimTime::from_ticks(4),
    )
    .unwrap();
    assert!(handle.poll(SimTime::from_ticks(4)).is_empty());
    assert!(
        hub.drain_changes()
            .iter()
            .any(|change| change.signal == timer2_irq && change.value.bit(0) == Some(Logic::Zero))
    );
}

#[test]
fn spi0_master_transfer_returns_injected_byte_and_interrupts() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    handle.inject_spi_rx(0x3c);
    io.write(
        u64::from(SPCR0 - IO_BASE),
        AccessWidth::Byte,
        u64::from(SPCR_SPIE | SPCR_SPE),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(SPDR0 - IO_BASE),
        AccessWidth::Byte,
        0xa5,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.spi_bytes(), [0xa5]);
    assert_eq!(
        handle.poll(SimTime::from_ticks(1)),
        vec![SPI0_INTERRUPT_LINE]
    );
    assert_eq!(
        io.read(u64::from(SPDR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
            .unwrap(),
        0x3c
    );
    assert_eq!(
        io.read(u64::from(SPSR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
            .unwrap()
            & u64::from(SPSR_SPIF),
        u64::from(SPSR_SPIF)
    );
    assert_eq!(
        io.read(u64::from(SPDR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
            .unwrap(),
        0x3c
    );
    assert_eq!(
        io.read(u64::from(SPSR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
            .unwrap()
            & u64::from(SPSR_SPIF),
        0
    );
    assert!(handle.poll(SimTime::from_ticks(1)).is_empty());
}

#[test]
fn spi0_write_collision_requires_an_unacknowledged_transfer() {
    let hub = SignalHub::new();
    let (mut io, _, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    io.write(
        u64::from(SPDR0 - IO_BASE),
        AccessWidth::Byte,
        0x11,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        io.read(u64::from(SPSR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
            .unwrap()
            & u64::from(SPSR_WCOL),
        0
    );

    io.write(
        u64::from(SPCR0 - IO_BASE),
        AccessWidth::Byte,
        u64::from(SPCR_SPE),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(SPDR0 - IO_BASE),
        AccessWidth::Byte,
        0x22,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(SPDR0 - IO_BASE),
        AccessWidth::Byte,
        0x33,
        SimTime::ZERO,
    )
    .unwrap();
    let status = io
        .read(u64::from(SPSR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        status & u64::from(SPSR_SPIF | SPSR_WCOL),
        u64::from(SPSR_SPIF | SPSR_WCOL)
    );
    assert_eq!(
        io.read(u64::from(SPDR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
            .unwrap(),
        0x22
    );
}

#[test]
fn twi0_start_transmit_and_receive_have_deterministic_status() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    let twcr = u64::from(TWCR - IO_BASE);
    io.write(
        u64::from(TWBR - IO_BASE),
        AccessWidth::Byte,
        12,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(TWAR - IO_BASE),
        AccessWidth::Byte,
        0x22,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(TWAMR - IO_BASE),
        AccessWidth::Byte,
        0,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(twcr, AccessWidth::Byte, 0xA5 | 0x20, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        io.read(u64::from(TWSR - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x08
    );
    io.write(
        u64::from(TWDR - IO_BASE),
        AccessWidth::Byte,
        0x55,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(twcr, AccessWidth::Byte, 0x85, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.take_twi_tx(), vec![0x55]);
    handle.queue_twi_rx(0xa5);
    io.write(twcr, AccessWidth::Byte, 0xc5, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        io.read(u64::from(TWDR - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0xa5
    );
    assert_eq!(
        io.read(u64::from(TWSR - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x50
    );
    assert!(handle.poll(SimTime::ZERO).contains(&24));
}

#[test]
fn twi0_reset_values_and_reserved_bits_match_datasheet() {
    let hub = SignalHub::new();
    let (mut io, _, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    let read = |io: &mut AtmegaIo, register: u16| {
        io.read(
            u64::from(register - IO_BASE),
            AccessWidth::Byte,
            SimTime::ZERO,
        )
        .unwrap() as u8
    };
    assert_eq!(read(&mut io, TWSR), TWI_STATUS_RESET);
    assert_eq!(read(&mut io, TWAR), 0x02);
    assert_eq!(read(&mut io, TWDR), 0x01);
    assert_eq!(read(&mut io, TWAMR), 0);
    assert_eq!(read(&mut io, TWCR), 0);

    io.write(
        u64::from(TWSR - IO_BASE),
        AccessWidth::Byte,
        0xff,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(read(&mut io, TWSR), 0xfb);
    io.write(
        u64::from(TWAMR - IO_BASE),
        AccessWidth::Byte,
        0xff,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(read(&mut io, TWAMR), 0xfe);
    io.write(
        u64::from(TWCR - IO_BASE),
        AccessWidth::Byte,
        0xff,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(read(&mut io, TWCR) & 0x02, 0);
    assert_eq!(read(&mut io, TWCR) & TWWC, 0);
}

#[test]
fn twi0_data_collision_sets_twwc_until_interrupt_is_set() {
    let hub = SignalHub::new();
    let (mut io, _, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    let twdr = u64::from(TWDR - IO_BASE);
    let twcr = u64::from(TWCR - IO_BASE);
    io.write(twdr, AccessWidth::Byte, 0x55, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        io.read(twdr, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x01
    );
    assert_ne!(
        io.read(twcr, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8 & TWWC,
        0
    );

    io.write(
        twcr,
        AccessWidth::Byte,
        u64::from(TWINT | TWSTA | TWEN),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(twdr, AccessWidth::Byte, 0x55, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        io.read(twdr, AccessWidth::Byte, SimTime::ZERO).unwrap(),
        0x55
    );
    assert_eq!(
        io.read(twcr, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8 & TWWC,
        0
    );
}

#[test]
fn twi0_stop_clears_twsto_without_requesting_interrupt() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    let twcr = u64::from(TWCR - IO_BASE);
    io.write(
        twcr,
        AccessWidth::Byte,
        u64::from(TWINT | TWSTA | TWEN | TWIE),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        twcr,
        AccessWidth::Byte,
        u64::from(TWINT | TWSTO | TWEN | TWIE),
        SimTime::ZERO,
    )
    .unwrap();
    let twcr_value = io.read(twcr, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8;
    assert_eq!(twcr_value & (TWSTO | TWINT), 0);
    assert!(!handle.poll(SimTime::ZERO).contains(&24));
    assert_eq!(
        io.read(u64::from(TWSR - IO_BASE), AccessWidth::Byte, SimTime::ZERO,)
            .unwrap(),
        u64::from(TWI_STATUS_RESET)
    );
}

#[test]
fn adc_conversion_latches_right_and_left_adjusted_results() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    handle.set_adc_input(3, 0x02aa);
    io.write(
        u64::from(ADMUX - IO_BASE),
        AccessWidth::Byte,
        3,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(ADCSRA - IO_BASE),
        AccessWidth::Byte,
        u64::from(ADEN | ADIE),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(ADCSRA - IO_BASE),
        AccessWidth::Byte,
        u64::from(ADEN | ADIE | ADSC),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.adc_value(), 0);
    assert_eq!(
        io.read(
            u64::from(ADCSRA - IO_BASE),
            AccessWidth::Byte,
            SimTime::ZERO
        )
        .unwrap() as u8
            & ADSC,
        ADSC
    );
    assert!(handle.poll(SimTime::from_ticks(49)).is_empty());
    assert_eq!(
        handle.poll(SimTime::from_ticks(50)),
        vec![ADC_INTERRUPT_LINE]
    );
    assert_eq!(handle.adc_value(), 0x02aa);
    assert_ne!(
        io.read(
            u64::from(ADCSRA - IO_BASE),
            AccessWidth::Byte,
            SimTime::ZERO
        )
        .unwrap() as u8
            & ADIF,
        0
    );
    io.write(
        u64::from(ADCSRA - IO_BASE),
        AccessWidth::Byte,
        u64::from(ADEN | ADIE | ADIF),
        SimTime::from_ticks(50),
    )
    .unwrap();
    assert!(handle.poll(SimTime::from_ticks(51)).is_empty());
    assert_eq!(
        io.read(
            u64::from(ADCSRA - IO_BASE),
            AccessWidth::Byte,
            SimTime::from_ticks(51)
        )
        .unwrap() as u8
            & ADIF,
        0
    );
    io.write(
        u64::from(ADMUX - IO_BASE),
        AccessWidth::Byte,
        u64::from(ADLAR | 3),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(ADCSRA - IO_BASE),
        AccessWidth::Byte,
        u64::from(ADEN | ADIE | ADIF | ADSC),
        SimTime::from_ticks(51),
    )
    .unwrap();
    assert!(handle.poll(SimTime::from_ticks(76)).is_empty());
    assert_eq!(
        handle.poll(SimTime::from_ticks(77)),
        vec![ADC_INTERRUPT_LINE]
    );
    assert_eq!(
        io.read(u64::from(ADCH - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0xaa
    );
    assert_eq!(
        io.read(u64::from(ADCL - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x80
    );
}

#[test]
fn adc_does_not_alias_reserved_mux_values_and_preserves_external_inputs_on_reset() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    handle.set_adc_input(0, 0x03ff);
    io.write(
        u64::from(ADMUX - IO_BASE),
        AccessWidth::Byte,
        8,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(ADCSRA - IO_BASE),
        AccessWidth::Byte,
        u64::from(ADEN | ADSC),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.poll(SimTime::from_ticks(50)).is_empty());
    assert_eq!(handle.adc_value(), 0);
    io.reset(ResetKind::External);
    handle.set_adc_input(0, 0x03ff);
    io.write(
        u64::from(ADMUX - IO_BASE),
        AccessWidth::Byte,
        0,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(ADCSRA - IO_BASE),
        AccessWidth::Byte,
        u64::from(ADEN | ADSC),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.poll(SimTime::from_ticks(50)).is_empty());
    assert_eq!(handle.adc_value(), 0x03ff);
}

#[test]
fn second_usart_captures_transmit_data_and_reports_ready() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    io.write(
        u64::from(UCSR1B - IO_BASE),
        AccessWidth::Byte,
        u64::from(TXEN1),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(UDR1 - IO_BASE),
        AccessWidth::Byte,
        b'Z'.into(),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.uart1_bytes(), b"Z");
    assert_eq!(
        io.read(
            u64::from(UCSR1A - IO_BASE),
            AccessWidth::Byte,
            SimTime::ZERO,
        )
        .unwrap() as u8
            & (UDRE1 | TXC1),
        UDRE1 | TXC1
    );
}

#[test]
fn analog_comparator_reports_output_and_selected_edges() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    let acsr = u64::from(AtmegaComparatorRegister::Acsr.io_offset());

    // Rising-output interrupt mode, with the comparator interrupt enabled.
    io.write(
        acsr,
        AccessWidth::Byte,
        u64::from(ACSR_ACIE | 3),
        SimTime::ZERO,
    )
    .unwrap();
    handle.set_comparator_inputs(false, true, SimTime::ZERO);
    assert_eq!(
        io.read(acsr, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8 & ACSR_ACO,
        0
    );

    handle.set_comparator_inputs(true, false, SimTime::from_ticks(1));
    let status = io
        .read(acsr, AccessWidth::Byte, SimTime::from_ticks(1))
        .unwrap() as u8;
    assert_ne!(status & ACSR_ACO, 0);
    assert_ne!(status & ACSR_ACI, 0);
    assert_eq!(handle.poll(SimTime::from_ticks(1)), vec![22]);

    // ACI is write-one-to-clear and the falling edge is ignored in rising mode.
    io.write(
        acsr,
        AccessWidth::Byte,
        u64::from(ACSR_ACI | ACSR_ACIE | 3),
        SimTime::from_ticks(1),
    )
    .unwrap();
    assert_eq!(
        io.read(acsr, AccessWidth::Byte, SimTime::from_ticks(1))
            .unwrap() as u8
            & ACSR_ACI,
        0
    );
    handle.set_comparator_inputs(false, true, SimTime::from_ticks(2));
    assert_eq!(handle.poll(SimTime::from_ticks(2)), Vec::<u16>::new());

    // Switching to falling-edge mode makes the next low transition observable.
    io.write(
        acsr,
        AccessWidth::Byte,
        u64::from(ACSR_ACIE | 2),
        SimTime::from_ticks(2),
    )
    .unwrap();
    handle.set_comparator_inputs(true, false, SimTime::from_ticks(3));
    handle.set_comparator_inputs(false, true, SimTime::from_ticks(4));
    assert_eq!(handle.poll(SimTime::from_ticks(4)), vec![22]);
}

#[test]
fn timer3_counter_preload_advances_from_written_value() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.timer3", hub).unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Ocr3al.io_offset()),
        AccessWidth::Byte,
        0xff,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Ocr3ah.io_offset()),
        AccessWidth::Byte,
        0xff,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Tcnt3l.io_offset()),
        AccessWidth::Byte,
        0xfe,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Tcnt3h.io_offset()),
        AccessWidth::Byte,
        0xff,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Timsk3.io_offset()),
        AccessWidth::Byte,
        1 << 1,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(AtmegaTimerRegister::Tccr3b.io_offset()),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.poll(SimTime::from_ticks(1)), Vec::<u16>::new());
    assert_eq!(
        io.read(
            u64::from(AtmegaTimerRegister::Tcnt3l.io_offset()),
            AccessWidth::Byte,
            SimTime::from_ticks(1),
        )
        .unwrap(),
        0xff
    );
    assert_eq!(
        io.read(
            u64::from(AtmegaTimerRegister::Tcnt3h.io_offset()),
            AccessWidth::Byte,
            SimTime::from_ticks(1),
        )
        .unwrap(),
        0xff
    );
    assert_eq!(handle.poll(SimTime::from_ticks(2)), vec![32]);
}

#[test]
fn power_registers_apply_masks_and_clkpr_authorization_window() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();

    assert_eq!(handle.clock_divider(), 1);
    assert!(!handle.sleep_enabled());
    assert_eq!(handle.sleep_mode(), 0);

    io.write(
        u64::from(SMCR - IO_BASE),
        AccessWidth::Byte,
        0xff,
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.sleep_enabled());
    assert_eq!(handle.sleep_mode(), 7);
    assert_eq!(
        io.read(u64::from(SMCR - IO_BASE), AccessWidth::Byte, SimTime::ZERO),
        Ok(0x0f)
    );

    // CLKPR writes without CLKPCE are ignored.
    io.write(
        u64::from(CLKPR - IO_BASE),
        AccessWidth::Byte,
        0x0f,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.clock_divider(), 1);
    io.write(
        u64::from(CLKPR - IO_BASE),
        AccessWidth::Byte,
        CLKPR_CHANGE_ENABLE.into(),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(CLKPR - IO_BASE),
        AccessWidth::Byte,
        2,
        SimTime::from_ticks(1),
    )
    .unwrap();
    assert_eq!(handle.clock_divider(), 4);

    // A second write after the four-tick authorization window is ignored.
    io.write(
        u64::from(CLKPR - IO_BASE),
        AccessWidth::Byte,
        CLKPR_CHANGE_ENABLE.into(),
        SimTime::from_ticks(2),
    )
    .unwrap();
    io.write(
        u64::from(CLKPR - IO_BASE),
        AccessWidth::Byte,
        4,
        SimTime::from_ticks(7),
    )
    .unwrap();
    assert_eq!(handle.clock_divider(), 4);

    io.write(
        u64::from(PRR0 - IO_BASE),
        AccessWidth::Byte,
        0xff,
        SimTime::from_ticks(8),
    )
    .unwrap();
    io.write(
        u64::from(PRR1 - IO_BASE),
        AccessWidth::Byte,
        0xff,
        SimTime::from_ticks(8),
    )
    .unwrap();
    assert_eq!(
        io.read(u64::from(PRR0 - IO_BASE), AccessWidth::Byte, SimTime::ZERO),
        Ok(0xff)
    );
    assert_eq!(
        io.read(u64::from(PRR1 - IO_BASE), AccessWidth::Byte, SimTime::ZERO),
        Ok(u64::from(PRR1_WRITABLE_MASK))
    );
}

#[test]
fn power_reduction_gates_timer_and_uart_facades() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    io.write(
        u64::from(OCR0A - IO_BASE),
        AccessWidth::Byte,
        3,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(TIMSK0 - IO_BASE),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(TCCR0B - IO_BASE),
        AccessWidth::Byte,
        1,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(PRR0 - IO_BASE),
        AccessWidth::Byte,
        PRR0_PRTIM0.into(),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.poll(SimTime::from_ticks(4)).is_empty());
    io.write(
        u64::from(PRR0 - IO_BASE),
        AccessWidth::Byte,
        0,
        SimTime::from_ticks(4),
    )
    .unwrap();
    assert_eq!(handle.poll(SimTime::from_ticks(8)), vec![15]);

    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    io.write(
        u64::from(PRR0 - IO_BASE),
        AccessWidth::Byte,
        PRR0_PRUSART0.into(),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(UDR0 - IO_BASE),
        AccessWidth::Byte,
        u64::from(b'X'),
        SimTime::ZERO,
    )
    .unwrap();
    assert!(handle.uart_bytes().is_empty());
    io.write(
        u64::from(PRR0 - IO_BASE),
        AccessWidth::Byte,
        0,
        SimTime::from_ticks(1),
    )
    .unwrap();
    io.write(
        u64::from(UDR0 - IO_BASE),
        AccessWidth::Byte,
        u64::from(b'Y'),
        SimTime::from_ticks(1),
    )
    .unwrap();
    assert_eq!(handle.uart_bytes(), b"Y");
}

#[test]
fn pb_second_spi_and_twi_instances_use_native_registers_and_vectors() {
    let hub = SignalHub::new();
    let (mut io, handle, _) = AtmegaIo::new("atmega328pb.io", hub).unwrap();
    handle.inject_spi1_rx(0x5a);
    io.write(
        u64::from(SPCR1 - IO_BASE),
        AccessWidth::Byte,
        u64::from(SPCR_SPIE | SPCR_SPE),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(SPDR1 - IO_BASE),
        AccessWidth::Byte,
        0xa5,
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.spi1_bytes(), [0xa5]);
    assert_eq!(handle.poll(SimTime::ZERO), vec![SPI1_INTERRUPT_LINE]);
    assert_eq!(
        io.read(u64::from(SPSR1 - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
            .unwrap()
            & u64::from(SPSR_SPIF),
        u64::from(SPSR_SPIF)
    );
    assert_eq!(
        io.read(u64::from(SPDR1 - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x5a
    );

    let twcr1 = u64::from(TWCR1 - IO_BASE);
    io.write(
        twcr1,
        AccessWidth::Byte,
        u64::from(TWINT | TWSTA | TWEN | TWIE),
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        u64::from(TWDR1 - IO_BASE),
        AccessWidth::Byte,
        0x42,
        SimTime::ZERO,
    )
    .unwrap();
    io.write(
        twcr1,
        AccessWidth::Byte,
        u64::from(TWINT | TWEN | TWIE),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(handle.take_twi1_tx(), [0x42]);
    assert!(handle.poll(SimTime::ZERO).contains(&TWI1_INTERRUPT_LINE));
    handle.queue_twi1_rx(0x9c);
    io.write(
        twcr1,
        AccessWidth::Byte,
        u64::from(TWINT | TWEA | TWEN),
        SimTime::ZERO,
    )
    .unwrap();
    assert_eq!(
        io.read(u64::from(TWDR1 - IO_BASE), AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x9c
    );
}
