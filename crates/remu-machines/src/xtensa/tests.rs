use super::*;

#[test]
fn direct_load_starts_with_appcpu_reset_and_parked() {
    let machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    assert!(machine.appcpu_boot_address.is_none());
    assert_eq!(machine.cpu1.snapshot().pc, 0);
    assert!(!machine.cpu1.snapshot().waiting);
    assert!(!machine.cpu1.snapshot().halted);
}

#[test]
fn appcpu_systimer_defers_to_a_logical_window_safe_point_during_usb_execution() {
    assert!(appcpu_systimer_level(true, false, false));
    assert!(!appcpu_systimer_level(true, true, false));
    assert!(appcpu_systimer_level(true, true, true));
    assert!(!appcpu_systimer_level(false, true, true));
}

#[test]
fn dwc2_host_completes_only_after_the_final_raw_prompt() {
    let mut host = EspDwc2Host::new();
    assert!(!host.input_complete());
    host.queue_input(b"\x01print(1)\n\x04");
    host.input.clear();
    host.sending_raw_chunk = false;
    host.raw_prompt_ready = true;
    host.output
        .extend_from_slice(b"__REMU_HOST_SCRIPT_COMPLETE__\r\n\x04\x04>");
    assert!(host.input_complete());
}

#[test]
fn esp32s3_ulp_timer_and_rtc_memory_are_deterministic() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let rtc = 0x6000_8000;
    let period = 4_u64 << 8;
    let ulp_interrupt = 1_u64 << 5;

    machine
        .bus
        .write(rtc + 0x134, AccessWidth::Word, period, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(rtc + 0x40, AccessWidth::Word, ulp_interrupt, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(rtc + 0x100, AccessWidth::Word, 1_u64 << 31, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(rtc + 0xfc, AccessWidth::Word, 1_u64 << 31, SimTime::ZERO)
        .unwrap();

    machine
        .bus
        .write(0x5000_0000, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x5000_0000,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x1234_5678
    );
    assert!(!machine.rtc_control().ulp_pending(SimTime::from_ticks(3)));
    assert!(machine.rtc_control().ulp_pending(SimTime::from_ticks(4)));
    assert_eq!(
        machine
            .bus
            .read(
                rtc + 0x44,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap()
            & ulp_interrupt,
        ulp_interrupt
    );

    machine
        .bus
        .write(rtc + 0x4c, AccessWidth::Word, ulp_interrupt, SimTime::ZERO)
        .unwrap();
    assert!(!machine.rtc_control().ulp_pending(SimTime::from_ticks(4)));
}
