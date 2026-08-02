use super::{
    AccessWidth, CRC0CN0, CRC0DAT, CRC0FLIP, CRC0IN, EIE1_EPCA0, Efm8PcaRegister, Efm8Peripherals,
    Efm8SmbusRegister, IE, IE_EA, IE_ESPI0, IE_ET0, IE_ET1, P0, P0MDOUT, PCA0CN_CR, SBUF0,
    SPI0_SPIEN, SPI0_TXNF, SPI0CN0, SPI0DAT, SimTime, TCON, TCON_TF1, TCON_TR0, TCON_TR1, TH1, TL1,
    TMOD, XBR0, XBR0_URT0E, XBR2, XBR2_XBARE,
};
use remu_bus::Device;
use remu_signals::Logic;

#[test]
fn pca_register_ids_are_named_and_page_aliased() {
    assert_eq!(Efm8PcaRegister::Pca0Cn.address(), 0xd8);
    assert_eq!(Efm8PcaRegister::Pca0Cn.name(), "pca0cn");
    assert_eq!(
        Efm8PcaRegister::from_address(0x10d8),
        Some(Efm8PcaRegister::Pca0Cn)
    );
    assert_eq!(
        Efm8PcaRegister::from_address(Efm8PcaRegister::Eip1.address()),
        Some(Efm8PcaRegister::Eip1)
    );
    assert_eq!(Efm8PcaRegister::ALL.len(), 19);
}

