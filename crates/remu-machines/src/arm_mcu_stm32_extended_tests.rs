use super::*;
use remu_image::FirmwareSegment;

#[test]
fn internal_flash_load_alias_and_programming_path_are_coherent() {
    let image = FirmwareImage {
        architecture: FirmwareArchitecture::Arm,
        entry: u64::from(remu_devices::STM32_FLASH_BASE + 8),
        segments: vec![FirmwareSegment {
            address: u64::from(remu_devices::STM32_FLASH_BASE),
            load_address: None,
            initialized_size: 8,
            data: vec![0, 0, 1, 0x20, 9, 0, 0, 0x08],
            executable: true,
            writable: false,
            alignment: 8,
        }],
        symbols: Vec::new(),
    };
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine.load_firmware(&image).unwrap();
    assert_eq!(
        machine.debug_read_memory(0, 8).unwrap(),
        [0, 0, 1, 0x20, 9, 0, 0, 0x08]
    );

    for key in [0x4567_0123, 0xcdef_89ab] {
        machine
            .bus
            .write(0x4002_2008, AccessWidth::Word, key, SimTime::ZERO)
            .unwrap();
    }
    machine
        .bus
        .write(0x4002_2014, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            u64::from(remu_devices::STM32_FLASH_BASE + 0x10),
            AccessWidth::DoubleWord,
            0x1122_3344_5566_7788,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                u64::from(remu_devices::STM32_FLASH_BASE + 0x10),
                AccessWidth::DoubleWord,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x1122_3344_5566_7788
    );
    assert_eq!(
        machine.debug_read_memory(0x10, 8).unwrap(),
        0x1122_3344_5566_7788_u64.to_le_bytes()
    );
}

#[test]
fn usb_fs_core_and_packet_memory_are_mapped() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let core = 0x4000_6800;
    let pma = 0x4000_6c00;
    machine
        .bus
        .write(core + 0x40, AccessWidth::HalfWord, 1 << 10, SimTime::ZERO)
        .unwrap();
    machine
        .usb_fs
        .as_ref()
        .expect("STM32 USB FS handle")
        .bus_reset(SimTime::from_ticks(1));
    assert_eq!(
        machine.bus.read(
            core + 0x44,
            AccessWidth::HalfWord,
            AccessKind::Read,
            SimTime::ZERO,
        ),
        Ok(1),
    );
    machine
        .bus
        .write(
            pma + 0x120,
            AccessWidth::Byte,
            u64::from(b'R'),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        machine.bus.read(
            pma + 0x120,
            AccessWidth::Byte,
            AccessKind::Read,
            SimTime::ZERO
        ),
        Ok(u64::from(b'R')),
    );
}

#[test]
fn sai1_block_accepts_and_exposes_transmit_samples() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    let base = 0x4001_5400;
    machine
        .bus
        .write(base, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            base + 0x1c,
            AccessWidth::Word,
            0x1234,
            SimTime::from_ticks(1),
        )
        .unwrap();
    let sai = machine.sai1.as_ref().expect("STM32 SAI1 handle");
    assert!(sai.enabled(0));
    assert_eq!(sai.take_tx(0).unwrap(), vec![0x1234]);
}

#[test]
fn quadspi_maps_registers_and_external_flash_window() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine
        .qspi_load_flash(0x100, &[0xde, 0xad, 0xbe, 0xef])
        .unwrap();
    assert_eq!(
        machine.debug_read_memory(0x9000_0100, 4).unwrap(),
        [0xde, 0xad, 0xbe, 0xef],
    );
    for (address, value) in [
        (0xa000_1000, 1),
        (0xa000_1010, 3),
        (0xa000_1018, 0x100),
        (
            0xa000_1014,
            u64::from(0x0b_u32 | (1 << 8) | (1 << 10) | (2 << 12) | (1 << 24) | (3 << 26)),
        ),
    ] {
        machine
            .bus
            .write(address, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    assert_eq!(machine.qspi_flash().unwrap()[0x100], 0xde);
}

#[test]
fn swpmi_maps_registers_and_host_receive_endpoint() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine.inject_swpmi_rx(0x1020_3040, 3).unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x4000_8818,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        3,
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x4000_8820,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0x1020_3040,
    );
    machine
        .bus
        .write(0x4000_8820, AccessWidth::Word, 0, SimTime::ZERO)
        .expect_err("RDR must be read-only");
    machine
        .bus
        .write(0x4000_8800, AccessWidth::Word, 1 << 5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_881c, AccessWidth::Word, 0x5566_7788, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.take_swpmi_tx().unwrap(), vec![0x5566_7788]);
}

#[test]
fn dma1_copies_memory_and_sets_completion_flags() {
    const ENABLE: u32 = 1;
    const TCIE: u32 = 1 << 1;
    const HTIE: u32 = 1 << 2;
    const PINC: u32 = 1 << 6;
    const MINC: u32 = 1 << 7;
    const MEM2MEM: u32 = 1 << 14;
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine
        .debug_write_memory(0x2000_0000, &0x1234_5678_u32.to_le_bytes())
        .unwrap();
    machine
        .debug_write_memory(0x2000_0004, &0x9abc_def0_u32.to_le_bytes())
        .unwrap();
    for (address, value) in [
        (0x4002_0010, 0x2000_0000),
        (0x4002_0014, 0x2000_0008),
        (0x4002_000c, 2),
        (0x4002_00a8, 5),
        (
            0x4002_0008,
            ENABLE | TCIE | HTIE | PINC | MINC | MEM2MEM | (2 << 8) | (2 << 10),
        ),
    ] {
        machine
            .bus
            .write(address, AccessWidth::Word, u64::from(value), SimTime::ZERO)
            .unwrap();
    }
    assert_eq!(machine.service_stm32_dma().unwrap(), 1);
    assert_eq!(machine.service_stm32_dma().unwrap(), 1);
    let mut expected = Vec::new();
    expected.extend_from_slice(&0x1234_5678_u32.to_le_bytes());
    expected.extend_from_slice(&0x9abc_def0_u32.to_le_bytes());
    assert_eq!(machine.debug_read_memory(0x2000_0008, 8).unwrap(), expected);
    let flags = machine
        .bus
        .read(
            0x4002_0000,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(flags & 0x7, 0x7);
}

#[test]
fn touch_count_is_host_configurable_and_interruptible() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine.set_stm32_tsc_group_count(0, 0x321).unwrap();
    for (address, value) in [
        (0x4002_4030, 1),
        (0x4002_4004, 1),
        (0x4002_4000, 1 | 2 | (3 << 5)),
    ] {
        machine
            .bus
            .write(address, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }
    assert_eq!(
        machine
            .bus
            .read(
                0x4002_4034,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0x321,
    );
    assert!(machine.tsc.as_ref().unwrap().interrupt_pending());
}

#[test]
fn comparator_and_opamp_host_inputs_are_observable() {
    let mut machine = ArmMcuMachine::new(TargetId::Stm32l432kc).unwrap();
    machine.set_stm32_comparator_inputs(0, 900, 400).unwrap();
    machine
        .bus
        .write(0x4001_0200, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.comparators.as_ref().unwrap().output(0), Some(true));
    assert!(machine.comparators.as_ref().unwrap().interrupt_pending());
    machine.set_stm32_opamp_inputs(1200, 200).unwrap();
    machine
        .bus
        .write(
            0x4000_7800,
            AccessWidth::Word,
            1 | (3 << 2) | (3 << 4),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(machine.opamp.as_ref().unwrap().output(), 8200);
}
