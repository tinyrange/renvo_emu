use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn raspberry_pi_arm_pwm_maps_native_global_registers() {
    let mut rp2040 = ArmMachine::new(TargetId::Rp2040).unwrap();
    rp2040
        .bus
        .write(0x4005_00a0, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        rp2040
            .bus
            .read(
                0x4005_00a0,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1
    );

    let mut rp2350 = ArmMachine::new(TargetId::Rp2350).unwrap();
    rp2350
        .bus
        .write(0x400a_80f0, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        rp2350
            .bus
            .read(
                0x400a_80f0,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1
    );
}
