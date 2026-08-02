use super::*;
use remu_devices::Esp32S3HmacRegister;

#[test]
fn direct_load_starts_with_appcpu_reset_and_parked() {
    let machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    assert!(machine.appcpu_boot_address.is_none());
    assert_eq!(machine.cpu1.snapshot().pc, 0);
    assert!(!machine.cpu1.snapshot().waiting);
    assert!(!machine.cpu1.snapshot().halted);
}

#[test]
fn esp32s3_hmac_native_register_window_produces_sha256_digest() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6003_e000;
    let write = |machine: &mut XtensaMachine, register: Esp32S3HmacRegister, value: u64| {
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
    write(&mut machine, Esp32S3HmacRegister::SetStart, 1);
    write(&mut machine, Esp32S3HmacRegister::SetParaPurpose, 8);
    write(&mut machine, Esp32S3HmacRegister::SetParaKey, 2);
    write(&mut machine, Esp32S3HmacRegister::SetParaFinish, 1);
    for index in 0..16_u64 {
        let start = index * 4;
        let word = start | ((start + 1) << 8) | ((start + 2) << 16) | ((start + 3) << 24);
        write(
            &mut machine,
            Esp32S3HmacRegister::from_offset(Esp32S3HmacRegister::Wdata0.offset() + index * 4)
                .expect("HMAC write window register"),
            word,
        );
    }
    write(&mut machine, Esp32S3HmacRegister::SetMessageOne, 1);

    assert_eq!(
        machine
            .bus
            .read(
                base + Esp32S3HmacRegister::QueryError.offset(),
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0
    );
    let digest = (0..8_u64)
        .flat_map(|index| {
            u32::try_from(
                machine
                    .bus
                    .read(
                        base + Esp32S3HmacRegister::Rdata0.offset() + index * 4,
                        AccessWidth::Word,
                        AccessKind::Read,
                        SimTime::ZERO,
                    )
                    .unwrap(),
            )
            .unwrap()
            .to_le_bytes()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        digest,
        [
            0x14, 0x3e, 0xa7, 0x44, 0xc3, 0x9c, 0x49, 0xdc, 0xc2, 0x90, 0x53, 0xc5, 0x5d, 0x37,
            0xd2, 0xc3, 0xa3, 0x08, 0xf4, 0xb2, 0x22, 0xc3, 0xea, 0x21, 0x30, 0xbc, 0x66, 0x89,
            0xbf, 0x5d, 0x96, 0x5b,
        ]
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
