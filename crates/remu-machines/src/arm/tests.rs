use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn rp2040_io_bank_reports_and_clears_external_rising_edges() {
    let mut machine = ArmMachine::new(TargetId::Rp2040).unwrap();
    machine
        .bus
        .write(
            0x4001_4000 + 0x100,
            AccessWidth::Word,
            1 << 3,
            SimTime::ZERO,
        )
        .unwrap();
    machine.set_pin(0, Logic::One).unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x4001_4000 + 0x0f0,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1 << 3
    );
    assert_eq!(
        machine
            .bus
            .read(
                0x4001_4000 + 0x120,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        1 << 3
    );
    machine
        .bus
        .write(
            0x4001_4000 + 0x0f0,
            AccessWidth::Word,
            1 << 3,
            SimTime::ZERO,
        )
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                0x4001_4000 + 0x0f0,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0
    );
}
