use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn rp2040_watchdog_reset_is_visible_to_the_run_loop() {
    let mut machine = ArmMachine::new(TargetId::Rp2040).unwrap();
    machine
        .bus
        .load(u64::from(machine.flash_base), &[0x00, 0xbf])
        .unwrap();
    machine
        .cpu
        .set_direct_state(machine.default_stack, machine.flash_base | 1)
        .unwrap();
    machine
        .bus
        .write(0x4005_8004, AccessWidth::Word, 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4005_8000, AccessWidth::Word, 1 << 30, SimTime::ZERO)
        .unwrap();
    let result = machine
        .run(
            RunLimits {
                instructions: Some(8),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(
        result.reason,
        StopReason::Fault("RP2040 watchdog reset".to_owned())
    );
    assert_eq!(result.stats.instructions, 1);
}
