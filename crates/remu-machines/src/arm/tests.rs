use super::*;

#[test]
fn both_raspberry_pi_arm_profiles_construct() {
    ArmMachine::new(TargetId::Rp2040).unwrap();
    ArmMachine::new(TargetId::Rp2350).unwrap();
}

#[test]
fn raspberry_pi_i2c0_and_i2c1_have_addressed_functional_transfers() {
    for (target, bases) in [
        (TargetId::Rp2040, [0x4004_4000_u64, 0x4004_8000]),
        (TargetId::Rp2350, [0x4009_0000_u64, 0x4009_8000]),
    ] {
        let mut machine = ArmMachine::new(target).unwrap();
        for (index, base) in bases.into_iter().enumerate() {
            assert!(machine.queue_i2c_read(index, 0x58, &[0x12 + index as u8]));
            machine
                .bus
                .write(base + 0x04, AccessWidth::Word, 0x58, SimTime::ZERO)
                .unwrap();
            machine
                .bus
                .write(base + 0x10, AccessWidth::Word, 0xa0, SimTime::ZERO)
                .unwrap();
            machine
                .bus
                .write(
                    base + 0x10,
                    AccessWidth::Word,
                    (1 << 8) | (1 << 9),
                    SimTime::ZERO,
                )
                .unwrap();
            assert_eq!(
                machine
                    .bus
                    .read(
                        base + 0x10,
                        AccessWidth::Word,
                        AccessKind::Read,
                        SimTime::ZERO,
                    )
                    .unwrap(),
                0x12 + index as u64
            );
            assert_eq!(
                machine.i2c_events(index).unwrap(),
                [
                    I2cEvent::Write {
                        address: 0x58,
                        value: 0xa0,
                    },
                    I2cEvent::Read {
                        address: 0x58,
                        value: 0x12 + index as u8,
                    },
                ]
            );
        }
    }
}
