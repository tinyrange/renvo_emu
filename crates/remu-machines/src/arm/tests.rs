use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn raspberry_pi_adc_models_channel_and_temperature_samples() {
    for (target, base) in [
        (TargetId::Rp2040, 0x4004_c000_u64),
        (TargetId::Rp2350, 0x400a_0000_u64),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        assert!(machine.set_adc_sample(4, 0x456));
        machine
            .bus
            .write(base, AccessWidth::Word, (4 << 12) | (1 << 2), SimTime::ZERO)
            .unwrap();
        assert_eq!(machine.adc_conversions(), [0x456]);
        assert_eq!(
            machine
                .bus
                .read(
                    base + 0x04,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO
                )
                .unwrap(),
            0x456
        );
    }
}
