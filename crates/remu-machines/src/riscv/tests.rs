use super::*;
use remu_image::{EspImageHeader, EspImageSegment, FirmwareSegment};
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
fn esp32c6_direct_elf_leaves_the_bss_tail_poisoned() {
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
        [
            0x13, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5
        ]
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
fn wch_i2c1_is_mapped_for_both_qingke_targets() {
    const I2C1: u64 = 0x4000_5400;
    const PE: u64 = 1;
    const START: u64 = 1 << 8;
    const STOP: u64 = 1 << 9;
    const ADDR: u64 = 1 << 1;
    const RXNE: u64 = 1 << 6;
    for target in [TargetId::Ch32v003, TargetId::Ch32v006] {
        let mut machine = RiscVMachine::new(target).unwrap();
        let i2c = machine.wch_i2c().expect("WCH target exposes I2C1");
        i2c.queue_read(0x50, &[0xde, 0xad]);
        machine
            .bus
            .write(I2C1, AccessWidth::HalfWord, PE | START, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    I2C1 + 0x14,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap()
                & 1,
            1,
            "{target} start flag"
        );
        machine
            .bus
            .write(I2C1 + 0x10, AccessWidth::HalfWord, 0xa1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    I2C1 + 0x14,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap()
                & ADDR,
            ADDR,
            "{target} address acknowledge"
        );
        let _ = machine
            .bus
            .read(
                I2C1 + 0x14,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap();
        let _ = machine
            .bus
            .read(
                I2C1 + 0x18,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap();
        assert_ne!(
            machine
                .bus
                .read(
                    I2C1 + 0x14,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap()
                & RXNE,
            0,
            "{target} receive data ready"
        );
        assert_eq!(
            machine
                .bus
                .read(
                    I2C1 + 0x10,
                    AccessWidth::HalfWord,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap(),
            0xde,
            "{target} first received byte"
        );
        machine
            .bus
            .write(I2C1, AccessWidth::HalfWord, PE | STOP, SimTime::ZERO)
            .unwrap();
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
