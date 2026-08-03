use super::*;
use remu_devices::{Rp2040SpiRegister, Rp2350SpiRegister};

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
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
                    SimTime::ZERO,
                )
                .unwrap(),
            value
        );
    }

    for (index, base) in [0x4008_0000_u64, 0x4008_8000].into_iter().enumerate() {
        let mut machine = ArmMachine::new(TargetId::Rp2350).unwrap();
        let value = 0x30 + index as u64;
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
                    SimTime::ZERO,
                )
                .unwrap(),
            value
        );
    }
}
