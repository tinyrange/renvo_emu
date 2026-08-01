use super::*;

#[test]
fn verified_xip_requires_the_rom_instruction_cache_configuration() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine.bus.load(0x4200_0000, &[0x3d, 0xf0]).unwrap();
    machine.cpu.set_direct_state(0x3fc8_0100, 0x4200_0000);

    let blocked = machine
        .run(
            RunLimits {
                instructions: Some(1),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert!(matches!(
        blocked.reason,
        StopReason::Fault(message) if message.contains("instruction-cache configuration")
    ));

    machine.cpu.set_direct_state(0x3fc8_0100, 0x4000_1a1c);
    machine.cpu.set_register(XtensaRegister::A2, 0x4000);
    machine.cpu.set_register(XtensaRegister::A3, 8);
    machine.cpu.set_register(XtensaRegister::A4, 32);
    assert!(machine.service_functional_rom().unwrap());
    assert!(machine.instruction_cache_configured);

    machine.cpu.set_direct_state(0x3fc8_0100, 0x4200_0000);
    let allowed = machine
        .run(
            RunLimits {
                instructions: Some(1),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(allowed.reason, StopReason::InstructionLimit);
}

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
