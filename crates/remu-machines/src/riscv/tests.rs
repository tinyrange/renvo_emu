use super::*;
use remu_image::{EspImageHeader, EspImageSegment, FirmwareSegment, FirmwareSymbol};
use remu_trace::{Timescale, VcdWriter};

fn esp32c6_header(entry: u32, segment_count: u8) -> EspImageHeader {
    EspImageHeader {
        segment_count,
        flash_mode: 2,
        flash_size_frequency: 0x20,
        entry,
        write_protect_pin: 0xee,
        drive_settings: [0; 3],
        chip_id: 13,
        minimum_revision_legacy: 0,
        minimum_revision: 0,
        maximum_revision: 0x63,
        hash_appended: true,
    }
}

fn esp32c6_elf(entry: u64, address: u64, data: Vec<u8>) -> FirmwareImage {
    FirmwareImage {
        architecture: FirmwareArchitecture::RiscV32,
        entry,
        segments: vec![FirmwareSegment {
            address,
            load_address: None,
            initialized_size: data.len(),
            data,
            executable: true,
            writable: false,
            alignment: 0x1000,
        }],
        symbols: Vec::new(),
    }
}

fn app_descriptor() -> Vec<u8> {
    let mut descriptor = vec![0; 256];
    descriptor[..4].copy_from_slice(&0xabcd_5432_u32.to_le_bytes());
    descriptor[180] = 16;
    descriptor
}

fn program_esp32c6_wifi_rf(machine: &mut RiscVMachine, channel: u8, power_qdbm: i16) {
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 9) | (1 << 10),
            machine.now,
        )
        .unwrap();
    let frequency_code = 0x380 + u64::from(channel) * 0x280;
    machine
        .bus
        .write(
            0x600a_00c0,
            AccessWidth::Word,
            0x4284_0000 | (1 << 14) | frequency_code,
            machine.now,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_0474, AccessWidth::Word, 1 << 1, machine.now)
        .unwrap();
    for entry in 0..43_u64 {
        let final_word = if entry == 0 {
            0xfe
        } else if entry == 42 {
            u64::from(((i32::from(power_qdbm) - 133) * 128) as u32)
        } else {
            entry
        };
        for (address, value) in [
            (0x600a_08cc, entry),
            (0x600a_08d0, entry),
            (0x600a_08d4, final_word),
        ] {
            machine
                .bus
                .write(address, AccessWidth::Word, value, machine.now)
                .unwrap();
        }
    }
    machine
        .bus
        .write(0x600a_0910, AccessWidth::Word, 0, machine.now)
        .unwrap();
}

#[test]
fn esp32c6_boot_validator_accepts_separate_descriptor_and_text_mappings() {
    let text = (0_u8..64).collect::<Vec<_>>();
    let elf = esp32c6_elf(0x4200_0100, 0x4200_0100, text.clone());
    let application = EspExecutableImage {
        flash_offset: 0,
        header: esp32c6_header(0x4200_0100, 3),
        segments: vec![
            EspImageSegment {
                address: 0x4201_0020,
                flash_offset: 0x20,
                data: app_descriptor(),
            },
            EspImageSegment {
                address: 0x4200_0100,
                flash_offset: 0x1_0100,
                data: text,
            },
            EspImageSegment {
                address: 0x4080_0000,
                flash_offset: 0x1_0148,
                data: vec![0; 8],
            },
        ],
        checksum: 0xef,
        appended_sha256: None,
        end_offset: 0x1_0160,
    };

    RiscVMachine::validate_esp32c6_boot_image(&elf, &application, 0x1_0000).unwrap();
}

#[test]
fn esp32c6_boot_validator_rejects_the_merged_descriptor_and_text_reproducer() {
    let mut merged = app_descriptor();
    merged.extend(0_u8..64);
    let elf = esp32c6_elf(0x4200_0120, 0x4200_0020, merged.clone());
    let application = EspExecutableImage {
        flash_offset: 0,
        header: esp32c6_header(0x4200_0120, 2),
        segments: vec![
            EspImageSegment {
                address: 0x4200_0020,
                flash_offset: 0x20,
                data: merged,
            },
            EspImageSegment {
                address: 0x4080_0000,
                flash_offset: 0x168,
                data: vec![0; 8],
            },
        ],
        checksum: 0xef,
        appended_sha256: None,
        end_offset: 0x180,
    };

    let error =
        RiscVMachine::validate_esp32c6_boot_image(&elf, &application, 0x1_0000).unwrap_err();
    assert!(error.to_string().contains("exactly two mapped segments"));
}

