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
fn esp32s3_exposes_the_m5sticks3_octal_psram_window() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .debug_write_memory(0x3d00_0000, &[0x12, 0x34, 0x56, 0x78])
        .unwrap();
    machine
        .debug_write_memory(0x3d7f_fffc, &[0xa5, 0x5a, 0xc3, 0x3c])
        .unwrap();

    assert_eq!(
        machine.debug_read_memory(0x3d00_0000, 4).unwrap(),
        vec![0x12, 0x34, 0x56, 0x78]
    );
    assert_eq!(
        machine.debug_read_memory(0x3d7f_fffc, 4).unwrap(),
        vec![0xa5, 0x5a, 0xc3, 0x3c]
    );
    assert!(machine.debug_read_memory(0x3d80_0000, 1).is_err());
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
