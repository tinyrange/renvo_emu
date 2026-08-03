use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
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
