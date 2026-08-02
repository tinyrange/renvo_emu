use super::*;
use remu_devices::Esp32S3DigitalSignatureRegister;

#[test]
fn direct_load_starts_with_appcpu_reset_and_parked() {
    let machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    assert!(machine.appcpu_boot_address.is_none());
    assert_eq!(machine.cpu1.snapshot().pc, 0);
    assert!(!machine.cpu1.snapshot().waiting);
    assert!(!machine.cpu1.snapshot().halted);
}

#[test]
fn esp32s3_digital_signature_native_commands_report_deterministic_status() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6003_d000;
    let write =
        |machine: &mut XtensaMachine, register: Esp32S3DigitalSignatureRegister, value: u64| {
            machine
                .bus
                .write(
                    base + register.offset(),
                    AccessWidth::Word,
                    value,
                    SimTime::ZERO,
                )
                .unwrap();
        };
    write(&mut machine, Esp32S3DigitalSignatureRegister::C(0), 1);
    write(
        &mut machine,
        Esp32S3DigitalSignatureRegister::X(0),
        0x0102_0304,
    );
    write(&mut machine, Esp32S3DigitalSignatureRegister::SetStart, 1);
    write(&mut machine, Esp32S3DigitalSignatureRegister::SetMe, 1);
    assert_eq!(
        machine
            .bus
            .read(
                base + Esp32S3DigitalSignatureRegister::QueryBusy.offset(),
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0
    );
    assert_eq!(
        machine
            .bus
            .read(
                base + Esp32S3DigitalSignatureRegister::QueryKeyWrong.offset(),
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0
    );
    assert_eq!(
        machine
            .bus
            .read(
                base + Esp32S3DigitalSignatureRegister::QueryCheck.offset(),
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0
    );
    assert_ne!(
        machine
            .bus
            .read(
                base + Esp32S3DigitalSignatureRegister::Z(0).offset(),
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0
    );
    write(&mut machine, Esp32S3DigitalSignatureRegister::SetFinish, 1);
    assert_eq!(
        machine
            .bus
            .read(
                base + Esp32S3DigitalSignatureRegister::Z(0).offset(),
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
