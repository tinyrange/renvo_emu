use super::*;
use remu_devices::Esp32S3TsensRegister;

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
fn esp32s3_tsens_native_window_reports_ready_raw_code_and_interrupt() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6000_8800;
    machine.tsens().set_raw(173);
    machine
        .bus
        .write(
            base + Esp32S3TsensRegister::SarCocpuIntEna.offset(),
            AccessWidth::Word,
            1 << 5,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            base + Esp32S3TsensRegister::TsensCtrl.offset(),
            AccessWidth::Word,
            (1 << 22) | (1 << 12),
            SimTime::from_ticks(3),
        )
        .unwrap();
    let control = machine
        .bus
        .read(
            base + Esp32S3TsensRegister::TsensCtrl.offset(),
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(control & 0xff, 173);
    assert_ne!(control & (1 << 8), 0);
    assert_eq!(
        machine
            .bus
            .read(
                base + Esp32S3TsensRegister::SarCocpuIntStatus.offset(),
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        1 << 5
    );
    machine
        .bus
        .write(
            base + Esp32S3TsensRegister::SarCocpuIntClear.offset(),
            AccessWidth::Word,
            1 << 5,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                base + Esp32S3TsensRegister::SarCocpuIntStatus.offset(),
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0
    );
}
