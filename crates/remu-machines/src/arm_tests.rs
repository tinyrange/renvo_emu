use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn rp2040_rtc_alarm_reaches_the_machine_interrupt_poll() {
    let mut machine = ArmMachine::new(TargetId::Rp2040).unwrap();
    machine
        .bus
        .load(
            u64::from(machine.flash_base),
            &[0x00, 0xbf, 0x00, 0xbf, 0x00, 0xbf, 0x00, 0xbf],
        )
        .unwrap();
    let mut vectors = vec![0_u8; 0xa8];
    vectors[0..4].copy_from_slice(&machine.default_stack.to_le_bytes());
    vectors[0xa4..0xa8].copy_from_slice(&(machine.flash_base + 0x100 | 1).to_le_bytes());
    machine
        .bus
        .load(u64::from(machine.flash_base), &vectors)
        .unwrap();
    machine
        .bus
        .load(
            u64::from(machine.flash_base + 0x100),
            &[0x00, 0xbf, 0x00, 0xbf],
        )
        .unwrap();
    machine.cpu.set_vector_base(machine.flash_base);
    machine
        .bus
        .load(
            u64::from(machine.flash_base + 0x20),
            &[0x00, 0xbf, 0x00, 0xbf, 0x00, 0xbf],
        )
        .unwrap();
    machine
        .cpu
        .set_direct_state(machine.default_stack, machine.flash_base + 0x20 | 1)
        .unwrap();
    machine
        .cpu
        .set_direct_state(machine.default_stack, machine.flash_base | 1)
        .unwrap();
    machine
        .bus
        .write(
            0x4005_c004,
            AccessWidth::Word,
            (2024 << 12) | (1 << 8) | 1,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x4005_c008, AccessWidth::Word, 0, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4005_c00c, AccessWidth::Word, (1 << 4) | 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x4005_c010,
            AccessWidth::Word,
            (1 << 28) | (1 << 24) | 1,
            SimTime::ZERO,
        )
        .unwrap();
    machine
        .bus
        .write(0x4005_c014, AccessWidth::Word, (1 << 28) | 2, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4005_c024, AccessWidth::Word, 1, SimTime::ZERO)
        .unwrap();

    let result = machine
        .run(
            RunLimits {
                instructions: Some(3),
                deadline: None,
            },
            None,
        )
        .unwrap();
    assert_eq!(result.reason, StopReason::InstructionLimit);
    assert_eq!(result.stats.events, 1);
}
