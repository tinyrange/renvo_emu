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
fn esp32s3_i2s1_mmio_transmits_a_single_data_frame() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    const BASE: u64 = 0x6002_d000;
    machine
        .bus
        .write(
            BASE + Esp32s3I2s::SINGLE_DATA,
            AccessWidth::Word,
            0x1234_5678,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            BASE + Esp32s3I2s::TX_CONF1,
            AccessWidth::Word,
            15 << 13,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            BASE + Esp32s3I2s::TX_CONF,
            AccessWidth::Word,
            1 << 2,
            SimTime::from_ticks(1),
        )
        .unwrap();

    assert_eq!(
        machine
            .bus
            .read(
                BASE + Esp32s3I2s::INT_RAW,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1 << 1
    );
    assert_eq!(
        machine
            .bus
            .read(
                BASE + Esp32s3I2s::STATE,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1
    );
    assert!(
        machine
            .signals
            .with_registry(|registry| registry.find("esp32s3.i2s1.bclk").is_some())
    );
    assert!(
        machine
            .signals
            .with_registry(|registry| registry.find("esp32s3.i2s1.dout").is_some())
    );
}
