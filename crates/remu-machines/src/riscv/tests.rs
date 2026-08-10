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
    ] {
        assert_eq!(regions.get(name), Some(&(start, size)), "{name}");
    }
}

#[test]
fn esp32c6_radio_frontend_exposes_clock_split_and_ieee802154_events() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_9804,
            AccessWidth::Word,
            (1 << 23) | (1 << 24),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_3060, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x41, SimTime::ZERO)
        .unwrap();
    let handles = machine.esp32c6_peripherals.as_ref().unwrap();
    assert!(handles.modem.ieee802154_ready());
    assert_eq!(
        handles.ieee802154.take_command(),
        Some(remu_devices::EspIeee802154Command::TxStart)
    );
    handles.ieee802154.complete_tx();
    assert!(handles.ieee802154.interrupt_pending());
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_3064,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap()
            & 1,
        1
    );
}

#[test]
fn esp32c6_ieee802154_dma_transmit_and_explicit_host_receive_use_shared_medium() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let tx_address = 0x4080_0100_u32;
    let rx_address = 0x4080_0200_u32;
    machine
        .bus
        .write(
            0x600a_9804,
            AccessWidth::Word,
            (1 << 23) | (1 << 24),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_3048, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600a_3054, AccessWidth::Word, 0xb5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_30d0,
            AccessWidth::Word,
            u64::from(tx_address),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            0x600a_30e0,
            AccessWidth::Word,
            u64::from(rx_address),
            SimTime::ZERO,
        )
        .unwrap();
    for (offset, byte) in [3_u8, 0x61, 0x88, 0x01].into_iter().enumerate() {
        machine
            .bus
            .write(
                u64::from(tx_address) + offset as u64,
                AccessWidth::Byte,
                u64::from(byte),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x41, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);
    machine.now = SimTime::from_ticks(96);
    assert_eq!(machine.service_radio().unwrap(), 1);
    let replay = machine.radio_replay_artifact().unwrap();
    assert!(replay.events.iter().any(|event| matches!(
        event,
        remu_radio::MediumEvent::Submitted { request, .. }
            if request.frame.bytes == [0x61, 0x88, 0x01]
                && request.frame.origin == remu_radio::FrameOrigin::Emulated
    )));

    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x42, machine.now)
        .unwrap();
    machine
        .bus
        .write(0x600a_3004, AccessWidth::Word, 1 << 7, machine.now)
        .unwrap();
    assert_eq!(machine.service_radio().unwrap(), 1);
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Ieee802154,
            remu_radio::Spectrum::new(2_405_000, 2_000),
            "ieee802154-oqpsk-250k",
            remu_radio::Ieee802154Mac::with_fcs(vec![0x01, 0x00, 0x02, 0xaa]),
            0,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(288);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine.debug_read_memory(u64::from(rx_address), 7).unwrap(),
        [6, 0x01, 0x00, 0x02, 0xaa, (-40_i8) as u8, 191]
    );
}

