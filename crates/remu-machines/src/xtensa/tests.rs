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
fn esp32s3_rmt_native_mmio_emits_a_named_channel_waveform() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6001_6000;
    let item = 2_u64 | (1 << 15) | (3 << 16);
    machine
        .bus
        .write(base, AccessWidth::Word, item, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x20, AccessWidth::Word, 1 | (1 << 6), SimTime::ZERO)
        .unwrap();
    let raw = machine
        .bus
        .read(
            base + 0x70,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(raw & 1, 1);
    assert!(
        machine
            .signals
            .with_registry(|registry| registry.find("board.esp32s3.rmt.ch0").is_some())
    );
    assert!(
        machine
            .signals
            .drain_changes()
            .iter()
            .any(|change| change.at == SimTime::from_ticks(2))
    );
}
