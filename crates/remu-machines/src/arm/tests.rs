use super::*;

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
    machine
        .bus
        .write(0x5000_0000, AccessWidth::Word, 0x2000_0000, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x5000_0004, AccessWidth::Word, 0x2000_0004, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x5000_0008, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x5000_000c,
            AccessWidth::Word,
            1 | (2 << 2) | (1 << 4) | (1 << 5),
            SimTime::ZERO,
        )
        .unwrap();
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
                SimTime::ZERO,
            )
            .unwrap(),
        0x1234_5678
    );
    assert_eq!(machine.dma.pending(), 1);
}