#[test]
fn esp32c6_ieee802154_ack_request_enters_native_rx_ack_and_completes_matching_sequence() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let tx_address = 0x4080_0280_u32;
    let rx_address = 0x4080_02c0_u32;
    machine
        .bus
        .write(
            0x600a_9804,
            AccessWidth::Word,
            (1 << 23) | (1 << 24),
            SimTime::ZERO,
        )
        .unwrap();
    for (address, value) in [
        (0x600a_3048, 3_u64),
        (0x600a_3004, 1 << 3),
        (0x600a_30d0, u64::from(tx_address)),
        (0x600a_30e0, u64::from(rx_address)),
    ] {
        machine
            .bus
            .write(address, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    for (offset, byte) in [4_u8, 0x21, 0x00, 0x2a, 0xa5].into_iter().enumerate() {
        machine
            .bus
            .write(
                u64::from(tx_address) + offset as u64,
                AccessWidth::Byte,
                u64::from(byte),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x41, SimTime::ZERO)
        .unwrap();
    machine.service_radio().unwrap();
    machine.now = SimTime::from_ticks(128);
    machine.service_radio().unwrap();
    assert_eq!(
        machine
            .esp32c6_peripherals
            .as_ref()
            .unwrap()
            .ieee802154
            .awaiting_ack_sequence(),
        Some(0x2a)
    );

    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Ieee802154,
            remu_radio::Spectrum::new(2_405_000, 2_000),
            "ieee802154-oqpsk-250k",
            remu_radio::Ieee802154Mac::with_fcs(vec![0x02, 0x00, 0x2a]),
            0,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(288);
    machine.service_radio().unwrap();
    assert_eq!(
        machine
            .esp32c6_peripherals
            .as_ref()
            .unwrap()
            .ieee802154
            .awaiting_ack_sequence(),
        None
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_3064,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & (1 << 3),
        1 << 3
    );
    assert_eq!(
        machine.debug_read_memory(u64::from(rx_address), 6).unwrap(),
        [5, 0x02, 0x00, 0x2a, (-40_i8) as u8, 191]
    );
}

#[test]
fn esp32c6_ieee802154_dma_security_applies_vendor_programmed_ccm_star() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let tx_address = 0x4080_0300_u32;
    machine
        .bus
        .write(
            0x600a_9804,
            AccessWidth::Word,
            (1 << 23) | (1 << 24) | (1 << 27),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_3048, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_30d0,
            AccessWidth::Word,
            u64::from(tx_address),
            SimTime::ZERO,
        )
        .unwrap();

    let fcf = 0x9849_u16;
    let mut frame = Vec::from(fcf.to_le_bytes());
    frame.push(0x2a);
    frame.extend_from_slice(&0x1234_u16.to_le_bytes());
    frame.extend_from_slice(&0x5678_u16.to_le_bytes());
    frame.extend_from_slice(&0x9abc_u16.to_le_bytes());
    frame.push(5);
    frame.extend_from_slice(&7_u32.to_le_bytes());
    let payload_offset = frame.len();
    frame.extend_from_slice(b"secured");
    machine
        .bus
        .write(
            0x600a_3128,
            AccessWidth::Word,
            1 | ((payload_offset as u64 + 1) << 8),
            SimTime::ZERO,
        )
        .unwrap();
    for (word, value) in [
        0x0403_0201_u32,
        0x0807_0605,
        0x1111_1111,
        0x1111_1111,
        0x1111_1111,
        0x1111_1111,
    ]
    .into_iter()
    .enumerate()
    {
        machine
            .bus
            .write(
                0x600a_312c + word as u64 * 4,
                AccessWidth::Word,
                u64::from(value),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(
            u64::from(tx_address),
            AccessWidth::Byte,
            frame.len() as u64,
            SimTime::ZERO,
        )
        .unwrap();
    for (offset, byte) in frame.iter().copied().enumerate() {
        machine
            .bus
            .write(
                u64::from(tx_address) + 1 + offset as u64,
                AccessWidth::Byte,
                u64::from(byte),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x41, SimTime::ZERO)
        .unwrap();
    machine.service_radio().unwrap();

    let replay = machine.radio_replay_artifact().unwrap();
    let protected = replay
        .events
        .iter()
        .find_map(|event| match event {
            remu_radio::MediumEvent::Submitted { request, .. }
                if request.frame.origin == remu_radio::FrameOrigin::Emulated =>
            {
                Some(&request.frame.bytes)
            }
            _ => None,
        })
        .expect("secured frame submitted");
    assert_eq!(&protected[..payload_offset], &frame[..payload_offset]);
    assert_ne!(&protected[payload_offset..payload_offset + 7], b"secured");
    assert_eq!(protected.len(), frame.len() + 4);
}

#[test]
fn esp32c6_ieee802154_security_failures_preserve_vendor_reason_codes() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let tx_address = 0x4080_0380_u32;
    machine
        .bus
        .write(
            0x600a_9804,
            AccessWidth::Word,
            (1 << 23) | (1 << 24) | (1 << 27),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_3048, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_30d0,
            AccessWidth::Word,
            u64::from(tx_address),
            SimTime::ZERO,
        )
        .unwrap();

    let mut assert_failure = |frame: &[u8], hardware_offset: u8, reason: u8, count: u32| {
        machine
            .bus
            .write(
                0x600a_3128,
                AccessWidth::Word,
                1 | (u64::from(hardware_offset) << 8),
                machine.now,
            )
            .unwrap();
        machine
            .bus
            .write(
                u64::from(tx_address),
                AccessWidth::Byte,
                frame.len() as u64,
                machine.now,
            )
            .unwrap();
        for (offset, byte) in frame.iter().copied().enumerate() {
            machine
                .bus
                .write(
                    u64::from(tx_address) + 1 + offset as u64,
                    AccessWidth::Byte,
                    u64::from(byte),
                    machine.now,
                )
                .unwrap();
        }
        machine
            .bus
            .write(0x600a_3000, AccessWidth::Word, 0x41, machine.now)
            .unwrap();
        machine.service_radio().unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    0x600a_3084,
                    AccessWidth::Word,
                    AccessKind::Read,
                    machine.now,
                )
                .unwrap(),
            (19 << 4) | (u64::from(reason) << 16)
        );
        assert_eq!(
            machine
                .bus
                .read(
                    0x600a_3178,
                    AccessWidth::Word,
                    AccessKind::Read,
                    machine.now,
                )
                .unwrap(),
            u64::from(count)
        );
        machine
            .bus
            .write(0x600a_3064, AccessWidth::Word, 1 << 5, machine.now)
            .unwrap();
    };

    // Security enable register set, but FCF security bit clear.
    assert_failure(&[0x01, 0x00, 1, 0xaa], 5, 1, 1);
    // Security level zero is reserved for a hardware-protected transmit.
    assert_failure(&[0x09, 0x00, 1, 0, 1, 0, 0, 0, 0xaa], 9, 2, 2);
    // Reserved address modes fail while parsing the secured MAC header.
    assert_failure(&[0x08, 0x04, 1, 5, 1, 0, 0, 0, 0xaa], 9, 3, 3);
    // A payload offset before the complete auxiliary header is invalid.
    assert_failure(&[0x09, 0x00, 1, 5, 1, 0, 0, 0, 0xaa], 4, 4, 4);
    // C6 transmit security requires the auxiliary frame counter.
    assert_failure(&[0x09, 0x00, 1, 0x25, 0xaa], 5, 5, 5);
}

#[test]
fn esp32c6_ieee802154_cca_reports_busy_and_leaves_csma_retry_to_firmware() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    let tx_address = 0x4080_0400_u32;
    machine
        .bus
        .write(
            0x600a_9804,
            AccessWidth::Word,
            (1 << 23) | (1 << 24),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_3048, AccessWidth::Word, 3, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x600a_3050, AccessWidth::Word, 8, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x600a_3054,
            AccessWidth::Word,
            0xb5 | (1 << 14),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            0x600a_30d0,
            AccessWidth::Word,
            u64::from(tx_address),
            SimTime::ZERO,
        )
        .unwrap();
    for (offset, byte) in [3_u8, 0x01, 0x00, 0x2a].into_iter().enumerate() {
        machine
            .bus
            .write(
                u64::from(tx_address) + offset as u64,
                AccessWidth::Byte,
                u64::from(byte),
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Ieee802154,
            remu_radio::Spectrum::new(2_405_000, 2_000),
            "ieee802154-oqpsk-250k",
            vec![0; 100],
            0,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x43, SimTime::ZERO)
        .unwrap();

    machine.service_radio().unwrap();
    assert_eq!(
        machine.radio_pending_ieee802154_cca,
        Some(SimTime::from_ticks(128))
    );
    machine.now = SimTime::from_ticks(128);
    machine.service_radio().unwrap();
    assert!(machine.radio_pending_ieee802154_cca.is_none());
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_3084,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        25 << 4
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x600a_317c,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap(),
        1
    );
    assert!(
        !machine
            .radio_replay_artifact()
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(
                event,
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.origin == remu_radio::FrameOrigin::Emulated
            ))
    );

    // CSMA policy lives in guest firmware: retry only after the interfering
    // frame has ended, then the same one-shot peripheral command succeeds.
    machine
        .bus
        .write(0x600a_3064, AccessWidth::Word, 1 << 5, machine.now)
        .unwrap();
    machine.now = SimTime::from_ticks(4000);
    machine.service_radio().unwrap();
    machine
        .bus
        .write(0x600a_3000, AccessWidth::Word, 0x43, machine.now)
        .unwrap();
    machine.service_radio().unwrap();
    assert_eq!(
        machine.radio_pending_ieee802154_cca,
        Some(SimTime::from_ticks(4128))
    );
    machine.now = SimTime::from_ticks(4128);
    machine.service_radio().unwrap();
    assert!(
        machine
            .radio_replay_artifact()
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(
                event,
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.origin == remu_radio::FrameOrigin::Emulated
            ))
    );
}

