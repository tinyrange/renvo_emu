use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn raspberry_pi_spi0_and_spi1_have_functional_loopback() {
    for (target, bases) in [
        (TargetId::Rp2040, [0x4003_c000_u64, 0x4004_0000]),
        (TargetId::Rp2350, [0x4008_0000_u64, 0x4008_8000]),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        for (index, base) in bases.into_iter().enumerate() {
            let value = 0x30 + index as u64;
            machine
                .bus
                .write(base + 0x08, AccessWidth::Word, value, SimTime::ZERO)
                .unwrap();
            assert_eq!(machine.spi_transmitted(index).unwrap(), [value as u8]);
            assert_eq!(
                machine
                    .bus
                    .read(
                        base + 0x08,
                        AccessWidth::Word,
                        AccessKind::Read,
                        SimTime::ZERO
                    )
                    .unwrap(),
                value
            );
        }
    }
}
