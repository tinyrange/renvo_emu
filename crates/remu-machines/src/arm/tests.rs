use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    let mut rp2040 = ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
    assert_eq!(
        rp2040
            .bus
            .read(
                0x4003_8018,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x90
    );
    rp2040
        .bus
        .write(
            0x4003_8000,
            AccessWidth::Word,
            u64::from(b'Z'),
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(rp2040.chip_uart1.as_ref().unwrap().bytes(), b"Z");
}
