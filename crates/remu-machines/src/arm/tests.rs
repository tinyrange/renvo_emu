use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn rp2040_maps_functional_pio1() {
    let mut machine = ArmMachine::new(TargetId::Rp2040).unwrap();
    let _ = machine.signals.drain_changes();
    machine
        .bus
        .write(0x5030_00dc, AccessWidth::Word, 1_u64 << 26, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x5030_0048, AccessWidth::Word, 0xe001, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x5030_0000, AccessWidth::Word, 1, SimTime::ZERO)
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
    // The RP2040 constructor installs PIO1 before the common PIO0 block.
    assert!(machine.pio[0].poll(SimTime::from_ticks(1)).unwrap());
    let changes = machine.signals.drain_changes();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].value.bit(0), Some(Logic::One));
}
