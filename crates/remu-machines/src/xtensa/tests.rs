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
fn esp32s3_i2c0_mmio_executes_sgp30_transaction() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let at = SimTime::from_ticks(remu_devices::Sgp30::WARMUP_TICKS);
    const BASE: u64 = 0x6001_3000;
    const DATA: u64 = 0x1c;
    const CTR: u64 = 0x04;
    const COMMAND0: u64 = 0x58;
    const RESTART: u64 = 0;
    const WRITE: u64 = 1;
    const READ: u64 = 2;
    const STOP: u64 = 3;
    const END: u64 = 4;
    const fn command(bytes: u64, opcode: u64) -> u64 {
        bytes | (opcode << 11)
    }
    let write_word = |machine: &mut XtensaMachine, offset: u64, value: u64, at: SimTime| {
        machine
            .bus
            .write(BASE + offset, remu_core::AccessWidth::Word, value, at)
            .unwrap();
    };
    for byte in [0xb0, 0x20, 0x03] {
        write_word(&mut machine, DATA, byte, SimTime::ZERO);
    }
    for (index, command) in [
        command(0, RESTART),
        command(3, WRITE),
        command(0, STOP),
        command(0, END),
    ]
    .into_iter()
    .enumerate()
    {
        write_word(
            &mut machine,
            COMMAND0 + (index as u64 * 4),
            command,
            SimTime::ZERO,
        );
    }
    write_word(&mut machine, CTR, 0x30, SimTime::ZERO);

    for byte in [0xb0, 0x20, 0x08, 0xb1] {
        write_word(&mut machine, DATA, byte, at);
    }
    for (index, command) in [
        command(0, RESTART),
        command(3, WRITE),
        command(0, RESTART),
        command(1, WRITE),
        command(6, READ),
        command(0, STOP),
        command(0, END),
    ]
    .into_iter()
    .enumerate()
    {
        write_word(&mut machine, COMMAND0 + (index as u64 * 4), command, at);
    }
    write_word(&mut machine, CTR, 0x30, at);
    let measurement = (0..6)
        .map(|_| {
            machine
                .bus
                .read(
                    BASE + DATA,
                    remu_core::AccessWidth::Word,
                    remu_core::AccessKind::Read,
                    at,
                )
                .unwrap() as u8
        })
        .collect::<Vec<_>>();
    assert_eq!(measurement[0..2], [1, 164]);
    assert!(
        machine
            .signals
            .with_registry(|registry| registry.find("board.esp32s3.i2c0.sda"))
            .is_some()
    );
    assert!(
        machine
            .signals
            .with_registry(|registry| registry.find("board.esp32s3.i2c1.scl"))
            .is_some()
    );
}
