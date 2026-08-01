use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn raspberry_pi_uart1_has_functional_transmit_registers() {
    for (target, address) in [
        (TargetId::Rp2040, 0x4003_8000_u64),
        (TargetId::Rp2350, 0x4007_8000_u64),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        machine
            .bus
            .write(address, AccessWidth::Word, 0x5a, SimTime::ZERO)
            .unwrap();
        assert_eq!(machine.chip_uart1.bytes(), [0x5a]);
        assert_eq!(
            machine
                .bus
                .read(
                    address + 0x18,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x90
        );
    }
}
