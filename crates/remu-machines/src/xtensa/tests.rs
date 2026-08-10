use super::*;
use remu_image::{
    EspExecutableImage, EspFlashImage, EspImageHeader, EspImageSegment, EspPartition,
    EspPartitionTable, FirmwareSegment,
};

fn image_with_segment(address: u64, data: Vec<u8>) -> FirmwareImage {
    FirmwareImage {
        architecture: FirmwareArchitecture::Xtensa,
        entry: 0x4037_0000,
        segments: vec![FirmwareSegment {
            address,
            load_address: None,
            initialized_size: data.len(),
            data,
            executable: false,
            writable: true,
            alignment: 4,
        }],
        symbols: Vec::new(),
    }
}

#[test]
fn esp32s3_wifi_and_ble_use_shared_deterministic_radio_api() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let mut wifi_frame = vec![0_u8; 24];
    wifi_frame[4..10].fill(0xff);
    machine
        .wifi_engine()
        .unwrap()
        .start(remu_radio::WifiMode::Station)
        .unwrap();
    machine
        .wifi_engine()
        .unwrap()
        .queue_tx(wifi_frame.clone())
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert!(
        machine
            .radio_replay_artifact()
            .events
            .iter()
            .any(|event| matches!(
                event,
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.protocol == remu_radio::RadioProtocol::Wifi
                        && request.frame.origin == remu_radio::FrameOrigin::Emulated
            ))
    );
    machine.now = SimTime::from_ticks(192);
    assert_eq!(machine.service_radio().unwrap(), 0);

    machine
        .ble_controller()
        .unwrap()
        .process_h4(&[1, 3, 12, 0])
        .unwrap();
    assert_eq!(
        machine.ble_controller().unwrap().take_h4_output(),
        Some(vec![4, 0x0e, 4, 1, 3, 12, 0])
    );

    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            wifi_frame.clone(),
            0,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(384);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(machine.wifi_engine().unwrap().take_rx(), Some(wifi_frame));
}

#[test]
fn esp32s3_illegal_native_wifi_dma_is_a_hard_machine_error() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .bus
        .write(
            0x6003_3d08,
            AccessWidth::Word,
            (3_u64 << 30) | 2,
            SimTime::ZERO,
        )
        .unwrap();

    let error = machine.service_radio().unwrap_err();
    let XtensaMachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::DmaAddress);
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Wifi);
    assert!(error.to_string().contains("0x3fc00002"));
}

#[test]
fn esp32s3_native_ble_scheduler_transmits_exchange_memory_pdu_and_completes() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let slot_address = 0x3fca_1000_u32;
    let cs_address = 0x3fca_2000_u32;
    let descriptor_address = 0x3fca_3000_u32;
    let payload_address = 0x3fca_4000_u32;
    for (index, (em_offset, cpu_address)) in [
        (0x0000_u32, slot_address),
        (0x0400, cs_address),
        (0x1400, descriptor_address),
        (0x2400, payload_address),
    ]
    .into_iter()
    .enumerate()
    {
        let mapping = ((em_offset >> 2) << 18) | ((cpu_address & 0x000f_ffff) >> 2);
        machine
            .bus
            .write(
                0x6003_1204 + index as u64 * 4,
                AccessWidth::Word,
                u64::from(mapping),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x6003_12c4, AccessWidth::Word, 0x0f, SimTime::ZERO)
        .unwrap();

    let mut slot = [0_u8; 16];
    slot[0..2].copy_from_slice(&0x2802_u16.to_le_bytes());
    slot[2..6].copy_from_slice(&2_u32.to_le_bytes());
    slot[6..8].copy_from_slice(&624_u16.to_le_bytes());
    // The scheduler stores the 90-byte BLE control-structure offset divided by two.
    slot[8..10].copy_from_slice(&0x0200_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(slot_address), &slot)
        .unwrap();

    let mut control_structure = [0_u8; 90];
    control_structure[12..16].copy_from_slice(&0x8e89_bed6_u32.to_le_bytes());
    control_structure[16..20].copy_from_slice(&0x0055_5555_u32.to_le_bytes());
    control_structure[22..24].copy_from_slice(&39_u16.to_le_bytes());
    control_structure[28..30].copy_from_slice(&0x1400_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(cs_address), &control_structure)
        .unwrap();

    let advertising_data = b"\x02\x01\x06\x0b\x09Renvo-BLE1";
    let mut descriptor = [0_u8; 32];
    descriptor[2..4].copy_from_slice(&0x1546_u16.to_le_bytes());
    descriptor[4..6].copy_from_slice(&0x2400_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor_address), &descriptor)
        .unwrap();
    machine
        .debug_write_memory(u64::from(payload_address), advertising_data)
        .unwrap();
    machine
        .bus
        .write(0x6003_1100, AccessWidth::Word, 1 << 31, SimTime::ZERO)
        .unwrap();

    assert_eq!(machine.service_radio().unwrap(), 0);
    machine.now = SimTime::from_ticks(9_999);
    assert_eq!(machine.service_radio().unwrap(), 0);
    machine.now = SimTime::from_ticks(10_000);
    assert_eq!(machine.service_radio().unwrap(), 1);
    let replay = machine.radio_replay_artifact();
    let request = replay
        .events
        .iter()
        .find_map(|event| match event {
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.protocol == remu_radio::RadioProtocol::BluetoothLe =>
            {
                Some(request)
            }
            _ => None,
        })
        .unwrap();
    let mut expected = vec![0x46, 0x15, 0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
    expected.extend_from_slice(advertising_data);
    assert_eq!(request.frame.bytes, expected);
    assert_eq!(request.frame.spectrum.center_khz, 2_480_000);
    assert_eq!(request.frame.origin, remu_radio::FrameOrigin::Emulated);
    assert_eq!(request.start, SimTime::from_ticks(10_000));
    assert_eq!(
        machine
            .bus
            .read(
                0x6003_1010,
                AccessWidth::Word,
                remu_core::AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );

    machine.now = SimTime::from_ticks(10_000 + (expected.len() * 8) as u64);
    machine.service_radio().unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                u64::from(slot_address),
                AccessWidth::HalfWord,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & 0x38,
        2 << 3
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x6003_1010,
                AccessWidth::Word,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        0
    );
    machine.now = SimTime::from_ticks(10_000 + (expected.len() * 8) as u64 + 2_400);
    machine.service_radio().unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                u64::from(slot_address),
                AccessWidth::HalfWord,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & 0x38,
        4 << 3
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x6003_1010,
                AccessWidth::Word,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        1 << 5
    );
}

