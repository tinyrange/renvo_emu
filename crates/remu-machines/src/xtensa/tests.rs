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
fn esp32s3_radio_pages_provide_deterministic_wifi_loopback_and_btle_identity() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let wifi = 0x6000_6000;
    machine
        .bus
        .write(wifi, AccessWidth::Word, 0b11, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(wifi + 0x14, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(wifi + 0x0c, AccessWidth::Word, 4, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(wifi + 0x100, AccessWidth::Word, 0xaabb_ccdd, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(wifi + 0x24, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                wifi + 0x04,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0b101
    );
    assert_eq!(
        machine
            .bus
            .read(
                wifi + 0x1c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        1
    );
    assert_eq!(
        machine
            .bus
            .read(
                wifi + 0x200,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0xaabb_ccdd
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x6001_1000 + 0x28,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        u64::from(u32::from_le_bytes(*b"BTLE"))
    );
}
