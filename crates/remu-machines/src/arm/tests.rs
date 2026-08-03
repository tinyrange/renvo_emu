use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn raspberry_pi_pwm_models_compare_outputs() {
    for (target, base) in [
        (TargetId::Rp2040, 0x4005_0000_u64),
        (TargetId::Rp2350, 0x400a_8000_u64),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        machine
            .bus
            .write(base + 0x0c, AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base + 0x10, AccessWidth::Word, 9, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        let global_base = if target == TargetId::Rp2040 {
            0xa0
        } else {
            0xf0
        };
        machine
            .bus
            .write(base + global_base, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(machine.pwm_outputs(0), Some([true, false]));
        assert_eq!(
            machine
                .bus
                .read(
                    base + 0x08,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::from_ticks(5),
                )
                .unwrap(),
            5
        );
        assert_eq!(machine.pwm_outputs(0), Some([false, false]));
    }
}
