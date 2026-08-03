use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn raspberry_pi_adc_mapping_uses_the_correct_native_offsets() {
    for (target, base) in [
        (TargetId::Rp2040, 0x4004_c000_u64),
        (TargetId::Rp2350, 0x400a_0000_u64),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        assert!(machine.set_adc_sample(2, 0x5a5));
        machine
            .bus
            .write(
                base,
                AccessWidth::Word,
                u64::from(1_u32 | (1 << 2) | (2 << 12)),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(machine.adc_result(), 0x5a5);
    }
}