#[test]
fn esp32s3_native_ble_scan_writes_receive_ring_metadata_and_interrupt() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let slot_address = 0x3fca_1000_u32;
    let cs_address = 0x3fca_2000_u32;
    let descriptor_address = 0x3fca_3000_u32;
    let payload_address = 0x3fca_4000_u32;
    for (index, (em_offset, cpu_address)) in [
        (0x0000_u32, slot_address),
        (0x0400, cs_address),
        (0x1000, descriptor_address),
        (0x3000, payload_address),
    ]
    .into_iter()
    .enumerate()
    {
        let mapping = ((em_offset >> 2) << 18) | ((cpu_address & 0x000f_ffff) >> 2);
        machine
            .bus
            .write(
                0x6003_1204 + index as u64 * 4,
                AccessWidth::Word,
                u64::from(mapping),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x6003_12c4, AccessWidth::Word, 0x0f, SimTime::ZERO)
        .unwrap();

    let mut slot = [0_u8; 16];
    slot[0..2].copy_from_slice(&0x0208_u16.to_le_bytes());
    slot[2..6].copy_from_slice(&2_u32.to_le_bytes());
    slot[6..8].copy_from_slice(&624_u16.to_le_bytes());
    slot[8..10].copy_from_slice(&0x0200_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(slot_address), &slot)
        .unwrap();
    let mut control_structure = [0_u8; 90];
    control_structure[12..16].copy_from_slice(&0x8e89_bed6_u32.to_le_bytes());
    control_structure[22..24].copy_from_slice(&39_u16.to_le_bytes());
    control_structure[32..34].copy_from_slice(&16_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(cs_address), &control_structure)
        .unwrap();
    let mut descriptor = [0_u8; 20];
    descriptor[0..2].copy_from_slice(&0x1000_u16.to_le_bytes());
    descriptor[2..4].copy_from_slice(&0x8000_u16.to_le_bytes());
    descriptor[18..20].copy_from_slice(&0x3000_u16.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor_address), &descriptor)
        .unwrap();
    machine
        .bus
        .write(0x6003_1100, AccessWidth::Word, 1 << 31, SimTime::ZERO)
        .unwrap();
    machine.service_radio().unwrap();

    let pdu = vec![
        0x42, 0x0c, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xc1, 0x02, 0x01, 0x06, 0x02, 0x09, 0x52,
    ];
    machine
        .inject_radio_frame_at(
            SimTime::from_ticks(10_000),
            remu_radio::RadioProtocol::BluetoothLe,
            remu_radio::Spectrum::new(2_480_000, 2_000),
            "ble-1m",
            pdu.clone(),
            -40,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(10_000 + (pdu.len() * 8) as u64);
    assert_eq!(machine.service_radio().unwrap(), 1);

    let completed = machine
        .debug_read_memory(u64::from(descriptor_address), 20)
        .unwrap();
    assert_eq!(
        u16::from_le_bytes(completed[0..2].try_into().unwrap()),
        0x9000
    );
    assert_eq!(u16::from_le_bytes(completed[2..4].try_into().unwrap()), 0);
    assert_eq!(
        u16::from_le_bytes(completed[4..6].try_into().unwrap()),
        0x0c42
    );
    assert_eq!(u16::from_le_bytes(completed[12..14].try_into().unwrap()), 0);
    assert_eq!(completed[6], 0xb0);
    assert_eq!(
        u16::from_le_bytes(completed[14..16].try_into().unwrap()),
        0x0027
    );
    assert_eq!(
        machine
            .debug_read_memory(u64::from(payload_address), pdu.len() - 2)
            .unwrap(),
        pdu[2..]
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x6003_1010,
                AccessWidth::Word,
                remu_core::AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        1 << 2
    );
}