#[test]
fn gpio_timer_uart_and_interrupt_slice_is_functional() {
    let hub = super::SignalHub::new();
    let (mut device, handle, ports) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    device
        .write(P0MDOUT as u64, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    device
        .write(P0 as u64, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(ports[0].output() & 1, 1);

    device
        .write(
            XBR0 as u64,
            AccessWidth::Byte,
            XBR0_URT0E.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            XBR2 as u64,
            AccessWidth::Byte,
            XBR2_XBARE.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(SBUF0 as u64, AccessWidth::Byte, b'E'.into(), SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.uart_bytes(), b"E");

    device
        .write(TMOD as u64, AccessWidth::Byte, 2, SimTime::ZERO)
        .unwrap();
    device
        .write(super::TH0 as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
        .unwrap();
    device
        .write(
            IE as u64,
            AccessWidth::Byte,
            (IE_EA | IE_ET0).into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            TCON as u64,
            AccessWidth::Byte,
            TCON_TR0.into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(handle.poll(SimTime::from_ticks(4))[0]);
}

#[test]
fn uart1_paged_fifo_and_interrupt_slice_is_functional() {
    let hub = super::SignalHub::new();
    let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    device
        .write(
            super::SBCON1 as u64,
            AccessWidth::Byte,
            super::SBCON1_BREN.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            super::SCON1 as u64,
            AccessWidth::Byte,
            super::SCON1_REN.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(super::EIE2 as u64, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    device
        .write(IE as u64, AccessWidth::Byte, IE_EA.into(), SimTime::ZERO)
        .unwrap();
    device
        .write(
            super::SBUF1 as u64,
            AccessWidth::Byte,
            b'U'.into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(handle.uart1_bytes(), b"U");
    assert!(handle.poll(SimTime::ZERO)[12]);

    handle.inject_uart1_rx(0xa5, SimTime::from_ticks(1));
    assert_eq!(
        device
            .read(super::UART1FCT as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        1
    );
    assert_eq!(
        device
            .read(super::SBUF1 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0xa5
    );
}

#[test]
fn timer345_reload_flags_and_interrupts_are_functional() {
    let hub = super::SignalHub::new();
    let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    device
        .write(
            Efm8SmbusRegister::Eie1.offset() as u64,
            AccessWidth::Byte,
            super::EIE1_ET3.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            super::EIE2 as u64,
            AccessWidth::Byte,
            (super::EIE2_ET4 | super::EIE2_ET5).into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(IE as u64, AccessWidth::Byte, IE_EA.into(), SimTime::ZERO)
        .unwrap();

    for (reload_low, reload_high, current_low, current_high, control, run, cen) in [
        (
            super::TMR3RLL,
            super::TMR3RLH,
            super::TMR3L,
            super::TMR3H,
            super::TMR3CN0,
            super::TMR3_TR3,
            super::TMR3_TF3CEN,
        ),
        (
            super::TMR4RLL,
            super::TMR4RLH,
            super::TMR4L,
            super::TMR4H,
            super::TMR4CN0,
            super::TMR4_TR4,
            super::TMR4_TF4CEN,
        ),
        (
            super::TMR5RLL,
            super::TMR5RLH,
            super::TMR5L,
            super::TMR5H,
            super::TMR5CN0,
            super::TMR5_TR5,
            super::TMR5_TF5CEN,
        ),
    ] {
        device
            .write(reload_low as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
            .unwrap();
        device
            .write(reload_high as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        device
            .write(current_low as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
            .unwrap();
        device
            .write(current_high as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
            .unwrap();
        device
            .write(
                control as u64,
                AccessWidth::Byte,
                (run | cen).into(),
                SimTime::ZERO,
            )
            .unwrap();
    }

    for address in [super::TMR3CN1, super::TMR4CN1, super::TMR5CN1] {
        device
            .write(address as u64, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
    }
    let levels = handle.poll(SimTime::from_ticks(4));
    assert!(levels[14]);
    assert!(levels[16]);
    assert!(levels[18]);
    for (control, flag) in [
        (super::TMR3CN0, super::TMR3_TF3H),
        (super::TMR4CN0, super::TMR4_TF4H),
        (super::TMR5CN0, super::TMR5_TF5H),
    ] {
        assert_ne!(
            device
                .read(control as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap()
                & u64::from(flag),
            0
        );
    }
}

#[test]
fn adc_channel_conversion_window_and_interrupts_are_functional() {
    let hub = super::SignalHub::new();
    let trace_hub = hub.clone();
    let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    handle.set_adc_input(3, 0x0abc).unwrap();

    for (address, value) in [
        (super::ADC0MX, 3),
        (super::ADC0GTH, 0x0b),
        (super::ADC0GTL, 0xff),
        (super::ADC0LTH, 0x08),
        (super::ADC0LTL, 0x00),
        (0x30b2, 0),
        (0x30b3, 0),
    ] {
        device
            .write(address as u64, AccessWidth::Byte, value, SimTime::ZERO)
            .unwrap();
    }
    device
        .write(
            Efm8SmbusRegister::Eie1.offset() as u64,
            AccessWidth::Byte,
            u64::from(super::ADC0_EADC0 | super::ADC0_EWADC0),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(IE as u64, AccessWidth::Byte, IE_EA.into(), SimTime::ZERO)
        .unwrap();
    device
        .write(
            super::ADC0CN0 as u64,
            AccessWidth::Byte,
            u64::from(super::ADC0_ADEN | super::ADC0_ADBUSY),
            SimTime::from_ticks(1),
        )
        .unwrap();

    assert_eq!(
        device
            .read(super::ADC0L as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0xbc
    );
    assert_eq!(
        device
            .read(super::ADC0H as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x0a
    );
    let control = device
        .read(super::ADC0CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
        .unwrap();
    assert_ne!(control & u64::from(super::ADC0_ADINT), 0);
    assert_eq!(control & u64::from(super::ADC0_ADWINT), 0);
    let levels = handle.poll(SimTime::from_ticks(1));
    assert!(!levels[20]);
    assert!(levels[22]);

    let result_id = trace_hub
        .with_registry(|registry| registry.find("board.efm8bb52f32g.adc0.result"))
        .unwrap();
    assert_eq!(
        trace_hub.with_registry(|registry| registry.value(result_id).unwrap().to_vcd_binary()),
        "0000101010111100"
    );

    device
        .write(
            super::ADC0CN0 as u64,
            AccessWidth::Byte,
            u64::from(super::ADC0_ADEN),
            SimTime::from_ticks(2),
        )
        .unwrap();
    handle.set_adc_input(3, 0x0fff).unwrap();
    device
        .write(
            super::ADC0CN0 as u64,
            AccessWidth::Byte,
            u64::from(super::ADC0_ADEN | super::ADC0_ADBUSY),
            SimTime::from_ticks(3),
        )
        .unwrap();
    assert!(handle.poll(SimTime::from_ticks(3))[20]);
}

#[test]
fn dac_paged_registers_format_code_and_track_enable_state() {
    let hub = super::SignalHub::new();
    let trace_hub = hub.clone();
    let (mut device, _, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();

    for (address, value, at) in [
        (super::DAC0CF0, 0x80, SimTime::ZERO),
        (super::DAC0L, 0x5a, SimTime::ZERO),
        (super::DAC0H, 0x02, SimTime::from_ticks(1)),
    ] {
        device
            .write(address as u64, AccessWidth::Byte, value, at)
            .unwrap();
    }
    assert_eq!(
        device
            .read(super::DAC0L as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x5a
    );
    let output_id = trace_hub
        .with_registry(|registry| registry.find("board.efm8bb52f32g.dac0.output"))
        .unwrap();
    let enabled_id = trace_hub
        .with_registry(|registry| registry.find("board.efm8bb52f32g.dac0.enabled"))
        .unwrap();
    assert_eq!(
        trace_hub.with_registry(|registry| registry.value(output_id).unwrap().to_vcd_binary()),
        "1001011010"
    );
    assert_eq!(
        trace_hub.with_registry(|registry| registry.value(enabled_id).unwrap().to_vcd_binary()),
        "1"
    );

    for (address, value, at) in [
        (super::DAC0CF0, 0xa0, SimTime::from_ticks(2)),
        (super::DAC0L, 0xc0, SimTime::from_ticks(2)),
        (super::DAC0H, 0x55, SimTime::from_ticks(3)),
        (super::DAC0CF1, 0xff, SimTime::from_ticks(4)),
    ] {
        device
            .write(address as u64, AccessWidth::Byte, value, at)
            .unwrap();
    }
    assert_eq!(
        trace_hub.with_registry(|registry| registry.value(output_id).unwrap().to_vcd_binary()),
        "0101010111"
    );
    assert_eq!(
        device
            .read(super::DAC0CF1 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x0f
    );
}

#[test]
fn comparators_latch_edges_and_raise_documented_interrupts() {
    let hub = super::SignalHub::new();
    let trace_hub = hub.clone();
    let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    device
        .write(
            Efm8SmbusRegister::Eie1.offset() as u64,
            AccessWidth::Byte,
            0x60,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(IE as u64, AccessWidth::Byte, IE_EA.into(), SimTime::ZERO)
        .unwrap();
    for mode in [super::CMP0MD, super::CMP1MD] {
        device
            .write(mode as u64, AccessWidth::Byte, 0x30, SimTime::ZERO)
            .unwrap();
    }
    handle
        .set_comparator_inputs(0, 100, 20, SimTime::from_ticks(1))
        .unwrap();
    handle
        .set_comparator_inputs(1, 20, 10, SimTime::from_ticks(1))
        .unwrap();
    for control in [super::CMP0CN0, super::CMP1CN0] {
        device
            .write(
                control as u64,
                AccessWidth::Byte,
                0x80,
                SimTime::from_ticks(2),
            )
            .unwrap();
        assert_eq!(
            device
                .read(control as u64, AccessWidth::Byte, SimTime::ZERO)
                .unwrap()
                & 0x70,
            0x60
        );
    }
    let levels = handle.poll(SimTime::from_ticks(2));
    assert!(levels[24]);
    assert!(levels[26]);

    for (name, expected) in [
        ("board.efm8bb52f32g.comparator0.output", Logic::One),
        ("board.efm8bb52f32g.comparator1.output", Logic::One),
    ] {
        let id = trace_hub
            .with_registry(|registry| registry.find(name))
            .unwrap();
        assert_eq!(
            trace_hub.with_registry(|registry| registry.value(id).unwrap().bit(0)),
            Some(expected)
        );
    }

    device
        .write(
            super::CMP0CN0 as u64,
            AccessWidth::Byte,
            0x80,
            SimTime::from_ticks(3),
        )
        .unwrap();
    handle
        .set_comparator_inputs(0, 1, 2, SimTime::from_ticks(4))
        .unwrap();
    assert_ne!(
        device
            .read(super::CMP0CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap()
            & 0x10,
        0
    );
}

#[test]
fn configurable_logic_lut_edges_and_interrupts_are_functional() {
    let hub = super::SignalHub::new();
    let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    for (address, value) in [
        (super::CLU_FN[0], 0xc0),
        (super::CLU_CF[0], 0x80),
        (super::CLEN0, 1),
        (super::CLIE0, 0x03),
        (super::EIE2, super::EIE2_CL0),
        (IE, IE_EA),
    ] {
        device
            .write(
                address as u64,
                AccessWidth::Byte,
                value.into(),
                SimTime::ZERO,
            )
            .unwrap();
    }
    handle
        .set_clu_inputs(0, false, true, SimTime::from_ticks(1))
        .unwrap();
    assert!(!handle.clu_output(0).unwrap());
    handle
        .set_clu_inputs(0, true, true, SimTime::from_ticks(2))
        .unwrap();
    assert!(handle.clu_output(0).unwrap());
    assert_eq!(
        device
            .read(super::CLOUT0 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        1
    );
    assert_eq!(
        device
            .read(super::CLIF0 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x02
    );
    assert!(handle.poll(SimTime::from_ticks(2))[28]);

    device
        .write(super::CLIF0 as u64, AccessWidth::Byte, 0, SimTime::ZERO)
        .unwrap();
    handle
        .set_clu_inputs(0, false, true, SimTime::from_ticks(3))
        .unwrap();
    assert_eq!(
        device
            .read(super::CLIF0 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x01
    );
}

#[test]
fn spi0_master_transfer_exposes_injected_miso_and_interrupt() {
    let hub = super::SignalHub::new();
    let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    handle.inject_spi_rx(0x3c);
    device
        .write(
            SPI0CN0 as u64,
            AccessWidth::Byte,
            SPI0_SPIEN.into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            IE as u64,
            AccessWidth::Byte,
            (IE_EA | IE_ESPI0).into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(SPI0DAT as u64, AccessWidth::Byte, 0xa5, SimTime::ZERO)
        .unwrap();
    assert_eq!(handle.spi_bytes(), [0xa5]);
    assert!(handle.poll(SimTime::from_ticks(1))[6]);
    assert_eq!(
        device
            .read(SPI0DAT as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x3c
    );
    assert_eq!(
        device
            .read((0x20_00 | SPI0DAT) as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x3c
    );
    assert!(handle.poll(SimTime::from_ticks(1))[6]);
    device
        .write(
            SPI0CN0 as u64,
            AccessWidth::Byte,
            SPI0_SPIEN.into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(!handle.poll(SimTime::from_ticks(1))[6]);
    assert_eq!(
        device
            .read(SPI0CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap()
            & u64::from(SPI0_TXNF),
        u64::from(SPI0_TXNF)
    );
}

#[test]
fn timer1_mode2_sets_its_dedicated_interrupt_line() {
    let hub = super::SignalHub::new();
    let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    device
        .write(TMOD as u64, AccessWidth::Byte, 0x20, SimTime::ZERO)
        .unwrap();
    // The first overflow is measured from TL1; TH1 is only the reload
    // value after that overflow.
    device
        .write(TH1 as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
        .unwrap();
    device
        .write(TL1 as u64, AccessWidth::Byte, 0xfc, SimTime::ZERO)
        .unwrap();
    device
        .write(
            IE as u64,
            AccessWidth::Byte,
            (IE_EA | IE_ET1).into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            TCON as u64,
            AccessWidth::Byte,
            TCON_TR1.into(),
            SimTime::ZERO,
        )
        .unwrap();

    let interrupts = handle.poll(SimTime::from_ticks(4));
    assert!(interrupts[8]);
    assert_eq!(
        device
            .read(TCON as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap()
            & 0x80,
        0x80
    );
    assert_eq!(
        device
            .read(TL1 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0xfc
    );

    handle.acknowledge_timer1_interrupt(SimTime::from_ticks(4));
    assert!(!handle.poll(SimTime::from_ticks(5))[8]);
    assert_eq!(
        device
            .read(TCON as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap() as u8
            & TCON_TF1,
        0
    );
}

#[test]
fn timer1_mode1_overflows_from_the_programmed_16_bit_value() {
    let hub = super::SignalHub::new();
    let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    device
        .write(TMOD as u64, AccessWidth::Byte, 0x10, SimTime::ZERO)
        .unwrap();
    device
        .write(TH1 as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
        .unwrap();
    device
        .write(TL1 as u64, AccessWidth::Byte, 0xfe, SimTime::ZERO)
        .unwrap();
    device
        .write(
            IE as u64,
            AccessWidth::Byte,
            (IE_EA | IE_ET1).into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            TCON as u64,
            AccessWidth::Byte,
            TCON_TR1.into(),
            SimTime::ZERO,
        )
        .unwrap();

    assert!(!handle.poll(SimTime::from_ticks(1))[8]);
    assert!(handle.poll(SimTime::from_ticks(2))[8]);
    assert_eq!(
        device
            .read(TCON as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap() as u8
            & TCON_TF1,
        TCON_TF1
    );
    assert_eq!(
        device
            .read(TL1 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0
    );
    assert_eq!(
        device
            .read(TH1 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0
    );
}

#[test]
fn timer1_mode3_remains_inactive() {
    let hub = super::SignalHub::new();
    let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    device
        .write(TMOD as u64, AccessWidth::Byte, 0x30, SimTime::ZERO)
        .unwrap();
    device
        .write(TH1 as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
        .unwrap();
    device
        .write(TL1 as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
        .unwrap();
    device
        .write(
            IE as u64,
            AccessWidth::Byte,
            (IE_EA | IE_ET1).into(),
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            TCON as u64,
            AccessWidth::Byte,
            TCON_TR1.into(),
            SimTime::ZERO,
        )
        .unwrap();

    assert!(!handle.poll(SimTime::from_ticks(100_000))[8]);
    assert_eq!(
        device
            .read(TL1 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0xff
    );
    assert_eq!(
        device
            .read(TH1 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0xff
    );
    assert_eq!(
        device
            .read(TCON as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap() as u8
            & TCON_TF1,
        0
    );
}

#[test]
fn crc16_stream_and_bit_reverse_follow_efm8_register_contract() {
    let hub = super::SignalHub::new();
    let (mut device, _, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    device
        .write(CRC0CN0 as u64, AccessWidth::Byte, 0x0c, SimTime::ZERO)
        .unwrap();
    for byte in [0xaa, 0xbb, 0xcc] {
        device
            .write(CRC0IN as u64, AccessWidth::Byte, byte, SimTime::ZERO)
            .unwrap();
    }
    assert_eq!(
        device
            .read(CRC0DAT as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0xf6
    );
    assert_eq!(
        device
            .read(CRC0DAT as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x6c
    );
    device
        .write(CRC0FLIP as u64, AccessWidth::Byte, 0xc0, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        device
            .read(CRC0FLIP as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x03
    );
}

#[test]
fn crc0_control_masks_reserved_bits_and_supports_result_writes() {
    let hub = super::SignalHub::new();
    let (mut device, _, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    device
        .write(CRC0CN0 as u64, AccessWidth::Byte, 0xff, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        device
            .read(CRC0CN0 as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x05
    );

    device
        .write(CRC0CN0 as u64, AccessWidth::Byte, 0x0c, SimTime::ZERO)
        .unwrap();
    device
        .write(CRC0DAT as u64, AccessWidth::Byte, 0x34, SimTime::ZERO)
        .unwrap();
    device
        .write(CRC0DAT as u64, AccessWidth::Byte, 0x12, SimTime::ZERO)
        .unwrap();
    device
        .write(CRC0CN0 as u64, AccessWidth::Byte, 0x00, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        device
            .read(CRC0DAT as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x34
    );
    assert_eq!(
        device
            .read(CRC0DAT as u64, AccessWidth::Byte, SimTime::ZERO)
            .unwrap(),
        0x12
    );
}

#[test]
fn pca_pwm_capture_and_interrupt_slice_is_functional() {
    let hub = super::SignalHub::new();
    let (mut device, handle, _) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    // Select SYSCLK as the abstract PCA timebase and configure an 8-bit PWM.
    device
        .write(
            Efm8PcaRegister::Pca0Md.address() as u64,
            AccessWidth::Byte,
            0x08,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Efm8PcaRegister::Pca0Pwm.address() as u64,
            AccessWidth::Byte,
            0,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Efm8PcaRegister::Pca0Cpm0.address() as u64,
            AccessWidth::Byte,
            0x02,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Efm8PcaRegister::Pca0Cpl0.address() as u64,
            AccessWidth::Byte,
            0x40,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Efm8PcaRegister::Pca0Cph0.address() as u64,
            AccessWidth::Byte,
            0,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Efm8PcaRegister::Pca0Cn.address() as u64,
            AccessWidth::Byte,
            PCA0CN_CR.into(),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(!handle.poll(SimTime::from_ticks(0))[0]);
    assert!(!handle.poll(SimTime::from_ticks(0x40))[0]);
    assert_eq!(handle.pca_output(0), Logic::One);
    assert_eq!(handle.pca_counter(), 0x40);
    assert!(!handle.poll(SimTime::from_ticks(0x100))[0]);
    assert_eq!(handle.pca_output(0), Logic::Zero);

    // A channel compare and an input capture share the PCA request line.
    device
        .write(
            Efm8PcaRegister::Pca0Cpm0.address() as u64,
            AccessWidth::Byte,
            0x49,
            SimTime::from_ticks(0x100),
        )
        .unwrap();
    device
        .write(
            Efm8PcaRegister::Pca0Cpl0.address() as u64,
            AccessWidth::Byte,
            2,
            SimTime::from_ticks(0x100),
        )
        .unwrap();
    device
        .write(
            Efm8PcaRegister::Pca0Cph0.address() as u64,
            AccessWidth::Byte,
            1,
            SimTime::from_ticks(0x100),
        )
        .unwrap();
    device
        .write(
            Efm8PcaRegister::Eie1.address() as u64,
            AccessWidth::Byte,
            EIE1_EPCA0.into(),
            SimTime::from_ticks(0x100),
        )
        .unwrap();
    device
        .write(
            IE as u64,
            AccessWidth::Byte,
            IE_EA.into(),
            SimTime::from_ticks(0x100),
        )
        .unwrap();
    let levels = handle.poll(SimTime::from_ticks(0x104));
    assert!(levels[6]);
    assert!(handle.pca_interrupt_pending());

    device
        .write(
            Efm8PcaRegister::Pca0Cpm1.address() as u64,
            AccessWidth::Byte,
            0x21,
            SimTime::from_ticks(0x104),
        )
        .unwrap();
    handle
        .set_pca_input(1, Logic::One, SimTime::from_ticks(0x108))
        .unwrap();
    assert_eq!(
        device
            .read(
                Efm8PcaRegister::Pca0Cpl1.address() as u64,
                AccessWidth::Byte,
                SimTime::from_ticks(0x108)
            )
            .unwrap(),
        0x08
    );
    assert_eq!(
        device
            .read(
                Efm8PcaRegister::Pca0Cph1.address() as u64,
                AccessWidth::Byte,
                SimTime::from_ticks(0x108)
            )
            .unwrap(),
        1
    );
}

#[test]
fn smbus0_master_and_follower_byte_paths_are_observable() {
    let hub = super::SignalHub::new();
    let (mut device, handle, _ports) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    device
        .write(
            Efm8SmbusRegister::Smb0Cf.offset() as u64,
            AccessWidth::Byte,
            0x80,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Efm8SmbusRegister::Eie1.offset() as u64,
            AccessWidth::Byte,
            1,
            SimTime::ZERO,
        )
        .unwrap();
    device
        .write(
            Efm8SmbusRegister::Smb0Cn0.offset() as u64,
            AccessWidth::Byte,
            0x20,
            SimTime::ZERO,
        )
        .unwrap();
    assert!(handle.smbus0_busy());
    assert!(handle.smbus0_interrupt());
    device
        .write(
            Efm8SmbusRegister::Smb0Cn0.offset() as u64,
            AccessWidth::Byte,
            0,
            SimTime::from_ticks(1),
        )
        .unwrap();
    device
        .write(
            Efm8SmbusRegister::Smb0Dat.offset() as u64,
            AccessWidth::Byte,
            0xa0,
            SimTime::from_ticks(2),
        )
        .unwrap();
    assert_eq!(handle.smbus0_tx_bytes(), vec![0xa0]);
    assert!(handle.smbus0_interrupt());

    device
        .write(
            Efm8SmbusRegister::Smb0Cn0.offset() as u64,
            AccessWidth::Byte,
            0,
            SimTime::from_ticks(3),
        )
        .unwrap();
    handle.inject_smbus0_rx(&[0x12, 0x34], SimTime::from_ticks(4));
    assert!(handle.smbus0_interrupt());
    assert_eq!(
        device.read(
            Efm8SmbusRegister::Smb0Dat.offset() as u64,
            AccessWidth::Byte,
            SimTime::from_ticks(5),
        ),
        Ok(0x12)
    );
    assert_eq!(
        device.read(
            Efm8SmbusRegister::Smb0Dat.offset() as u64,
            AccessWidth::Byte,
            SimTime::from_ticks(6),
        ),
        Ok(0x34)
    );
}

#[test]
fn smbus0_named_registers_and_status_bits_match_reference_surface() {
    assert_eq!(Efm8SmbusRegister::ALL.len(), 13);
    for (index, register) in Efm8SmbusRegister::ALL.iter().copied().enumerate() {
        assert_eq!(register.index(), index);
        assert_eq!(
            Efm8SmbusRegister::from_data_address(register.offset()),
            Some(register)
        );
    }
    assert_eq!(
        Efm8SmbusRegister::from_data_address(0x20c0),
        Some(Efm8SmbusRegister::Smb0Cn0)
    );
    assert_eq!(
        Efm8SmbusRegister::from_data_address(0x10bb),
        Some(Efm8SmbusRegister::Eip1)
    );
    assert_eq!(
        Efm8SmbusRegister::from_data_address(0x10e6),
        Some(Efm8SmbusRegister::Eie1)
    );

    let hub = super::SignalHub::new();
    let (mut device, handle, _ports) = Efm8Peripherals::new("efm8.sfr", hub).unwrap();
    let at = SimTime::ZERO;
    let offset = |register: Efm8SmbusRegister| register.offset() as u64;
    device
        .write(
            offset(Efm8SmbusRegister::Smb0Tc),
            AccessWidth::Byte,
            0xff,
            at,
        )
        .unwrap();
    assert_eq!(
        device.read(offset(Efm8SmbusRegister::Smb0Tc), AccessWidth::Byte, at),
        Ok(0x93)
    );
    device
        .write(
            offset(Efm8SmbusRegister::Smb0Adr),
            AccessWidth::Byte,
            0xff,
            at,
        )
        .unwrap();
    assert_eq!(
        device.read(offset(Efm8SmbusRegister::Smb0Adr), AccessWidth::Byte, at),
        Ok(0xff)
    );

    device
        .write(
            offset(Efm8SmbusRegister::Smb0Cf),
            AccessWidth::Byte,
            0x80,
            at,
        )
        .unwrap();
    device
        .write(offset(Efm8SmbusRegister::Eie1), AccessWidth::Byte, 1, at)
        .unwrap();
    device
        .write(IE as u64, AccessWidth::Byte, IE_EA.into(), at)
        .unwrap();
    device
        .write(
            offset(Efm8SmbusRegister::Smb0Cn0),
            AccessWidth::Byte,
            0x20,
            at,
        )
        .unwrap();
    assert!(handle.poll(at)[10]);
    device
        .write(offset(Efm8SmbusRegister::Smb0Cn0), AccessWidth::Byte, 0, at)
        .unwrap();
    assert!(!handle.smbus0_interrupt());

    device
        .write(
            offset(Efm8SmbusRegister::Smb0Dat),
            AccessWidth::Byte,
            0xa0,
            at,
        )
        .unwrap();
    assert_eq!(
        device.read(offset(Efm8SmbusRegister::Smb0Fct), AccessWidth::Byte, at),
        Ok(0x10)
    );
    assert_eq!(
        device.read(offset(Efm8SmbusRegister::Smb0Fcn1), AccessWidth::Byte, at),
        Ok(0x44)
    );
    device
        .write(
            offset(Efm8SmbusRegister::Smb0Fct),
            AccessWidth::Byte,
            0xff,
            at,
        )
        .unwrap();
    assert_eq!(
        device.read(offset(Efm8SmbusRegister::Smb0Fct), AccessWidth::Byte, at),
        Ok(0x10)
    );
    device
        .write(
            offset(Efm8SmbusRegister::Smb0Fcn0),
            AccessWidth::Byte,
            1 << 6,
            at,
        )
        .unwrap();
    assert_eq!(
        device.read(offset(Efm8SmbusRegister::Smb0Fct), AccessWidth::Byte, at),
        Ok(0)
    );
}
