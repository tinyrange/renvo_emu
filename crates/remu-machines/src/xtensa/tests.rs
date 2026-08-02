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
fn esp32s3_lcd_cam_native_window_transmits_and_captures_host_words() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6004_1000;
    machine
        .lcd_cam()
        .queue_lcd_words([0x1122_3344, 0x5566_7788]);
    machine
        .lcd_cam()
        .queue_camera_words([0xdead_beef, 0xcafe_babe]);
    machine
        .bus
        .write(base + 0x64, AccessWidth::Word, 0x0f, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x28, AccessWidth::Word, 0x00ab_cdef, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            base + 0x14,
            AccessWidth::Word,
            (1 << 26) | (1 << 27),
            SimTime::from_ticks(2),
        )
        .unwrap();
    machine
        .bus
        .write(base + 0x04, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            base + 0x08,
            AccessWidth::Word,
            1 << 29,
            SimTime::from_ticks(3),
        )
        .unwrap();
    assert_eq!(
        machine.lcd_cam().take_lcd_words(),
        vec![0x00ab_cdef, 0x1122_3344, 0x5566_7788]
    );
    assert_eq!(
        machine.lcd_cam().take_camera_words(),
        vec![0xdead_beef, 0xcafe_babe]
    );
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x6c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x0e
    );
}
