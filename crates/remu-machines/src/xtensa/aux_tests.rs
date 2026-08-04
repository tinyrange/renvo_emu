use super::*;

#[test]
fn esp32s3_rtc_io_drives_shared_pads_and_routes_wakeup_interrupts() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6000_8400;
    machine
        .bus
        .write(
            base + 0x84,
            AccessWidth::Word,
            (1 << 19) | 0x4000_0000,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(base + 0x0c, AccessWidth::Word, 1 << 10, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x04, AccessWidth::Word, 1 << 10, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.chip_gpio.resolved(0).unwrap(), Logic::One);
    machine
        .bus
        .write(
            base + 0x28,
            AccessWidth::Word,
            (1 << 10) | (2 << 7) | (1 << 2),
            SimTime::ZERO,
        )
        .unwrap();
    machine.set_pin(0, Logic::One).unwrap();
    machine.rtc_io.input_mask_unshifted();
    machine.set_pin(0, Logic::Zero).unwrap();
    assert!(machine.update_rtc_interrupt_lines().unwrap());
}

#[test]
fn esp32s3_sigma_delta_clock_produces_a_traceable_channel() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6000_4f00;
    machine
        .bus
        .write(base, AccessWidth::Word, 0x0080, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x24, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    machine.sdm.poll(SimTime::from_ticks(2)).unwrap();
    assert_eq!(machine.sdm.channel_level(0), Some(true));
}

#[test]
fn esp32s3_uhci1_uses_its_native_page_and_interrupt_source() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6000_c000;
    machine
        .bus
        .write(0x600c_2000 + 15 * 4, AccessWidth::Word, 5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x0c, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x14, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert!(machine.update_uhci_interrupt_lines().unwrap());
    assert_ne!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
}

#[test]
fn esp32s3_peripheral_backup_dma_copies_apb_words_and_completes() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6002_a000;
    for (offset, value) in [(0x50, 0x1122_3344), (0x54, 0x5566_7788)] {
        machine
            .bus
            .write(
                0x6000_8000 + offset,
                AccessWidth::Word,
                value,
                SimTime::ZERO,
            )
            .unwrap();
    }
    for (offset, value) in [
        (0x04, 0x6000_8050),
        (0x08, 0x3fc8_8000),
        (0x24, 1),
        (0x00, (1 << 31) | (1 << 30) | (1 << 29) | (2 << 19)),
    ] {
        machine
            .bus
            .write(base + offset, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    assert!(machine.service_peri_backup().unwrap());
    for (offset, expected) in [(0, 0x1122_3344), (4, 0x5566_7788)] {
        assert_eq!(
            machine
                .bus
                .read(
                    0x3fc8_8000 + offset,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            expected
        );
    }
    assert!(machine.peri_backup.interrupt_pending());
}

#[test]
fn esp32s3_assist_debug_observes_cpu_access_interrupts_and_trace_dma() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x600c_e000;
    for (offset, value) in [
        (0x10, 0x3fc8_8000),
        (0x14, 0x3fc8_800f),
        (0x48, 1),
        (0x4c, 1),
        (0x00, 1 << 1),
        (0x128, 0x44),
        (0x140, 0x3fc8_8000),
        (0x144, 0x3fc8_800f),
        (0x148, 0x3fc8_8100),
        (0x14c, 0x3fc8_811f),
    ] {
        machine
            .bus
            .write(base + offset, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    machine
        .bus
        .write(0x600c_2000 + 83 * 4, AccessWidth::Word, 5, SimTime::ZERO)
        .unwrap();
    {
        let mut guarded = pms::Esp32S3PmsBus::new(
            &mut machine.bus,
            &machine.pms,
            &machine.world_controller,
            &machine.extmem,
            &machine.assist_debug,
            0,
            0x4037_1234,
            0x3fc8_9000,
        );
        guarded
            .write(0x3fc8_8000, AccessWidth::Word, 0xa55a_5aa5, SimTime::ZERO)
            .unwrap();
    }
    assert!(machine.service_assist_debug_logs().unwrap());
    assert!(machine.update_assist_debug_interrupt_lines().unwrap());
    assert_ne!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x34,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x4037_1234
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x3fc8_8104,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x3fc8_8000
    );
}
