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

fn strict_flash_image() -> EspFlashImage {
    let mut image = merged_flash_image();
    image.bootloader.header.entry = 0x403c_0000;
    image.bootloader.header.segment_count = 2;
    image.bootloader.segments = vec![
        EspImageSegment {
            address: 0x3fce_0000,
            flash_offset: 0x20,
            data: vec![0x10, 0x20, 0x30, 0x40],
        },
        EspImageSegment {
            address: 0x403c_0000,
            flash_offset: 0x1000,
            data: vec![0x36, 0x81, 0x01, 0x3d, 0xf0],
        },
    ];
    image.bootloader.end_offset = 0x1005;
    image
}

fn flash_for(image: &EspFlashImage) -> Vec<u8> {
    let mut flash = vec![0xff; 0x20_000];
    for executable in [&image.bootloader, &image.application] {
        for segment in &executable.segments {
            let start = usize::try_from(segment.flash_offset).unwrap();
            flash[start..start + segment.data.len()].copy_from_slice(&segment.data);
        }
    }
    flash
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
fn native_boot_keeps_the_cache_configuration_rom_contract_observable() {
    let mut machine = XtensaMachine::new(TargetId::Esp32s3).unwrap();
    machine.boot_rom_loaded = true;
    machine.cpu.set_direct_state(0x3fc8_0100, 0x4000_1a1c);
    machine.cpu.set_register(XtensaRegister::A2, 0x4000);
    machine.cpu.set_register(XtensaRegister::A3, 8);
    machine.cpu.set_register(XtensaRegister::A4, 32);

    assert!(machine.service_functional_rom().unwrap());
    assert!(machine.instruction_cache_configured);
    assert_eq!(machine.cpu.register(XtensaRegister::A2), 0);
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
