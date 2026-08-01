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
fn esp32s3_auxiliary_uarts_capture_transmit_fifo_writes() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .bus
        .write(0x6001_0000, AccessWidth::Word, 0x31, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x6002_e000, AccessWidth::Word, 0x32, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.auxiliary_uarts[0].bytes(), [0x31]);
    assert_eq!(machine.auxiliary_uarts[1].bytes(), [0x32]);
    assert_eq!(
        machine
            .bus
            .read(
                0x6001_001c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );
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
