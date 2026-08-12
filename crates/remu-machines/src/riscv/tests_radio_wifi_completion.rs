#[test]
fn esp32c6_native_wifi_ack_timeout_is_delayed_and_publishes_vendor_status_five() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let descriptor = 0x4082_1000_u32;
    let buffer = 0x4082_1100_u32;
    let mut frame = vec![0x08, 0x00, 0, 0];
    frame.extend_from_slice(&[0x02, 6, 7, 8, 9, 10]);
    frame.extend_from_slice(&[0x02, 1, 2, 3, 4, 5]);
    frame.extend_from_slice(&[0; 8]);
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    let mut buffer_bytes = Vec::new();
    buffer_bytes.extend_from_slice(&((frame.len() + 4) as u32).to_le_bytes());
    buffer_bytes.extend_from_slice(&0_u32.to_le_bytes());
    buffer_bytes.extend_from_slice(&frame);
    buffer_bytes.extend_from_slice(&[0; 4]);
    machine
        .debug_write_memory(u64::from(buffer), &buffer_bytes)
        .unwrap();
    machine
        .bus
        .write(0x600a_4d68, AccessWidth::Word, 300, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_4d6c,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptor & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();

    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .bus
            .read(0x600a_4cb8, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap(),
        0
    );
    let deadline = u64::try_from(frame.len()).unwrap() * 32 + 300;
    machine.now = SimTime::from_ticks(deadline - 1);
    assert_eq!(machine.service_radio().unwrap(), 0);
    machine.now = SimTime::from_ticks(deadline);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .bus
            .read(0x600a_4cb8, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap(),
        1
    );
    assert_eq!(
        machine
            .bus
            .read(0x600a_54ec, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap() as u32
            & (0xf << 12),
        5 << 12
    );
}
