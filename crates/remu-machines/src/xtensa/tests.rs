use super::*;
use remu_devices::Esp32S3InterruptRegister;

#[test]
fn direct_load_starts_with_appcpu_reset_and_parked() {
    let machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    assert!(machine.appcpu_boot_address.is_none());
    assert_eq!(machine.cpu1.snapshot().pc, 0);
    assert!(!machine.cpu1.snapshot().waiting);
    assert!(!machine.cpu1.snapshot().halted);
}

#[test]
fn esp32s3_interrupt_matrix_native_routes_feed_the_scheduler_view() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x600c_2000;
    machine
        .bus
        .write(
            base + Esp32S3InterruptRegister::Core0Route(38).offset(),
            AccessWidth::Word,
            5,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            base + Esp32S3InterruptRegister::Core1Route(39).offset(),
            AccessWidth::Word,
            7,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                base + Esp32S3InterruptRegister::Core0Route(38).offset(),
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        5
    );
    assert_eq!(machine.interrupt_matrix.route(0, 38), 5);
    assert_eq!(machine.interrupt_matrix.route(1, 39), 7);
    machine.interrupt_matrix.set_source_pending(1, 39, true);
    assert_eq!(
        machine
            .bus
            .read(
                base + Esp32S3InterruptRegister::Core1Status(0).offset(),
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1 << 7
    );
    machine
        .bus
        .write(
            base + Esp32S3InterruptRegister::Core0Route(38).offset(),
            AccessWidth::Word,
            0x1f,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(machine.interrupt_matrix.route(0, 38), u8::MAX);
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
