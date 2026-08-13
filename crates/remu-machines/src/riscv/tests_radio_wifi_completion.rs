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
    descriptor_bytes.extend_from_slice(&(3_u32 << 30).to_le_bytes());
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

#[test]
fn esp32c6_descriptor_rts_defers_payload_until_cts_then_completes_on_ack() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let descriptor = 0x4082_1000_u32;
    let buffer = 0x4082_1100_u32;
    let receiver = [0x02, 6, 7, 8, 9, 10];
    let transmitter = [0x02, 1, 2, 3, 4, 5];
    let mut frame = vec![0x08, 0x00, 0x34, 0x12];
    frame.extend_from_slice(&receiver);
    frame.extend_from_slice(&transmitter);
    frame.extend_from_slice(&[0; 8]);
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&(3_u32 << 30).to_le_bytes());
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
        .write(0x600a_4d60, AccessWidth::Word, 1 << 31, SimTime::ZERO)
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
    let expected_rts = [
        0xb4, 0x00, 0x34, 0x12, 0x02, 6, 7, 8, 9, 10, 0x02, 1, 2, 3, 4, 5,
    ];
    assert!(machine
        .radio_replay_artifact()
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(
            event,
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.origin == remu_radio::FrameOrigin::Emulated
                    && request.frame.bytes == expected_rts
        )));
    assert!(!machine
        .radio_replay_artifact()
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(
            event,
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.origin == remu_radio::FrameOrigin::Emulated
                    && request.frame.bytes == frame
        )));

    let rts_end = u64::try_from(expected_rts.len()).unwrap() * 32;
    let mut cts = vec![0xc4, 0x00, 0, 0];
    cts.extend_from_slice(&transmitter);
    machine
        .inject_radio_frame_at(
            SimTime::from_ticks(rts_end),
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            cts,
            -35,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(rts_end + 10 * 32);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .bus
            .read(0x600a_4cb8, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap(),
        0
    );
    assert!(machine
        .radio_replay_artifact()
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(
            event,
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.origin == remu_radio::FrameOrigin::Emulated
                    && request.frame.bytes == frame
        )));

    let payload_end = machine.now.ticks() + u64::try_from(frame.len()).unwrap() * 32;
    let mut ack = vec![0xd4, 0x00, 0, 0];
    ack.extend_from_slice(&transmitter);
    machine
        .inject_radio_frame_at(
            SimTime::from_ticks(payload_end),
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            ack,
            -35,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(payload_end + 10 * 32);
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
        0
    );
}

#[test]
fn esp32c6_descriptor_rts_timeout_publishes_vendor_status_two() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let descriptor = 0x4082_1000_u32;
    let buffer = 0x4082_1100_u32;
    let mut frame = vec![0x08, 0x00, 0, 0];
    frame.extend_from_slice(&[0x02, 6, 7, 8, 9, 10]);
    frame.extend_from_slice(&[0x02, 1, 2, 3, 4, 5]);
    frame.extend_from_slice(&[0; 8]);
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&(3_u32 << 30).to_le_bytes());
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
        .write(0x600a_4d60, AccessWidth::Word, 1 << 31, SimTime::ZERO)
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
    machine.now = SimTime::from_ticks(16 * 32 + 300);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .bus
            .read(0x600a_54ec, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap() as u32
            & (0xf << 12),
        2 << 12
    );
}

#[test]
fn esp32c6_descriptor_chain_emits_one_ampdu_and_publishes_partial_block_ack() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let descriptors = [0x4082_1000_u32, 0x4082_1020];
    let buffers = [0x4082_1100_u32, 0x4082_1200];
    let receiver = [0x02, 6, 7, 8, 9, 10];
    let transmitter = [0x02, 1, 2, 3, 4, 5];
    let frames = [0x100_u16, 0x101]
        .into_iter()
        .map(|sequence| {
            let mut frame = vec![0x88, 0, 0, 0];
            frame.extend_from_slice(&receiver);
            frame.extend_from_slice(&transmitter);
            frame.extend_from_slice(&[0; 6]);
            frame.extend_from_slice(&(sequence << 4).to_le_bytes());
            frame.extend_from_slice(&[((3 << 5) | 5), 0]);
            frame
        })
        .collect::<Vec<_>>();
    for index in 0..2 {
        let next = if index == 0 { descriptors[1] } else { 0 };
        let mut descriptor = Vec::new();
        let control = (1_u32 << 31) | if next == 0 { 1 << 30 } else { 0 };
        descriptor.extend_from_slice(&control.to_le_bytes());
        descriptor.extend_from_slice(&buffers[index].to_le_bytes());
        descriptor.extend_from_slice(&next.to_le_bytes());
        machine
            .debug_write_memory(u64::from(descriptors[index]), &descriptor)
            .unwrap();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&((frames[index].len() + 4) as u32).to_le_bytes());
        buffer.extend_from_slice(&0_u32.to_le_bytes());
        buffer.extend_from_slice(&frames[index]);
        buffer.extend_from_slice(&[0; 4]);
        machine
            .debug_write_memory(u64::from(buffers[index]), &buffer)
            .unwrap();
    }
    machine
        .bus
        .write(
            0x600a_4d6c,
            AccessWidth::Word,
            u64::from((3_u32 << 30) | (descriptors[0] & 0x000f_ffff)),
            SimTime::ZERO,
        )
        .unwrap();

    assert_eq!(machine.service_radio().unwrap(), 1);
    assert!(machine
        .radio_replay_artifact()
        .unwrap()
        .events
        .iter()
        .any(|event| matches!(
            event,
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.origin == remu_radio::FrameOrigin::Emulated
                    && request.frame.phy == "wifi-ht20-ampdu"
                    && request.frame.bytes.is_empty()
                    && request.frame.mpdus == frames
        )));

    let tx_end = frames.iter().map(Vec::len).sum::<usize>() as u64 * 32;
    let mut block_ack = vec![0x94, 0, 0, 0];
    block_ack.extend_from_slice(&transmitter);
    block_ack.extend_from_slice(&receiver);
    block_ack.extend_from_slice(&(0x0004_u16 | (5 << 12)).to_le_bytes());
    block_ack.extend_from_slice(&(0x100_u16 << 4).to_le_bytes());
    block_ack.extend_from_slice(&1_u64.to_le_bytes());
    machine
        .inject_radio_frame_at(
            SimTime::from_ticks(tx_end),
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            block_ack,
            -35,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(tx_end + 28 * 32);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .bus
            .read(0x600a_54e8, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap() as u32
            & (0xff << 16),
        1 << 16
    );
    assert_eq!(
        machine
            .bus
            .read(0x600a_54dc, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap() as u32
            & 0x000f_0fff,
        0x100
    );
    assert_eq!(
        machine
            .bus
            .read(0x600a_54d8, AccessWidth::Word, AccessKind::Read, machine.now)
            .unwrap(),
        1
    );
}
