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
fn esp32s3_sdmmc_native_window_reads_a_host_card_block() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6002_8000;
    machine.sdmmc().load_card(512, [0x10, 0x20, 0x30, 0x40]);
    machine
        .bus
        .write(
            base + 0x24,
            AccessWidth::Word,
            (1 << 2) | (1 << 5) | (1 << 3),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(base + 0x28, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x20, AccessWidth::Word, 4, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            base + 0x2c,
            AccessWidth::Word,
            u64::from((1_u32 << 31) | (1 << 9) | 17),
            SimTime::from_ticks(1),
        )
        .unwrap();
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
        0x4030_2010
    );
    assert_ne!(
        machine
            .bus
            .read(
                base + 0x40,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap()
            & u64::from((1_u32 << 2) | (1 << 3)),
        0
    );
}
