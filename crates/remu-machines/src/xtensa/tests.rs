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
fn esp32s3_saradc_native_window_triggers_conversion() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6004_0000;
    machine.saradc().set_input(0, 0x12345).unwrap();
    machine
        .bus
        .write(base + 0x18, AccessWidth::Word, 3 | (1 << 6), SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x5c, AccessWidth::Word, 1 << 31, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base, AccessWidth::Word, 1, SimTime::from_ticks(2))
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x40,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0x12345
    );
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x64,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        1 << 31
    );
    machine
        .bus
        .write(base + 0x68, AccessWidth::Word, 1 << 31, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x64,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0
    );
}