#[test]
fn esp32c6_wifi_and_ble_protocol_engines_follow_modem_clock_gates() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    assert!(matches!(
        machine.wifi_engine(),
        Err(MachineError::RadioNotReady("Wi-Fi"))
    ));
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 9) | (1 << 10) | (1 << 17) | (1 << 18),
            SimTime::ZERO,
        )
        .unwrap();
    let mut wifi_frame = vec![0_u8; 24];
    wifi_frame[4..10].fill(0xff);
    machine
        .wifi_engine()
        .unwrap()
        .start(remu_radio::WifiMode::Station)
        .unwrap();
    machine.wifi_engine().unwrap().queue_tx(wifi_frame).unwrap();
    machine
        .ble_controller()
        .unwrap()
        .process_h4(&[1, 3, 12, 0])
        .unwrap();
    assert_eq!(
        machine.ble_controller().unwrap().take_h4_output(),
        Some(vec![4, 0x0e, 4, 1, 3, 12, 0])
    );
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert!(
        machine
            .radio_replay_artifact()
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(
                event,
                remu_radio::MediumEvent::Submitted { request, .. }
                    if request.frame.protocol == remu_radio::RadioProtocol::Wifi
            ))
    );
}

#[test]
fn esp32c6_illegal_native_wifi_dma_is_a_hard_machine_error() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 9) | (1 << 10) | (1 << 17) | (1 << 18),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(
            0x600a_4d6c,
            AccessWidth::Word,
            (3_u64 << 30) | 2,
            SimTime::ZERO,
        )
        .unwrap();

    let error = machine.service_radio().unwrap_err();
    let MachineError::RadioLegality(error) = error else {
        panic!("expected radio legality error, got {error}");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::DmaAddress);
    assert_eq!(error.subsystem, remu_radio::RadioSubsystem::Wifi);
    assert!(error.to_string().contains("0x40800002"));
}

