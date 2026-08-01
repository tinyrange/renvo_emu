use super::*;
use remu_devices::{Rp2040SpiRegister, Rp2350SpiRegister};

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn rp2040_dma_copies_a_word_and_reports_completion() {
    let mut machine = ArmMachine::new(TargetId::Rp2040).unwrap();
    machine
        .bus
        .write(0x2000_0000, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
        .unwrap();
    for (offset, value) in [
        (0x00, 0x2000_0000),
        (0x04, 0x2000_0004),
        (0x08, 1),
        (0x0c, 1 | (2 << 2) | (1 << 4) | (1 << 5)),
    ] {
        machine
            .bus
            .write(
                0x5000_0000 + offset,
                AccessWidth::Word,
                value,
                SimTime::ZERO,
            )
            .unwrap();
    }
    machine
        .bus
        .write(0x5000_0404, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .dma
            .service(&mut machine.bus, SimTime::ZERO)
            .unwrap(),
        1
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x2000_0004,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0x1234_5678
    );
    assert_eq!(machine.dma.pending(), 1);
}

#[test]
fn rp2040_watchdog_reset_is_visible_to_the_run_loop() {
    let mut machine = ArmMachine::new(TargetId::Rp2040).unwrap();
    machine
        .bus
        .load(u64::from(machine.flash_base), &[0x00, 0xbf])
        .unwrap();
    machine
        .cpu
        .set_direct_state(machine.default_stack, machine.flash_base | 1)
        .unwrap();
    machine
        .bus
        .write(0x4005_8004, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4005_8000, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    let result = machine
        .run(
            RunLimits {
                instructions: Some(8),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(
        result.reason,
        StopReason::Fault("RP2040 watchdog reset".to_owned())
    );
    assert_eq!(result.stats.instructions, 1);
}

#[test]
fn raspberry_pi_uart1_has_functional_transmit_registers() {
    for (target, address) in [
        (TargetId::Rp2040, 0x4003_8000_u64),
        (TargetId::Rp2350, 0x4007_8000_u64),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        machine
            .bus
            .write(address + 0x30, AccessWidth::Word, 0x301, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(address, AccessWidth::Word, 0x5a, SimTime::ZERO)
            .unwrap();
        assert_eq!(machine.chip_uart1.bytes(), [0x5a]);
        assert_eq!(
            machine
                .bus
                .read(
                    address + 0x18,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x90
        );
    }
}

#[test]
fn raspberry_pi_adc_mapping_uses_the_correct_native_offsets() {
    for (target, base) in [
        (TargetId::Rp2040, 0x4004_c000_u64),
        (TargetId::Rp2350, 0x400a_0000_u64),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        assert!(machine.set_adc_sample(2, 0x5a5));
        machine
            .bus
            .write(
                base,
                AccessWidth::Word,
                u64::from(1_u32 | (1 << 2) | (2 << 12)),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(machine.adc_result(), 0x5a5);
    }
}

#[test]
fn raspberry_pi_pwm_models_compare_outputs() {
    for (target, base) in [
        (TargetId::Rp2040, 0x4005_0000_u64),
        (TargetId::Rp2350, 0x400a_8000_u64),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        machine
            .bus
            .write(base + 0x0c, AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base + 0x10, AccessWidth::Word, 9, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        let global_base = if target == TargetId::Rp2040 {
            0xa0
        } else {
            0xf0
        };
        machine
            .bus
            .write(base + global_base, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(machine.pwm_outputs(0), Some([true, false]));
        assert_eq!(
            machine
                .bus
                .read(
                    base + 0x08,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::from_ticks(5),
                )
                .unwrap(),
            5
        );
        assert_eq!(machine.pwm_outputs(0), Some([false, false]));
    }
}

#[test]
fn raspberry_pi_secondary_pio_blocks_are_functional() {
    for (target, bases) in [
        (TargetId::Rp2040, vec![0x5030_0000_u64]),
        (TargetId::Rp2350, vec![0x5030_0000_u64, 0x5040_0000]),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        assert_eq!(machine.pio.len(), bases.len() + 1);
        for (index, base) in bases.into_iter().enumerate() {
            machine
                .bus
                .write(base, AccessWidth::Word, 1 << (index + 1), SimTime::ZERO)
                .unwrap();
            assert_eq!(
                machine
                    .bus
                    .read(base, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
                    .unwrap(),
                1 << (index + 1)
            );
        }
    }
}

#[test]
fn raspberry_pi_spi0_and_spi1_have_functional_loopback() {
    for (index, base) in [0x4003_c000_u64, 0x4004_0000].into_iter().enumerate() {
        let mut machine = ArmMachine::new(TargetId::Rp2040).unwrap();
        let value = 0x30 + index as u64;
        machine
            .bus
            .write(
                base + Rp2040SpiRegister::SsiEnr.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        machine
            .bus
            .write(
                base + Rp2040SpiRegister::Ser.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        machine
            .bus
            .write(
                base + Rp2040SpiRegister::Data(0).offset(),
                AccessWidth::Word,
                value,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(machine.spi_transmitted(index).unwrap(), [value as u8]);
        assert_eq!(
            machine
                .bus
                .read(
                    base + Rp2040SpiRegister::Data(0).offset(),
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap(),
            value
        );
    }

    for base in [0x4008_0000_u64, 0x4008_8000] {
        let mut machine = ArmMachine::new(TargetId::Rp2350).unwrap();
        let value = 0x30;
        machine
            .bus
            .write(
                base + Rp2350SpiRegister::Cr0.offset(),
                AccessWidth::Word,
                7,
                SimTime::ZERO,
            )
            .unwrap();
        machine
            .bus
            .write(
                base + Rp2350SpiRegister::Cr1.offset(),
                AccessWidth::Word,
                3,
                SimTime::ZERO,
            )
            .unwrap();
        machine
            .bus
            .write(
                base + Rp2350SpiRegister::Dr.offset(),
                AccessWidth::Word,
                value,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    base + Rp2350SpiRegister::Dr.offset(),
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap(),
            value
        );
    }
}

#[test]
fn raspberry_pi_i2c0_and_i2c1_have_addressed_functional_transfers() {
    for (target, bases) in [
        (TargetId::Rp2040, [0x4004_4000_u64, 0x4004_8000]),
        (TargetId::Rp2350, [0x4009_0000_u64, 0x4009_8000]),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        for (index, base) in bases.into_iter().enumerate() {
            assert!(machine.queue_i2c_read(index, 0x58, &[0x12 + index as u8]));
            machine
                .bus
                .write(base + 0x04, AccessWidth::Word, 0x58, SimTime::ZERO)
                .unwrap();
            machine
                .bus
                .write(base + 0x6c, AccessWidth::Word, 1, SimTime::ZERO)
                .unwrap();
            machine
                .bus
                .write(base + 0x10, AccessWidth::Word, 0xa0, SimTime::ZERO)
                .unwrap();
            machine
                .bus
                .write(
                    base + 0x10,
                    AccessWidth::Word,
                    (1 << 8) | (1 << 9),
                    SimTime::ZERO,
                )
                .unwrap();
            assert_eq!(
                machine
                    .bus
                    .read(
                        base + 0x10,
                        AccessWidth::Word,
                        AccessKind::Read,
                        SimTime::ZERO
                    )
                    .unwrap(),
                0x12 + index as u64
            );
            assert_eq!(
                machine.i2c_events(index).unwrap(),
                [
                    I2cEvent::Write {
                        address: 0x58,
                        value: 0xa0
                    },
                    I2cEvent::Read {
                        address: 0x58,
                        value: 0x12 + index as u8
                    },
                ]
            );
        }
    }
}

#[test]
fn rp2040_io_bank_reports_and_clears_external_rising_edges() {
    let mut machine = ArmMachine::new(TargetId::Rp2040).unwrap();
    machine
        .bus
        .write(0x4001_4100, AccessWidth::Word, 1 << 3, SimTime::ZERO)
        .unwrap();
    machine.set_pin(0, Logic::One).unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x4001_40f0,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        1 << 3
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x4001_4120,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        1 << 3
    );
    machine
        .bus
        .write(0x4001_40f0, AccessWidth::Word, 1 << 3, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x4001_40f0,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
        0
    );
}
