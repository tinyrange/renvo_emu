use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn rp2040_maps_functional_pio1() {
    let mut machine = ArmMachine::new(TargetId::Rp2040).unwrap();
    machine
        .bus
        .write(0x5030_0048, AccessWidth::Word, 0xe001, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x5030_00e0, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x5030_0048,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0xe001
    );
}