#[test]
fn esp32c6_native_wifi_rx_dma_writes_metadata_frame_and_completion() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine
        .bus
        .write(
            0x600a_9814,
            AccessWidth::Word,
            (1 << 9) | (1 << 10) | (1 << 17) | (1 << 18),
            SimTime::ZERO,
        )
        .unwrap();
    let descriptor = 0x4080_1000_u32;
    let buffer = 0x4080_1100_u32;
    let capacity = 512_u32;
    let control = (1 << 31) | (capacity << 14) | capacity;
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
            0x600a_4084,
            AccessWidth::Word,
            u64::from(descriptor),
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x600a_4c40, AccessWidth::Word, 1 << 14, SimTime::ZERO)
        .unwrap();
    let frame = vec![0x80, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2, 6];
    machine
        .inject_radio_frame(
            remu_radio::RadioProtocol::Wifi,
            remu_radio::Spectrum::new(2_412_000, 20_000),
            "wifi-ht20",
            frame.clone(),
            -40,
        )
        .unwrap();
    machine.now = SimTime::from_ticks(512);
    assert_eq!(machine.service_radio().unwrap(), 1);
    assert_eq!(
        machine
            .debug_read_memory(u64::from(buffer) + 92, frame.len())
            .unwrap(),
        frame
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
    assert_eq!((completed >> 14) & 0x3fff, 108);
    assert_ne!(
        machine
            .bus
            .read(
                0x600a_4c48,
                AccessWidth::Word,
                AccessKind::Read,
                machine.now,
            )
            .unwrap()
            & (1 << 14),
        0
    );
}

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
fn unsupported_targets_fail_explicitly() {
    assert!(matches!(
        RiscVMachine::new(TargetId::Rp2040),
        Err(MachineError::UnsupportedTarget(TargetId::Rp2040))
    ));
}
