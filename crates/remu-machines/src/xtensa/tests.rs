use super::*;
use remu_devices::Esp32S3AesRegister;

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
fn esp32s3_aes_native_text_window_matches_standard_vectors() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6003_a000;
    let key = [
        0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f,
        0x3c,
    ];
    let plaintext = [
        0x32, 0x43, 0xf6, 0xa8, 0x88, 0x5a, 0x30, 0x8d, 0x31, 0x31, 0x98, 0xa2, 0xe0, 0x37, 0x07,
        0x34,
    ];
    let ciphertext = [
        0x39, 0x25, 0x84, 0x1d, 0x02, 0xdc, 0x09, 0xfb, 0xdc, 0x11, 0x85, 0x97, 0x19, 0x6a, 0x0b,
        0x32,
    ];
    fn write_block(
        machine: &mut XtensaMachine,
        base: u64,
        register: Esp32S3AesRegister,
        bytes: &[u8],
    ) {
        let offset = register.offset();
        for (index, word) in bytes.chunks_exact(4).enumerate() {
            machine
                .bus
                .write(
                    base + offset + (index as u64 * 4),
                    AccessWidth::Word,
                    u64::from(u32::from_le_bytes(word.try_into().unwrap())),
                    SimTime::ZERO,
                )
                .unwrap();
        }
    }
    fn read_block(machine: &mut XtensaMachine, base: u64, register: Esp32S3AesRegister) -> Vec<u8> {
        let offset = register.offset();
        (0..4)
            .map(|index| {
                (machine
                    .bus
                    .read(
                        base + offset + (index * 4),
                        AccessWidth::Word,
                        AccessKind::Read,
                        SimTime::ZERO,
                    )
                    .unwrap() as u32)
                    .to_le_bytes()
            })
            .flatten()
            .collect::<Vec<_>>()
    }

    write_block(&mut machine, base, Esp32S3AesRegister::Key0, &key);
    write_block(&mut machine, base, Esp32S3AesRegister::TextIn0, &plaintext);
    machine
        .bus
        .write(
            base + Esp32S3AesRegister::IntEna.offset(),
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            base + Esp32S3AesRegister::Trigger.offset(),
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        read_block(&mut machine, base, Esp32S3AesRegister::TextOut0),
        ciphertext
    );
    // Typical CPU-driven AES is polled through AES_STATE_REG; only DMA-AES
    // completion raises the interrupt source.
    assert!(!machine.aes().interrupt_pending());

    write_block(&mut machine, base, Esp32S3AesRegister::TextIn0, &ciphertext);
    machine
        .bus
        .write(
            base + Esp32S3AesRegister::Mode.offset(),
            AccessWidth::Word,
            0x04,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            base + Esp32S3AesRegister::Trigger.offset(),
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        read_block(&mut machine, base, Esp32S3AesRegister::TextOut0),
        plaintext
    );
}
