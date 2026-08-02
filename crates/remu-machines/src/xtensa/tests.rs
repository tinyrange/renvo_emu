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
fn esp32s3_rsa_native_windows_execute_modular_exponentiation() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6003_c000;
    let write = |machine: &mut XtensaMachine, offset: u64, value: u64| {
        machine
            .bus
            .write(base + offset, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    };
    write(&mut machine, 0x804, 0);
    write(&mut machine, 0x000, 55);
    write(&mut machine, 0x600, 7);
    write(&mut machine, 0x400, 13);
    write(&mut machine, 0x80c, 1);
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x200,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        2
    );
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x818,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        1
    );
    write(&mut machine, 0x81c, 1);
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x818,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
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
