use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn rp2350_arm_maps_pio1_and_pio2() {
    let mut machine = ArmMachine::new(TargetId::Rp2350).unwrap();
    let _ = machine.signals.drain_changes();
    for (index, base) in [(1, 0x5030_0000), (2, 0x5040_0000)] {
        assert_eq!(
            machine
                .bus
                .read(
                    base + 0x44,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x1020_0404
        );
        machine
            .bus
            .write(base + 0x48, AccessWidth::Word, 0xe001, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base + 0x0dc, AccessWidth::Word, 1_u64 << 26, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base, AccessWidth::Word, 1, SimTime::ZERO)
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
        assert!(machine.pio[index].poll(SimTime::from_ticks(1)).unwrap());
    }
}
