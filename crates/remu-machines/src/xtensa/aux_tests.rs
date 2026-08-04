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