#[test]
fn esp32s3_native_wifi_rx_dma_writes_metadata_frame_and_completion() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let descriptor = 0x3fca_1000_u32;
    let buffer = 0x3fca_1100_u32;
    let capacity = 512_u32;
    let control = (1 << 31) | (capacity << 12) | capacity;
    let mut descriptor_bytes = Vec::new();
    descriptor_bytes.extend_from_slice(&control.to_le_bytes());
    descriptor_bytes.extend_from_slice(&buffer.to_le_bytes());
    descriptor_bytes.extend_from_slice(&0_u32.to_le_bytes());
    machine
        .debug_write_memory(u64::from(descriptor), &descriptor_bytes)
        .unwrap();
    machine
        .bus
        .write(
            0x6003_3088,
            AccessWidth::Word,
            u64::from(descriptor),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x6003_3c34, AccessWidth::Word, 1 << 14, SimTime::ZERO)
        .unwrap();
    let frame = vec![0x80, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2, 3];
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            frame.clone(),
            -40,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(256);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .debug_read_memory(u64::from(buffer) + 48, frame.len())
            .unwrap(),
        frame
    );
    let metadata = machine.debug_read_memory(u64::from(buffer), 48).unwrap();
    assert_eq!(metadata[0], (-40_i8) as u8);
    assert_eq!(metadata[3] & (1 << 4), 1 << 4);
    assert_eq!(metadata[8] & (1 << 1), 1 << 1);
    assert_eq!(metadata[10], 0);
    assert_eq!(metadata[11] & 0x0f, 1);
    assert_eq!(metadata[11] & (1 << 7), 1 << 7);
    assert_eq!(metadata[20], (-95_i8) as u8);
    assert_eq!(
        u32::from_le_bytes(metadata[44..48].try_into().unwrap()) & 0x0fff,
        16
    );
    let completed = u32::from_le_bytes(
        machine
            .debug_read_memory(u64::from(descriptor), 4)
            .unwrap()
            .try_into()
            .unwrap(),
    );
    assert_eq!(completed & (1 << 31), 0);
    assert_ne!(completed & (1 << 30), 0);
    assert_eq!((completed >> 12) & 0x0fff, 64);
    assert_ne!(
        machine
            .bus
            .read(
                0x6003_3c3c,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & (1 << 14),
        0
    );
}

fn merged_flash_image() -> EspFlashImage {
    let header = EspImageHeader {
        segment_count: 3,
        flash_mode: 0,
        flash_size_frequency: 0,
        entry: 0x4200_0020,
        write_protect_pin: 0,
        drive_settings: [0; 3],
        chip_id: 9,
        minimum_revision_legacy: 0,
        minimum_revision: 0,
        maximum_revision: u16::MAX,
        hash_appended: false,
    };
    let application = EspExecutableImage {
        flash_offset: 0x10_000,
        header: header.clone(),
        segments: vec![
            EspImageSegment {
                address: 0x4200_0020,
                flash_offset: 0x10_020,
                data: vec![0x11, 0x22, 0x33, 0x44],
            },
            EspImageSegment {
                address: 0x3c00_1000,
                flash_offset: 0x11_000,
                data: vec![0xa1, 0xb2, 0xc3, 0xd4],
            },
            EspImageSegment {
                address: 0x3fc8_8000,
                flash_offset: 0x12_000,
                data: vec![0x55, 0x66, 0x77, 0x88],
            },
        ],
        checksum: 0,
        appended_sha256: None,
        end_offset: 0x12_100,
    };
    let application_partition = EspPartition {
        partition_type: 0,
        subtype: 0,
        offset: 0x10_000,
        size: 0x20_000,
        label: "factory".to_owned(),
        flags: 0,
    };
    EspFlashImage {
        bootloader: EspExecutableImage {
            flash_offset: 0,
            header,
            segments: Vec::new(),
            checksum: 0,
            appended_sha256: None,
            end_offset: 0,
        },
        partition_table: EspPartitionTable {
            partitions: vec![application_partition.clone()],
            has_md5: false,
        },
        application,
        application_partition,
    }
}

fn handoff_image(entry: u32, code: &[u8]) -> EspFlashImage {
    let header = || EspImageHeader {
        segment_count: 1,
        flash_mode: 2,
        flash_size_frequency: 0x20,
        entry,
        write_protect_pin: 0xee,
        drive_settings: [0; 3],
        chip_id: 9,
        minimum_revision_legacy: 0,
        minimum_revision: 0,
        maximum_revision: 0x63,
        hash_appended: false,
    };
    EspFlashImage {
        bootloader: EspExecutableImage {
            flash_offset: 0,
            header: header(),
            segments: Vec::new(),
            checksum: 0xef,
            appended_sha256: None,
            end_offset: 0,
        },
        partition_table: EspPartitionTable {
            partitions: Vec::new(),
            has_md5: false,
        },
        application: EspExecutableImage {
            flash_offset: 0,
            header: header(),
            segments: vec![EspImageSegment {
                address: entry,
                flash_offset: 0x1000,
                data: code.to_vec(),
            }],
            checksum: 0xef,
            appended_sha256: None,
            end_offset: 0x1000 + code.len() as u32,
        },
        application_partition: EspPartition {
            partition_type: 0,
            subtype: 0,
            offset: 0,
            size: 0,
            label: "factory".to_owned(),
            flags: 0,
        },
    }
}

