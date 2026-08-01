use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn rp2350_arm_maps_pio1_and_pio2() {
    let mut machine = ArmMachine::new(TargetId::Rp2350).unwrap();
    for base in [0x5030_0000, 0x5040_0000] {
        machine
            .bus
            .write(base + 0x48, AccessWidth::Word, 0xe001, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    base + 0x48,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0xe001
        );
    }
}
