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
    let message = b"renvo-hmac";
    let mut block = [0_u8; 64];
    block[..message.len()].copy_from_slice(message);
    block[message.len()] = 0x80;
    block[56..].copy_from_slice(&(512_u64 + (message.len() as u64 * 8)).to_be_bytes());
    for (index, chunk) in block.chunks_exact(4).enumerate() {
        write(
            &mut machine,
            Esp32S3HmacRegister::from_offset(
                Esp32S3HmacRegister::Wdata0.offset() + (index as u64 * 4),
            )
            .expect("HMAC write window register"),
            u64::from(u32::from_le_bytes(chunk.try_into().unwrap())),
        );
    }
    write(&mut machine, Esp32S3HmacRegister::SetMessageOne, 1);
    write(&mut machine, Esp32S3HmacRegister::OneBlock, 1);

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
            0xec, 0xe8, 0x02, 0x8a, 0xaa, 0x34, 0x64, 0x62, 0x6b, 0xdc, 0x42, 0x3b, 0xae, 0xc8,
            0xa4, 0x08, 0x78, 0x49, 0xb0, 0xef, 0x93, 0x2b, 0x5f, 0x66, 0x0a, 0x3b, 0xda, 0x9d,
            0x2b, 0x9f, 0x8b, 0x46,
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