fn load_handoff_application(machine: &mut XtensaMachine, entry: u32, code: &[u8]) {
    let mut flash = vec![0xff; 0x1000 + code.len()];
    flash[0x1000..].copy_from_slice(code);
    machine.set_esp_flash_image(&flash);
    machine
        .load_esp_application(&handoff_image(entry, code))
        .unwrap();
}

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
fn verified_handoff_requires_entry_and_rotates_callx8_window() {
    let mut missing_entry = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    load_handoff_application(&mut missing_entry, 0x4037_0040, &[0x3d, 0xf0]);
    let fault = missing_entry
        .run(
            RunLimits {
                instructions: Some(1),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert!(matches!(
        fault.reason,
        StopReason::Fault(message) if message.contains("requires ENTRY")
    ));

    let mut valid = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    load_handoff_application(&mut valid, 0x4037_0040, &[0x36, 0x41, 0x00, 0x3d, 0xf0]);
    let result = valid
        .run(
            RunLimits {
                instructions: Some(1),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(result.reason, StopReason::InstructionLimit);
    let ps = result
        .cpu
        .registers
        .iter()
        .find(|register| register.name == "ps")
        .expect("Xtensa snapshot includes PS")
        .value;
    assert_eq!(ps & (3 << 16), 0);
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
fn appcpu_release_is_driven_by_the_system_boot_vector_register() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .bus
        .write(0x600c_0004, AccessWidth::Word, 0x4037_5690, SimTime::ZERO)
        .unwrap();

    machine.release_appcpu_if_requested();

    assert_eq!(machine.appcpu_boot_address, Some(0x4037_5690));
    assert_eq!(machine.cpu1.snapshot().pc, 0x4037_5690);
    assert_eq!(
        machine
            .cpu1
            .snapshot()
            .registers
            .iter()
            .find(|register| register.name == "a1")
            .unwrap()
            .value,
        0x3fce_a710
    );
}

#[test]
fn esp32s3_dram_starts_at_the_documented_soc_boundary() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let invalid = image_with_segment(0x3fc8_0000, vec![0; 4]);
    let error = machine.load_firmware(&invalid).unwrap_err();
    assert!(matches!(
        error,
        XtensaMachineError::Load {
            address: 0x3fc8_0000,
            ..
        }
    ));

    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let valid = image_with_segment(0x3fc8_8000, vec![0xa5; 4]);
    machine.load_firmware(&valid).unwrap();
}

#[test]
fn esp32s3_dram_upper_boundary_is_exclusive() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let last_word = image_with_segment(0x3fcf_fffc, vec![0xa5; 4]);
    machine.load_firmware(&last_word).unwrap();

    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let outside = image_with_segment(0x3fd0_0000, vec![0; 1]);
    assert!(machine.load_firmware(&outside).is_err());
}

#[test]
fn direct_elf_load_leaves_the_bss_tail_poisoned() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let initialized = [0x13, 0, 0, 0];
    let mut data = initialized.to_vec();
    data.resize(12, 0);
    let image = FirmwareImage {
        architecture: FirmwareArchitecture::Xtensa,
        entry: 0x3fc8_8000,
        segments: vec![FirmwareSegment {
            address: 0x3fc8_8000,
            load_address: None,
            data,
            initialized_size: initialized.len(),
            executable: true,
            writable: true,
            alignment: 4,
        }],
        symbols: Vec::new(),
    };

    machine.load_firmware(&image).unwrap();

    assert_eq!(
        machine.debug_read_memory(0x3fc8_8000, 12).unwrap(),
        [
            0x13, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5
        ]
    );
}

#[test]
fn esp32s3_auxiliary_uarts_capture_transmit_fifo_writes() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .bus
        .write(0x6001_0000, AccessWidth::Word, 0x31, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x6002_e000, AccessWidth::Word, 0x32, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.auxiliary_uarts[0].bytes(), [0x31]);
    assert_eq!(machine.auxiliary_uarts[1].bytes(), [0x32]);
    assert_eq!(
        machine
            .bus
            .read(
                0x6001_001c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );
}

#[test]
fn esp32s3_exposes_the_m5sticks3_octal_psram_window() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .debug_write_memory(0x3d00_0000, &[0x12, 0x34, 0x56, 0x78])
        .unwrap();
    machine
        .debug_write_memory(0x3d7f_fffc, &[0xa5, 0x5a, 0xc3, 0x3c])
        .unwrap();

    assert_eq!(
        machine.debug_read_memory(0x3d00_0000, 4).unwrap(),
        vec![0x12, 0x34, 0x56, 0x78]
    );
    assert_eq!(
        machine.debug_read_memory(0x3d7f_fffc, 4).unwrap(),
        vec![0xa5, 0x5a, 0xc3, 0x3c]
    );
    assert!(machine.debug_read_memory(0x3d80_0000, 1).is_err());
}

#[test]
fn esp32s3_machine_exposes_high_gpio_bank_signals() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    assert_eq!(machine.chip_gpio.pin_count(), 49);
    machine
        .add_signal_stop("board.esp32s3.chip_gpio.pin38", SignalEdge::Rising)
        .unwrap();
    machine.set_pin(38, Logic::One).unwrap();
    assert_eq!(machine.chip_gpio.resolved(38).unwrap(), Logic::One);
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
fn esp32s3_flash_application_loads_xip_and_dram_segments() {
    let image = merged_flash_image();
    let mut flash = vec![0xff; 0x13_000];
    flash[0x10_020..0x10_024].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
    flash[0x11_000..0x11_004].copy_from_slice(&[0xa1, 0xb2, 0xc3, 0xd4]);

    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine.set_esp_flash_image(&flash);
    machine.load_esp_application(&image).unwrap();
    machine.apply_pending_mmu_mappings().unwrap();

    assert_eq!(
        machine.debug_read_memory(0x4200_0020, 4).unwrap(),
        vec![0x11, 0x22, 0x33, 0x44]
    );
    assert_eq!(
        machine.debug_read_memory(0x3c00_1000, 4).unwrap(),
        vec![0xa1, 0xb2, 0xc3, 0xd4]
    );
    assert_eq!(
        machine.debug_read_memory(0x3fc8_8000, 4).unwrap(),
        vec![0x55, 0x66, 0x77, 0x88]
    );
    assert_eq!(machine.debug_snapshot().pc, 0x4200_0020);
}

