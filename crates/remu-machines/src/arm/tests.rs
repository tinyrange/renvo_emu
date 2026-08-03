use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn raspberry_pi_arm_adc_maps_native_register_offsets() {
    let mut rp2040 = ArmMachine::new(TargetId::Rp2040).unwrap();
    rp2040
        .bus
        .write(0x4004_c018, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        rp2040
            .bus
            .read(
                0x4004_c020,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );

    let mut rp2350 = ArmMachine::new(TargetId::Rp2350).unwrap();
    rp2350
        .bus
        .write(0x400a_0018, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        rp2350
            .bus
            .read(
                0x400a_0020,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );
}
