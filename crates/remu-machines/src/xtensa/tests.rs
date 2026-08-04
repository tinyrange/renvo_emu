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
        ("esp32s3.io-mux", 0x6000_9000, 0x1000),
        ("esp32s3.saradc", 0x6004_0000, 0x1000),
        ("esp32s3.tsens", 0x6000_8800, 0x0200),
        ("esp32s3.lcd-cam", 0x6004_1000, 0x1000),
        ("esp32s3.sdmmc", 0x6002_8000, 0x1000),
        ("esp32s3.sha", 0x6003_b000, 0x1000),
        ("esp32s3.aes", 0x6003_a000, 0x1000),
        ("esp32s3.efuse", 0x6000_7000, 0x1000),
        ("esp32s3.hmac", 0x6003_e000, 0x1000),
        ("esp32s3.rsa", 0x6003_c000, 0x1000),
        ("esp32s3.digital-signature", 0x6003_d000, 0x1000),
        ("esp32s3.rtc-control", 0x6000_8000, 0x0800),
        ("esp32s3.interrupt-matrix", 0x600c_2000, 0x1000),
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