#[test]
fn esp32s3_flash_application_rejects_noncongruent_xip_segments() {
    let mut image = merged_flash_image();
    image.application.segments[0].flash_offset = 0x10_001;
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine.set_esp_flash_image(&[0xff; 0x20_000]);
    let error = machine.load_esp_application(&image).unwrap_err();
    assert!(matches!(
        error,
        XtensaMachineError::Load {
            address: 0x4200_0020,
            ..
        }
    ));
}

#[test]
fn esp32s3_flash_application_rejects_segments_outside_simulated_flash() {
    let mut image = merged_flash_image();
    image.application.segments[0].flash_offset = 16 * 1024 * 1024;
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine.set_esp_flash_image(&[0xff; 0x20_000]);
    let error = machine.load_esp_application(&image).unwrap_err();
    assert!(matches!(
        error,
        XtensaMachineError::Load {
            address: 0x4200_0020,
            ..
        }
    ));
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

#[test]
fn esp32s3_spi3_mmio_executes_fifo_loopback_and_exposes_waveforms() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    const BASE: u64 = 0x6002_5000;
    machine
        .bus
        .write(
            BASE + Esp32s3Spi::W0,
            AccessWidth::Word,
            0x3c_a5_0000,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            BASE + Esp32s3Spi::MS_DLEN,
            AccessWidth::Word,
            15,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            BASE + Esp32s3Spi::USER,
            AccessWidth::Word,
            u64::from(1_u32 << 28 | 1_u32 << 27 | 1),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            BASE + Esp32s3Spi::CMD,
            AccessWidth::Word,
            1 << 24,
            SimTime::from_ticks(1),
        )
        .unwrap();

    assert_eq!(
        machine
            .bus
            .read(
                BASE + Esp32s3Spi::W0,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x3c_a5_0000
    );
    assert_eq!(
        machine
            .bus
            .read(
                BASE + Esp32s3Spi::DMA_INT_RAW,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1 << 12
    );
    assert!(
        machine
            .signals
            .with_registry(|registry| registry.find("esp32s3.spi3.mosi").is_some())
    );
    assert!(
        machine
            .signals
            .with_registry(|registry| registry.find("esp32s3.spi3.cs0").is_some())
    );
}

#[test]
fn esp32s3_i2s1_mmio_transmits_a_single_data_frame() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    const BASE: u64 = 0x6002_d000;
    machine
        .bus
        .write(
            BASE + Esp32s3I2s::SINGLE_DATA,
            AccessWidth::Word,
            0x1234_5678,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            BASE + Esp32s3I2s::TX_CONF1,
            AccessWidth::Word,
            15 << 13,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            BASE + Esp32s3I2s::TX_CONF,
            AccessWidth::Word,
            1 << 2,
            SimTime::from_ticks(1),
        )
        .unwrap();

    assert_eq!(
        machine
            .bus
            .read(
                BASE + Esp32s3I2s::INT_RAW,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1 << 1
    );
    assert_eq!(
        machine
            .bus
            .read(
                BASE + Esp32s3I2s::STATE,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1
    );
    assert!(
        machine
            .signals
            .with_registry(|registry| registry.find("esp32s3.i2s1.bclk").is_some())
    );
    assert!(
        machine
            .signals
            .with_registry(|registry| registry.find("esp32s3.i2s1.dout").is_some())
    );
}

#[test]
fn esp32s3_rmt_native_mmio_emits_a_named_channel_waveform() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6001_6000;
    let item = 2_u64 | (1 << 15) | (3 << 16);
    machine
        .bus
        .write(base, AccessWidth::Word, item, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x20, AccessWidth::Word, 1 | (1 << 6), SimTime::ZERO)
        .unwrap();
    let raw = machine
        .bus
        .read(
            base + 0x70,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(raw & 1, 1);
    assert!(
        machine
            .signals
            .with_registry(|registry| registry.find("board.esp32s3.rmt.ch0").is_some())
    );
    assert!(
        machine
            .signals
            .drain_changes()
            .iter()
            .any(|change| change.at == SimTime::from_ticks(2))
    );
}

#[test]
fn esp32s3_peripheral_inventory_is_mapped_at_native_addresses() {
    let machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let regions = machine
        .bus
        .region_map()
        .into_iter()
        .map(|(name, start, end, _)| (name.to_owned(), (start, end - start)))
        .collect::<std::collections::BTreeMap<_, _>>();

    let expected = [
        ("esp32s3.uart1", 0x6001_0000, 0x1000),
        ("esp32s3.uart2", 0x6002_e000, 0x1000),
        ("esp32s3.i2c0", 0x6001_3000, 0x1000),
        ("esp32s3.i2c1", 0x6002_7000, 0x1000),
        ("esp32s3.spi2", 0x6002_4000, 0x1000),
        ("esp32s3.spi3", 0x6002_5000, 0x1000),
        ("esp32s3.i2s0", 0x6000_f000, 0x1000),
        ("esp32s3.i2s1", 0x6002_d000, 0x1000),
        ("esp32s3.rmt", 0x6001_6000, 0x1000),
        ("esp32s3.ledc", 0x6001_9000, 0x1000),
        ("esp32s3.pcnt", 0x6001_7000, 0x1000),
        ("esp32s3.mcpwm0", 0x6001_e000, 0x1000),
        ("esp32s3.mcpwm1", 0x6002_c000, 0x1000),
        ("esp32s3.twai", 0x6002_b000, 0x1000),
        ("esp32s3.gdma", 0x6003_f000, 0x1000),
        ("esp32s3.uhci0", 0x6001_4000, 0x1000),
        ("esp32s3.io-mux", 0x6000_9000, 0x1000),
        ("esp32s3.saradc", 0x6004_0000, 0x1000),
        ("esp32s3.tsens", 0x6000_8800, 0x0200),
        ("esp32s3.rtc-i2c", 0x6000_8c00, 0x0400),
        ("esp32s3.lcd-cam", 0x6004_1000, 0x1000),
        ("esp32s3.sdmmc", 0x6002_8000, 0x1000),
        ("esp32s3.sha", 0x6003_b000, 0x1000),
        ("esp32s3.aes", 0x6003_a000, 0x1000),
        ("esp32s3.efuse", 0x6000_7000, 0x1000),
        ("esp32s3.hmac", 0x6003_e000, 0x1000),
        ("esp32s3.rsa", 0x6003_c000, 0x1000),
        ("esp32s3.digital-signature", 0x6003_d000, 0x1000),
        ("esp32s3.rtc-control", 0x6000_8000, 0x0400),
        ("esp32s3.rtc-io", 0x6000_8400, 0x0400),
        ("esp32s3.sdm", 0x6000_4f00, 0x0100),
        ("esp32s3.uhci1", 0x6000_c000, 0x1000),
        ("esp32s3.peri-backup", 0x6002_a000, 0x1000),
        ("esp32s3.assist-debug", 0x600c_e000, 0x1000),
        ("esp32s3.interrupt-matrix", 0x600c_2000, 0x1000),
        ("esp32s3.extmem", 0x600c_4000, 0x1000),
        ("esp32s3.xts-aes", 0x600c_c000, 0x1000),
        ("esp32s3.usb-serial-jtag", 0x6003_8000, 0x1000),
        ("esp32s3.usb-otg", 0x6008_0000, 0x1_0000),
    ];

    for (name, start, size) in expected {
        assert_eq!(
            regions.get(name),
            Some(&(start, size)),
            "missing or incorrect native mapping for {name}"
        );
    }
}

#[test]
fn esp32s3_rtc_i2c_routes_completed_transfers_through_rtc_core_interrupt() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6000_8c00;
    machine.rtc_i2c.set_pointer(0x42, 3);
    machine
        .bus
        .write(0x600c_2000 + 39 * 4, AccessWidth::Word, 5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x10, AccessWidth::Word, 0x42, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x34, AccessWidth::Word, 0x5a00, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x30, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x04, AccessWidth::Word, 0xa000_000c, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.rtc_i2c.slave_register(0x42, 3), 0x5a);
    assert!(machine.update_rtc_interrupt_lines().unwrap());
    assert_ne!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
    machine
        .bus
        .write(base + 0x24, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    assert!(!machine.update_rtc_interrupt_lines().unwrap());
    assert_eq!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
}

#[test]
fn esp32s3_sens_touch_scan_routes_ulp_interrupt_and_exposes_pad_status() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x6000_8800;
    machine.tsens.set_touch_raw(3, 100);
    machine
        .bus
        .write(0x600c_2000 + 39 * 4, AccessWidth::Word, 5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x06c, AccessWidth::Word, 200, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            base + 0x0ec,
            AccessWidth::Word,
            (1 << 11) | 5,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(base + 0x05c, AccessWidth::Word, 0x7fff, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x0ac,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        (1 << 29) | 100
    );
    assert!(machine.update_rtc_interrupt_lines().unwrap());
    assert_ne!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
}

#[test]
fn esp32s3_xts_aes_obeys_system_gate_and_releases_ciphertext_to_spi_side() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let base = 0x600c_c000;
    machine
        .bus
        .write(0x600c_004c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    for index in 0..4_u64 {
        machine
            .bus
            .write(
                base + index * 4,
                AccessWidth::Word,
                0x0302_0100 + index * 0x0404_0404,
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(base + 0x4c, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x58,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        2
    );
    assert!(machine.xts_aes.released_ciphertext().is_none());
    machine
        .bus
        .write(base + 0x50, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.xts_aes.released_ciphertext().unwrap().len(), 16);
}

#[test]
fn esp32s3_rtc_slow_memory_aliases_and_reserved_legacy_pages_fault() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine
        .bus
        .write(0x5000_0120, AccessWidth::Word, 0x5254_4353, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x6002_1120,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x5254_4353
    );
    machine
        .bus
        .write(0x6002_1124, AccessWidth::Word, 0x414c_4941, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x5000_0124,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x414c_4941
    );

    // These remaining legacy reg_base constants fall in ranges explicitly
    // marked Reserved by ESP32-S3 TRM table 4.3-3. The FE/FE2, PHY, BT,
    // AGC, NRX, and BB pages are exercised separately as radio-owned pages.
    for address in [0x6000_b000, 0x6001_5000, 0x6001_8000] {
        assert!(
            machine
                .bus
                .read(address, AccessWidth::Word, AccessKind::Read, SimTime::ZERO,)
                .is_err(),
            "reserved ESP32-S3 address {address:#x} unexpectedly responded"
        );
    }

    // The revision-zero mask ROM's rom_agc_reg_init routine accesses this
    // private page immediately before the public NRX base.
    machine
        .bus
        .write(0x6001_c13c, AccessWidth::Word, 0x0130_0000, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x6001_c13c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x0130_0000
    );
}

#[test]
fn esp32s3_extmem_couples_cached_accesses_and_native_interrupts() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let extmem = 0x600c_4000;
    machine.extmem.configure_boot_caches();

    // Preload completion uses native interrupt-matrix source 61.
    machine
        .bus
        .write(0x600c_2000 + 61 * 4, AccessWidth::Word, 5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(extmem + 0x140, AccessWidth::Word, 1 << 4, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            extmem + 0x044,
            AccessWidth::Word,
            0x3c00_0000,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(extmem + 0x048, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(extmem + 0x040, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert!(machine.update_extmem_interrupt_lines().unwrap());
    assert_ne!(machine.cpu.interrupt_state().1 & (1 << 5), 0);

    {
        let mut guarded = pms::Esp32S3PmsBus::new(
            &mut machine.bus,
            &machine.pms,
            &machine.world_controller,
            &machine.extmem,
            &machine.assist_debug,
            0,
            0,
            0,
        );
        assert_eq!(
            guarded
                .read(
                    0x3c00_0000,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0xffff_ffff
        );
    }
    assert_eq!(
        machine
            .bus
            .read(
                extmem + 0x0d8,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1
    );
    machine
        .bus
        .write(extmem + 0x140, AccessWidth::Word, 1 << 5, SimTime::ZERO)
        .unwrap();
    assert!(!machine.update_extmem_interrupt_lines().unwrap());
    assert_eq!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
}

#[test]
fn esp32s3_syscon_routes_external_memory_rejections_through_source_60() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let syscon = 0x6002_6000;
    assert_eq!(
        machine
            .bus
            .read(
                syscon + 0x3fc,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x0210_1150
    );
    machine
        .bus
        .write(0x600c_2000 + 60 * 4, AccessWidth::Word, 5, SimTime::ZERO)
        .unwrap();
    machine.syscon.report_external_reject(0x3c01_0000, 0x02);
    assert!(machine.update_syscon_interrupt_lines().unwrap());
    assert_ne!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
    assert_eq!(
        machine
            .bus
            .read(
                syscon + 0x88,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x09
    );
    assert_eq!(
        machine
            .bus
            .read(
                syscon + 0x8c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x3c01_0000
    );
    machine
        .bus
        .write(syscon + 0x88, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    assert!(!machine.update_syscon_interrupt_lines().unwrap());
    assert_eq!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
}

#[test]
fn esp32s3_usb_wrap_controls_the_functional_dwc2_host_link() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let usb_wrap = 0x6003_9000;
    assert_eq!(
        machine
            .bus
            .read(
                usb_wrap + 0x3fc,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x0210_2010
    );
    assert!(machine.usb_wrap.host_link_active());
    machine
        .bus
        .write(
            usb_wrap,
            AccessWidth::Word,
            (1 << 18) | (1 << 2),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(!machine.usb_wrap.host_link_active());
    machine
        .bus
        .write(
            usb_wrap,
            AccessWidth::Word,
            (1 << 18) | (1 << 12) | (1 << 13),
            SimTime::ZERO,
        )
        .unwrap();
    assert!(machine.usb_wrap.host_link_active());
    machine.usb_wrap.drive_test_inputs(true, false, true);
    machine
        .bus
        .write(usb_wrap + 4, AccessWidth::Word, 0x07, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.usb_wrap.test_output(), Some((true, false, true)));
    assert_eq!(
        machine
            .bus
            .read(
                usb_wrap + 4,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x37
    );
}

#[test]
fn esp32s3_uhci0_couples_gdma_uart_and_interrupt_matrix() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let uhci = 0x6001_4000;
    let gdma = 0x6003_f000;

    // UHCI0 is the single ESP32-S3 instance and attaches to UART2 here.
    machine
        .bus
        .write(uhci, AccessWidth::Word, 0x06e0 | (1 << 4), SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(gdma + 0xa8, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    for byte in [b'R', 0xc0] {
        machine
            .bus
            .write(
                gdma + 0x7c,
                AccessWidth::Word,
                u64::from((1 << 9) | u32::from(byte)),
                SimTime::ZERO,
            )
            .unwrap();
    }
    assert_eq!(machine.uhci.poll_gdma(&machine.gdma), 2);
    assert_eq!(
        machine.auxiliary_uarts[1].bytes(),
        vec![0xc0, b'R', 0xdb, 0xdc, 0xc0]
    );

    machine
        .bus
        .write(gdma + 0x48, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    assert!(machine.queue_uhci_input(&[0xc0, b'X', 0xdb, 0xdc, 0xc0]));
    assert_eq!(
        machine
            .bus
            .read(
                gdma + 0x1c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        u64::from(b'X')
    );

    // UHCI0 is source 14 in the native interrupt matrix.
    machine
        .bus
        .write(0x600c_2000 + 14 * 4, AccessWidth::Word, 5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(uhci + 0x0c, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(uhci + 0x14, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert!(machine.update_uhci_interrupt_lines().unwrap());
    assert_ne!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
    machine
        .bus
        .write(uhci + 0x10, AccessWidth::Word, 1 << 7, SimTime::ZERO)
        .unwrap();
    assert!(!machine.update_uhci_interrupt_lines().unwrap());
    assert_eq!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
}

#[test]
fn esp32s3_io_mux_native_writes_are_visible_to_pin_coupling() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let gpio2 = 0x6000_9000 + 0x0c;
    let input_pullup_gpio_function = (1 << 8) | (1 << 9) | (1 << 12);
    machine
        .bus
        .write(
            gpio2,
            AccessWidth::Word,
            input_pullup_gpio_function,
            SimTime::ZERO,
        )
        .unwrap();

    let config = machine.io_mux().pin_config(2).unwrap();
    assert!(config.pullup);
    assert!(config.input_enable);
    assert_eq!(config.function, 1);
}

#[test]
fn esp32s3_pms_blocks_cpu_writes_and_routes_first_fault_interrupts() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let pms_base = 0x600c_1000;
    let uart0_permissions = pms_base + 0x128;

    // Leave UART0 reads enabled while denying secure-world writes.
    let reset_permissions = machine
        .bus
        .read(
            uart0_permissions,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            uart0_permissions,
            AccessWidth::Word,
            reset_permissions & !1,
            SimTime::ZERO,
        )
        .unwrap();

    // CORE0_PIF_PMS_INTR is native interrupt-matrix source 87.
    machine
        .bus
        .write(0x600c_2000 + 87 * 4, AccessWidth::Word, 5, SimTime::ZERO)
        .unwrap();
    assert!(machine.chip_uart.bytes().is_empty());
    {
        let mut guarded = pms::Esp32S3PmsBus::new(
            &mut machine.bus,
            &machine.pms,
            &machine.world_controller,
            &machine.extmem,
            &machine.assist_debug,
            0,
            0,
            0,
        );
        guarded
            .write(0x6000_0000, AccessWidth::Word, b'X'.into(), SimTime::ZERO)
            .unwrap();
    }
    assert!(machine.chip_uart.bytes().is_empty());

    assert!(machine.update_pms_interrupt_lines().unwrap());
    assert_ne!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
    assert_eq!(
        machine
            .bus
            .read(
                pms_base + 0x1a8,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x6000_0000
    );
    assert_eq!(
        machine
            .bus
            .read(
                pms_base + 0x1a4,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x6b
    );

    machine
        .bus
        .write(pms_base + 0x1a0, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    assert!(!machine.update_pms_interrupt_lines().unwrap());
    assert_eq!(machine.cpu.interrupt_state().1 & (1 << 5), 0);
}

#[test]
fn esp32s3_world_controller_switches_pms_worlds_and_masks_nmi() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    let wcl = 0x600d_0000;
    let pms = 0x600c_1000;
    let trigger = 0x4037_0000;
    let secure_entry = 0x4037_0004;
    let message = 0x3fc8_8000;

    for (offset, value) in [
        (0x000, secure_entry),
        (0x07c, 2),
        (0x100, message),
        (0x104, 3),
        (0x140, trigger),
        (0x144, 2),
        (0x148, 0),
    ] {
        machine
            .bus
            .write(wcl + offset, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    let world1_uart = machine
        .bus
        .read(
            pms + 0x138,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            pms + 0x138,
            AccessWidth::Word,
            world1_uart & !1,
            SimTime::ZERO,
        )
        .unwrap();

    {
        let mut guarded = pms::Esp32S3PmsBus::new(
            &mut machine.bus,
            &machine.pms,
            &machine.world_controller,
            &machine.extmem,
            &machine.assist_debug,
            0,
            0,
            0,
        );
        guarded
            .read(
                trigger,
                AccessWidth::Word,
                AccessKind::Execute,
                SimTime::ZERO,
            )
            .unwrap();
        guarded
            .write(0x6000_0000, AccessWidth::Word, b'X'.into(), SimTime::ZERO)
            .unwrap();
        for value in 0..=3 {
            guarded
                .write(message, AccessWidth::Word, value, SimTime::ZERO)
                .unwrap();
        }
        guarded
            .read(
                secure_entry,
                AccessWidth::Word,
                AccessKind::Execute,
                SimTime::ZERO,
            )
            .unwrap();
    }
    assert!(machine.chip_uart.bytes().is_empty());
    assert_eq!(
        machine
            .world_controller
            .world_for_access(0, AccessKind::Read),
        remu_devices::Esp32S3World::Secure
    );
    assert_eq!(
        machine
            .world_controller
            .world_for_access(0, AccessKind::Execute),
        remu_devices::Esp32S3World::Secure
    );
    assert_eq!(
        machine
            .bus
            .read(
                wcl + 0x080,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x21
    );

    machine
        .bus
        .write(0x600c_2000 + 87 * 4, AccessWidth::Word, 14, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(wcl + 0x190, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    let world0_uart = machine
        .bus
        .read(
            pms + 0x128,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            pms + 0x128,
            AccessWidth::Word,
            world0_uart & !1,
            SimTime::ZERO,
        )
        .unwrap();
    {
        let mut guarded = pms::Esp32S3PmsBus::new(
            &mut machine.bus,
            &machine.pms,
            &machine.world_controller,
            &machine.extmem,
            &machine.assist_debug,
            0,
            0,
            0,
        );
        guarded
            .write(0x6000_0000, AccessWidth::Word, b'Y'.into(), SimTime::ZERO)
            .unwrap();
    }
    assert!(machine.update_pms_interrupt_lines().unwrap());
    assert_eq!(machine.cpu.interrupt_state().1 & (1 << 14), 0);
    machine
        .bus
        .write(wcl + 0x190, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    assert!(machine.update_pms_interrupt_lines().unwrap());
    assert_ne!(machine.cpu.interrupt_state().1 & (1 << 14), 0);
}