#[test]
fn rp2350_hazard3_adc_mapping_uses_the_correct_native_offsets() {
    let mut machine = RiscVMachine::new(TargetId::Rp2350).unwrap();
    assert!(machine.set_adc_sample(1, 0x321));
    machine
        .bus
        .write(
            0x400a_0000,
            AccessWidth::Word,
            u64::from(1_u32 | (1 << 2) | (1 << 12)),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(machine.adc_result(), Some(0x321));
}

#[test]
fn rp2350_hazard3_uart1_uses_the_audited_pl011_contract() {
    let mut machine = RiscVMachine::new(TargetId::Rp2350).unwrap();
    let uart1 = 0x4007_8000;
    machine
        .bus
        .write(uart1 + 0x30, AccessWidth::Word, 0x301, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(uart1, AccessWidth::Word, b'Z'.into(), SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.chip_uarts[1].bytes(), [b'Z']);
    assert_eq!(
        machine
            .bus
            .read(
                uart1 + 0xfe0,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x11
    );
}

#[test]
fn esp32c6_direct_elf_materializes_the_zero_fill_tail() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let initialized = [0x13, 0, 0, 0];
    let mut data = initialized.to_vec();
    data.resize(12, 0);
    let image = FirmwareImage {
        architecture: FirmwareArchitecture::RiscV32,
        entry: 0x4080_0000,
        segments: vec![FirmwareSegment {
            address: 0x4080_0000,
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
        machine.debug_read_memory(0x4080_0000, 12).unwrap(),
        [0x13, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn esp32c6_direct_elf_flash_dummy_does_not_overwrite_executable_load() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let image = FirmwareImage {
        architecture: FirmwareArchitecture::RiscV32,
        entry: 0x4200_0020,
        segments: vec![
            FirmwareSegment {
                address: 0x4200_0020,
                load_address: None,
                data: vec![0x13, 0, 0, 0, 0x13, 0, 0, 0],
                initialized_size: 8,
                executable: true,
                writable: false,
                alignment: 4,
            },
            FirmwareSegment {
                address: 0x4200_0020,
                load_address: None,
                data: vec![0; 12],
                initialized_size: 12,
                executable: false,
                writable: true,
                alignment: 4,
            },
        ],
        symbols: Vec::new(),
    };

    machine.load_firmware(&image).unwrap();
    assert_eq!(
        machine.debug_read_memory(0x4200_0020, 12).unwrap(),
        [0x13, 0, 0, 0, 0x13, 0, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn esp32c6_rom_systimer_period_is_visible_to_inlined_isr_reads() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine.cpu.set_pc(0x4000_03d8).unwrap();
    machine.cpu.set_register(RiscVRegister::A1, 0).unwrap();
    machine.cpu.set_register(RiscVRegister::A2, 1_000).unwrap();
    // set_alarm_period() has a 32-bit third argument; A3 must not leak
    // into the host-side period even if it contains unrelated state.
    machine
        .cpu
        .set_register(RiscVRegister::A3, u32::MAX)
        .unwrap();

    assert!(machine.service_functional_bootrom().unwrap());

    assert_eq!(machine.esp_systimer_periods[0], 1_000);
    assert_eq!(
        machine
            .bus
            .read(
                ESP32C6_SYSTIMER_TARGET_CONF,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap()
            & ((1 << 26) - 1),
        1_000
    );
}

#[test]
fn esp32c6_real_rom_bytes_disable_functional_address_dispatch() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let image = FirmwareImage {
        architecture: FirmwareArchitecture::RiscV32,
        entry: 0x4000_0000,
        segments: vec![FirmwareSegment {
            address: 0x4000_0100,
            load_address: None,
            data: vec![0x13, 0, 0, 0],
            initialized_size: 4,
            executable: true,
            writable: false,
            alignment: 4,
        }],
        symbols: vec![FirmwareSymbol {
            name: "a_name_that_must_not_drive_runtime_dispatch".to_owned(),
            address: 0x4000_03d8,
            size: 4,
            code: true,
        }],
    };

    machine.load_boot_rom(&image).unwrap();
    machine.cpu.set_pc(0x4000_03d8).unwrap();

    assert!(!machine.service_functional_bootrom().unwrap());
    assert_eq!(
        machine.debug_read_memory(0x4000_0100, 4).unwrap(),
        [0x13, 0, 0, 0]
    );
}

#[test]
fn esp32c6_real_rom_loader_ignores_elf_unwind_metadata_in_xip_window() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine.bus.load(0x4200_0000, &[0x13, 0, 0, 0]).unwrap();
    let image = FirmwareImage {
        architecture: FirmwareArchitecture::RiscV32,
        entry: 0x4000_0000,
        segments: vec![
            FirmwareSegment {
                address: 0x4087_f100,
                load_address: None,
                data: vec![0x55, 0, 0, 0],
                initialized_size: 1,
                executable: false,
                writable: true,
                alignment: 4,
            },
            FirmwareSegment {
                address: 0x4200_0000,
                load_address: None,
                data: vec![0x10, 0, 0, 0],
                initialized_size: 4,
                executable: false,
                writable: false,
                alignment: 4,
            },
        ],
        symbols: Vec::new(),
    };

    machine.load_boot_rom(&image).unwrap();

    assert_eq!(
        machine.debug_read_memory(0x4200_0000, 4).unwrap(),
        [0x13, 0, 0, 0]
    );
    assert_eq!(
        machine.debug_read_memory(0x4087_f100, 4).unwrap(),
        [0x55, 0, 0, 0]
    );
}

#[test]
fn esp32c6_non_radio_inventory_is_mapped_at_vendor_addresses() {
    let machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let regions = machine
        .bus
        .region_map()
        .into_iter()
        .map(|(name, start, end, _)| (name.to_owned(), (start, end - start)))
        .collect::<std::collections::BTreeMap<_, _>>();
    let expected = [
        ("esp32c6.uart0", 0x6000_0000, 0x1000),
        ("esp32c6.uart1", 0x6000_1000, 0x1000),
        ("esp32c6.i2c0", 0x6000_4000, 0x1000),
        ("esp32c6.uhci0", 0x6000_5000, 0x1000),
        ("esp32c6.rmt", 0x6000_6000, 0x1000),
        ("esp32c6.ledc", 0x6000_7000, 0x1000),
        ("esp32c6.timer-group0", 0x6000_8000, 0x1000),
        ("esp32c6.timer-group1", 0x6000_9000, 0x1000),
        ("esp32c6.systimer", 0x6000_a000, 0x1000),
        ("esp32c6.twai0", 0x6000_b000, 0x1000),
        ("esp32c6.i2s", 0x6000_c000, 0x1000),
        ("esp32c6.twai1", 0x6000_d000, 0x1000),
        ("esp32c6.saradc", 0x6000_e000, 0x1000),
        ("esp32c6.usb-serial-jtag", 0x6000_f000, 0x1000),
        ("esp32c6.interrupt-matrix", 0x6001_0000, 0x800),
        ("esp32c6.pcnt", 0x6001_2000, 0x1000),
        ("esp32c6.etm", 0x6001_3000, 0x1000),
        ("esp32c6.mcpwm", 0x6001_4000, 0x1000),
        ("esp32c6.parlio", 0x6001_5000, 0x1000),
        ("esp32c6.hinf", 0x6001_6000, 0x1000),
        ("esp32c6.slc", 0x6001_7000, 0x1000),
        ("esp32c6.gdma", 0x6008_0000, 0x2b0),
        ("esp32c6.spi2", 0x6008_1000, 0x1000),
        ("esp32c6.aes", 0x6008_8000, 0x1000),
        ("esp32c6.sha", 0x6008_9000, 0x1000),
        ("esp32c6.rsa", 0x6008_a000, 0x1000),
        ("esp32c6.ecc", 0x6008_b000, 0x1000),
        ("esp32c6.digital-signature", 0x6008_c000, 0x1000),
        ("esp32c6.hmac", 0x6008_d000, 0x1000),
        ("esp32c6.io-mux", 0x6009_0000, 0x1000),
        ("esp32c6.gpio", 0x6009_1000, 0x1000),
        ("esp32c6.pcr", 0x6009_6000, 0x1000),
        ("esp32c6.efuse", 0x600b_0800, 0x400),
        ("esp32c6.lp-uart", 0x600b_1400, 0x400),
        ("esp32c6.lp-i2c", 0x600b_1800, 0x400),
        ("esp32c6.lp-watchdog", 0x600b_1c00, 0x400),
        ("esp32c6.interrupt-priority", 0x600c_5000, 0x400),
        ("esp32c6.extmem", 0x600c_8000, 0x1000),
    ];
    for (name, start, size) in expected {
        assert_eq!(regions.get(name), Some(&(start, size)), "{name}");
    }
    for (name, start, size) in [
        ("esp32c6.ieee802154", 0x600a_3000, 0x188),
        ("esp32c6.modem-syscon", 0x600a_9800, 0x800),
        ("esp32c6.modem-lpcon", 0x600a_f000, 0x800),
        ("esp32c6.i2c-ana-mst", 0x600a_f800, 0x100),
        ("esp32c6.phy-i2c-command-memory", 0x600a_fc00, 0x400),
    ] {
        assert_eq!(regions.get(name), Some(&(start, size)), "{name}");
    }
}

include!("tests_radio.rs");
include!("tests_radio_rf_fuzz.rs");
include!("tests_radio_native.rs");
include!("tests_radio_wifi_completion.rs");

#[test]
fn esp32c6_host_bridges_cover_spi_audio_can_etm_parlio_dma_lp_i2c_and_sdio() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let handles = machine.esp32c6_peripherals.as_ref().unwrap();

    handles.spi2.queue_rx(&[9, 8]);
    machine
        .bus
        .write(0x6008_1080, AccessWidth::Word, 0x0201, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x6008_1028, AccessWidth::Word, 15, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x6008_102c, AccessWidth::Word, 15, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x6008_101c,
            AccessWidth::Word,
            (1 << 27) | (1 << 28),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x6008_1000, AccessWidth::Word, 1 << 24, SimTime::ZERO)
        .unwrap();
    assert_eq!(handles.spi2.take_tx(), vec![1, 2]);
    assert_eq!(
        machine
            .bus
            .read(
                0x6008_1080,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap()
            & 0xffff,
        0x0809
    );

    machine
        .bus
        .write(0x6000_c000, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
        .unwrap();
    assert_eq!(handles.i2s.take_tx_words(), vec![0x1234_5678]);
    machine
        .bus
        .write(0x6000_b040, AccessWidth::Word, 0x5a, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x6000_b004, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(handles.twai[0].take_tx_frames()[0][0], 0x5a);

    machine
        .bus
        .write(0x6001_3018, AccessWidth::Word, 7, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x6001_301c, AccessWidth::Word, 9, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x6001_3004, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        handles.etm.trigger(7, SimTime::from_ticks(1)).unwrap(),
        vec![9]
    );
    machine
        .bus
        .write(0x6001_5024, AccessWidth::Word, 0xabcd, SimTime::ZERO)
        .unwrap();
    assert_eq!(handles.parlio.take_tx_words(), vec![0xabcd]);
    machine
        .bus
        .write(
            0x6008_00dc,
            AccessWidth::Word,
            (1 << 9) | 0x155,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(handles.gdma.take_output_words(), vec![0x155]);

    machine
        .bus
        .write(0x600b_181c, AccessWidth::Word, 0xa5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600b_1858, AccessWidth::Word, (1 << 11) | 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600b_1804, AccessWidth::Word, 1 << 5, SimTime::ZERO)
        .unwrap();
    assert_eq!(handles.lp_i2c.take_tx(), vec![0xa5]);
    handles.sdio.queue_rx(0, &[0x123, 0x456]);
    assert_eq!(
        machine
            .bus
            .read(
                0x6001_7024,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap()
            >> 2
            & 0x3fff,
        2
    );
}

#[test]
fn esp32c6_lp_core_wakes_and_executes_from_retained_sram() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let hp_wfi = 0x1050_0073_u32.to_le_bytes();
    machine.bus.load(0x4080_1000, &hp_wfi).unwrap();
    machine.cpu.set_pc(0x4080_1000).unwrap();
    // addi a0,x0,42; lui t0,0x40800; sw a0,0(t0); ebreak
    let lp = [0x02a0_0513_u32, 0x4080_02b7, 0x00a2_a023, 0x0010_0073]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    let image = esp32c6_elf(0x5000_0080, 0x5000_0080, lp);
    machine.load_esp32c6_lp_firmware(&image).unwrap();
    // Hand LP SRAM ownership to the LP core, then trigger it from HP.
    machine
        .bus
        .write(0x600b_1048, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600b_0184, AccessWidth::Word, 1 << 31, SimTime::ZERO)
        .unwrap();

    let result = machine
        .run(
            RunLimits {
                instructions: Some(16),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(result.reason, StopReason::InstructionLimit);
    assert_eq!(
        machine.debug_read_memory(0x4080_0000, 4).unwrap(),
        42_u32.to_le_bytes()
    );
    assert_eq!(machine.cpu1.snapshot().registers[10].value, 42);
}

#[test]
fn esp32c6_cache_sync_refreshes_stale_rom_mmap_data() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine.set_esp_flash_image(&[0x11, 0x22, 0x33, 0x44]);
    machine
        .refresh_esp32c6_cache(ESP_FUNCTIONAL_MMAP_BASE, 4)
        .unwrap();
    machine.esp_flash[0] = 0xaa;
    assert_eq!(
        machine
            .debug_read_memory(u64::from(ESP_FUNCTIONAL_MMAP_BASE), 1)
            .unwrap(),
        [0x11]
    );
    machine
        .bus
        .write(
            0x600c_80a0,
            AccessWidth::Word,
            u64::from(ESP_FUNCTIONAL_MMAP_BASE),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600c_80a4, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600c_8098, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    let (address, size) = machine.esp_c6_extmem.as_ref().unwrap().take_sync().unwrap();
    machine.refresh_esp32c6_cache(address, size).unwrap();
    assert_eq!(
        machine
            .debug_read_memory(u64::from(ESP_FUNCTIONAL_MMAP_BASE), 1)
            .unwrap(),
        [0xaa]
    );
}

#[test]
fn esp32c6_spi1_user_commands_mutate_and_read_the_shared_flash_image() {
    const SPI1: u64 = 0x6000_3000;
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine.set_esp_flash_image(&vec![0xff; 0x20_000]);

    machine
        .bus
        .write(SPI1, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1 + 0x20, AccessWidth::Word, 0x7000_0002, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1 + 0x04, AccessWidth::Word, 0x9120, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1 + 0x24, AccessWidth::Word, 31, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1 + 0x58, AccessWidth::Word, 0x4433_2211, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1, AccessWidth::Word, 1 << 18, SimTime::ZERO)
        .unwrap();
    machine.poll_esp32c6_flash_commands().unwrap();
    assert_eq!(
        &machine.esp_flash[0x9120..0x9124],
        &[0x11, 0x22, 0x33, 0x44]
    );

    machine
        .bus
        .write(SPI1 + 0x20, AccessWidth::Word, 0x7000_00bb, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1 + 0x04, AccessWidth::Word, 0x9120, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1 + 0x28, AccessWidth::Word, 31, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1, AccessWidth::Word, 1 << 18, SimTime::ZERO)
        .unwrap();
    machine.poll_esp32c6_flash_commands().unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                SPI1 + 0x58,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x4433_2211
    );

    machine
        .bus
        .write(SPI1, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1 + 0x20, AccessWidth::Word, 0x7000_0020, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1 + 0x04, AccessWidth::Word, 0x9000, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(SPI1, AccessWidth::Word, 1 << 18, SimTime::ZERO)
        .unwrap();
    machine.poll_esp32c6_flash_commands().unwrap();
    assert!(
        machine.esp_flash[0x9000..0xa000]
            .iter()
            .all(|byte| *byte == 0xff)
    );
}

#[test]
fn esp32c6_indirect_mmu_maps_flash_at_the_hardware_cache_base() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let mut flash = vec![0xff; 0x1_0000];
    flash[0x8000..0x8004].copy_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
    machine.set_esp_flash_image(&flash);
    machine.configure_esp32c6_mmu_page_size(0x8000).unwrap();
    machine
        .bus
        .write(0x6000_2380, AccessWidth::Word, 8, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x6000_237c, AccessWidth::Word, 0x201, SimTime::ZERO)
        .unwrap();
    machine.refresh_esp32c6_mmu_mappings().unwrap();
    assert_eq!(
        machine.debug_read_memory(0x4204_0000, 4).unwrap(),
        [0xde, 0xad, 0xbe, 0xef]
    );
}

#[test]
fn esp32c6_main_watchdog_cpu_reset_reports_vendor_reset_reason() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(0x6000_8064, AccessWidth::Word, 0x50d8_3aa1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x6000_8050, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x6000_8048,
            AccessWidth::Word,
            (1 << 31) | (2 << 29),
            SimTime::ZERO,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(2);
    let mut stats = RunStats {
        instructions: 0,
        time: machine.now,
        events: 0,
    };
    assert!(machine.poll_esp32c6_watchdog(&mut stats).unwrap());
    assert_eq!(machine.esp_reset_reason, 0x0b);
    assert_eq!(machine.cpu.pc(), 0x4000_0000);
}

#[test]
fn all_initial_riscv_modes_execute_and_halt_deterministically() {
    // addi x1,x0,7; addi x2,x0,5; add x3,x1,x2; ebreak
    let program = [0x0070_0093_u32, 0x0050_0113, 0x0020_81b3, 0x0010_0073]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    for target in [
        TargetId::Ch32v003,
        TargetId::Ch32v006,
        TargetId::Esp32c6,
        TargetId::Rp2350,
    ] {
        let entry = target_manifest(target).memory[0].start;
        let mut machine = RiscVMachine::new(target).unwrap();
        machine.load_bytes(entry, &program).unwrap();
        machine
            .set_entry(u32::try_from(entry).expect("initial addresses fit RV32"))
            .unwrap();
        let result = machine
            .run(
                RunLimits {
                    instructions: Some(16),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(result.reason, StopReason::Halted, "{target}");
        assert_eq!(result.cpu.registers[3].value, 12, "{target}");
    }
}

#[test]
fn rp2350_hazard3_pwm_uses_rp2350_globals_and_irq_banks() {
    let mut machine = RiscVMachine::new(TargetId::Rp2350).unwrap();
    let base = 0x400a_8000;
    machine
        .bus
        .write(base + 0x0c, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x10, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0xf0, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.pwm_outputs(0), Some([true, false]));
    assert_eq!(
        machine
            .bus
            .read(
                base + 0xf0,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1
    );
    machine
        .bus
        .write(base + 0xf8, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0xfc, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.pwm_pending_interrupts(0), 1);
    machine
        .bus
        .write(base + 0x104, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x108, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.pwm_pending_interrupts(1), 1);
}

#[test]
fn rp2350_hazard3_maps_all_secondary_pio_blocks() {
    let mut machine = RiscVMachine::new(TargetId::Rp2350).unwrap();
    assert_eq!(machine.pio.len(), 3);
    for (index, base) in [(1, 0x5030_0000_u64), (2, 0x5040_0000_u64)] {
        machine
            .bus
            .write(
                base + 0x048,
                AccessWidth::Word,
                0xe000 | index,
                SimTime::ZERO,
            )
            .unwrap();
        machine
            .bus
            .write(base + 0x0dc, AccessWidth::Word, 1 << 26, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
    }
}

#[test]
fn riscv_bus_trace_correlates_cpu_accesses_but_not_debugger_reads() {
    // lui x1,0x20000; addi x2,x0,7; sw x2,0(x1); ebreak
    let program = [0x2000_00b7_u32, 0x0070_0113, 0x0020_a023, 0x0010_0073]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    let mut machine = RiscVMachine::new(TargetId::Ch32v003).unwrap();
    machine.load_bytes(0, &program).unwrap();
    machine.set_entry(0).unwrap();
    machine.set_access_recording(true);

    let result = machine
        .run(
            RunLimits {
                instructions: Some(16),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(result.reason, StopReason::Halted);
    let cpu_write = machine
        .access_log()
        .iter()
        .find(|record| record.kind == AccessKind::Write && record.address == 0x2000_0000)
        .unwrap();
    assert_eq!(cpu_write.pc, Some(8));

    let prior_records = machine.access_log().len();
    assert_eq!(machine.debug_read_memory(0x2000_0000, 1).unwrap(), [7]);
    assert_eq!(machine.access_log()[prior_records].pc, None);
}

#[test]
fn gpio_facade_streams_valid_vcd() {
    // lui x1,0xffff0; addi x2,x0,1; sw x2,0(x1); sw x2,4(x1); ebreak
    let program = [
        0xffff_00b7_u32,
        0x0010_0113,
        0x0020_a023,
        0x0020_a223,
        0x0010_0073,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect::<Vec<_>>();
    let mut machine = RiscVMachine::new(TargetId::Ch32v003).unwrap();
    machine.load_bytes(0, &program).unwrap();
    machine.set_entry(0).unwrap();
    let mut vcd = VcdWriter::new(Vec::new(), Timescale::Nanosecond);
    let result = machine
        .run(
            RunLimits {
                instructions: Some(16),
                deadline: None,
            },
            Some(&mut vcd),
        )
        .unwrap();
    assert_eq!(result.reason, StopReason::Halted);
    let output = String::from_utf8(vcd.into_inner()).unwrap();
    assert!(output.contains("$enddefinitions $end"));
    assert!(output.contains("#3"));
}

#[test]
fn wch_exti_uses_afio_mapping_and_external_gpio_stimulus() {
    let mut machine = RiscVMachine::new(TargetId::Ch32v003).unwrap();
    // ebreak at address zero keeps the test independent of a firmware image.
    machine
        .load_bytes(0, &0x0010_0073_u32.to_le_bytes())
        .unwrap();
    machine.set_entry(0).unwrap();
    machine
        .bus
        .write(0x4001_0008, AccessWidth::Word, 2 << (2 * 2), SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4001_0400, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4001_0408, AccessWidth::Word, 1 << 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0xe000_e100, AccessWidth::Word, 1 << 20, SimTime::ZERO)
        .unwrap();

    let result = machine
        .run_with_stimuli(
            RunLimits {
                instructions: Some(4),
                deadline: None,
            },
            &[PinStimulus {
                at: SimTime::ZERO,
                pin: 2,
                value: Logic::One,
            }],
            None,
        )
        .unwrap();
    assert_eq!(result.reason, StopReason::Halted);
    assert!(result.stats.events >= 2);
}

#[test]
fn wch_stk_registers_are_mapped_at_the_core_private_base() {
    for target in [TargetId::Ch32v003, TargetId::Ch32v006] {
        let mut machine = RiscVMachine::new(target).unwrap();
        machine
            .bus
            .write(0xe000_f010, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0xe000_f000, AccessWidth::Word, 0x0f, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    0xe000_f000,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x0f,
            "{target} STK control"
        );
        assert_eq!(
            machine
                .bus
                .read(
                    0xe000_f010,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            2,
            "{target} STK compare"
        );
    }
}

#[test]
fn wch_spi1_native_registers_are_mapped_for_both_targets() {
    for target in [TargetId::Ch32v003, TargetId::Ch32v006] {
        let mut machine = RiscVMachine::new(target).unwrap();
        machine.inject_wch_spi_rx(0xa5);
        machine
            .bus
            .write(0x4001_3000, AccessWidth::Word, 0x44, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x4001_3004, AccessWidth::Word, 0xc0, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x4001_300c, AccessWidth::Word, 0x3c, SimTime::ZERO)
            .unwrap();
        assert_eq!(machine.wch_spi_tx_bytes(), [0x3c], "{target}");
        assert_eq!(
            machine
                .bus
                .read(
                    0x4001_300c,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0xa5,
            "{target}"
        );
    }
}

#[test]
fn unsupported_targets_fail_explicitly() {
    assert!(matches!(
        RiscVMachine::new(TargetId::Rp2040),
        Err(MachineError::UnsupportedTarget(TargetId::Rp2040))
    ));
}

#[test]
fn wch_tim1_registers_and_update_interrupt_are_mapped() {
    let mut machine = RiscVMachine::new(TargetId::Ch32v003).unwrap();
    let base = 0x4001_2c00;
    machine
        .bus
        .write(base + 0x28, AccessWidth::HalfWord, 0, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x2c, AccessWidth::HalfWord, 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base + 0x0c, AccessWidth::HalfWord, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(base, AccessWidth::HalfWord, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x2c,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        2
    );
    assert_eq!(
        machine
            .bus
            .read(
                base + 0x10,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::from_ticks(3),
            )
            .unwrap()
            & 1,
        1
    );
}

#[test]
fn wch_tim1_is_mapped_on_ch32v006() {
    let mut machine = RiscVMachine::new(TargetId::Ch32v006).unwrap();
    let base = 0x4001_2c00;
    machine
        .bus
        .write(base, AccessWidth::HalfWord, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(base, AccessWidth::HalfWord, AccessKind::Read, SimTime::ZERO,)
            .unwrap(),
        1
    );
}

#[test]
fn wch_watchdog_blocks_are_mapped_for_both_qingke_targets() {
    for target in [TargetId::Ch32v003, TargetId::Ch32v006] {
        let mut machine = RiscVMachine::new(target).unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    0x4000_3000,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0,
            "{target} IWDG key reads as write-only"
        );
        assert_eq!(
            machine
                .bus
                .read(
                    0x4000_2c00,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap()
                & 0x80,
            0,
            "{target} WWDG starts disabled"
        );
    }
}

#[test]
fn wch_adc_block_is_mapped_for_both_qingke_targets() {
    for target in [TargetId::Ch32v003, TargetId::Ch32v006] {
        let mut machine = RiscVMachine::new(target).unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    0x4001_2400,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0,
            "{target} ADC status resets clear"
        );
    }
}

#[test]
fn wch_independent_watchdog_timeout_resets_the_riscv_machine() {
    let mut machine = RiscVMachine::new(TargetId::Ch32v003).unwrap();
    // `jal x0, 0`: keep the CPU runnable while the abstract IWDG expires.
    machine
        .load_bytes(0, &0x0000_006f_u32.to_le_bytes())
        .unwrap();
    machine.set_entry(0).unwrap();
    machine
        .bus
        .write(0x4000_3000, AccessWidth::Word, 0x5555, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_3008, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_3000, AccessWidth::Word, 0xcccc, SimTime::ZERO)
        .unwrap();

    let result = machine
        .run(
            RunLimits {
                // The CH32 independent watchdog uses a /4 prescaler.  Give the
                // scheduler enough abstract instruction ticks to observe the
                // reload=0 expiry before the bounded run stops.
                instructions: Some(8),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(result.reason, StopReason::InstructionLimit);
    assert!(
        result.stats.events >= 1,
        "watchdog reset was not dispatched"
    );
}

#[test]
fn wch_flash_controller_programs_and_erases_the_mapped_alias() {
    const KEY1: u64 = 0x4567_0123;
    const KEY2: u64 = 0xcdef_89ab;
    const PG: u64 = 1;
    const PAGE_PG: u64 = 1 << 16;
    const BUF_LOAD: u64 = 1 << 18;
    const PER_AND_STRT: u64 = (1 << 1) | (1 << 6);
    for target in [TargetId::Ch32v003, TargetId::Ch32v006] {
        let mut machine = RiscVMachine::new(target).unwrap();
        machine
            .load_bytes(0x400, &[0xff, 0xff, 0xff, 0xff])
            .unwrap();
        machine
            .bus
            .write(0x4002_2004, AccessWidth::Word, KEY1, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x4002_2004, AccessWidth::Word, KEY2, SimTime::ZERO)
            .unwrap();
        if target == TargetId::Ch32v003 {
            machine
                .bus
                .write(0x4002_2010, AccessWidth::Word, PG, SimTime::ZERO)
                .unwrap();
            machine
                .bus
                .write(0x400, AccessWidth::HalfWord, 0x1234, SimTime::ZERO)
                .unwrap();
        } else {
            machine
                .bus
                .write(0x4002_2024, AccessWidth::Word, KEY1, SimTime::ZERO)
                .unwrap();
            machine
                .bus
                .write(0x4002_2024, AccessWidth::Word, KEY2, SimTime::ZERO)
                .unwrap();
            machine
                .bus
                .write(0x4002_2010, AccessWidth::Word, PAGE_PG, SimTime::ZERO)
                .unwrap();
            machine
                .bus
                .write(0x400, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
                .unwrap();
            machine
                .bus
                .write(
                    0x4002_2010,
                    AccessWidth::Word,
                    PAGE_PG | BUF_LOAD,
                    SimTime::ZERO,
                )
                .unwrap();
        }
        assert_eq!(
            machine
                .bus
                .read(
                    0x0800_0400,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            if target == TargetId::Ch32v003 {
                0xffff_1234
            } else {
                0x1234_5678
            }
        );
        machine
            .bus
            .write(0x4002_2014, AccessWidth::Word, 0x400, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(0x4002_2010, AccessWidth::Word, PER_AND_STRT, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    0x400,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0xffff
        );
    }
}
