use super::*;

#[test]
fn maps_secondary_sercom_and_tc_instances_to_vendor_irqs() {
    let mut machine = ArmMcuMachine::new(TargetId::Atsamd21e18).unwrap();

    for (instance, base) in [0x4200_0c00, 0x4200_1000, 0x4200_1400]
        .into_iter()
        .enumerate()
    {
        machine
            .bus
            .write(base, AccessWidth::Word, 3_u64 << 2, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base + 0x04, AccessWidth::Word, 1 << 17, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base, AccessWidth::Word, (3_u64 << 2) | 2, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base + 0x16, AccessWidth::Byte, (1 << 2) | 1, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(
                base + 0x28,
                AccessWidth::Byte,
                0x51 + u64::try_from(instance).unwrap(),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            machine
                .bus
                .read(
                    base + 0x28,
                    AccessWidth::Byte,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            0x51 + u64::try_from(instance).unwrap()
        );
    }
    assert_eq!(
        machine
            .samd_sercom_irqs
            .iter()
            .map(|(line, _)| *line)
            .collect::<Vec<_>>(),
        [10, 11, 12]
    );
    assert!(
        machine
            .samd_sercom_irqs
            .iter()
            .all(|(_, handle)| handle.interrupt_pending())
    );

    for base in [0x4200_3000, 0x4200_3400] {
        machine
            .bus
            .write(base + 0x18, AccessWidth::HalfWord, 4, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base + 0x0d, AccessWidth::Byte, 0x10, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base, AccessWidth::HalfWord, 2, SimTime::ZERO)
            .unwrap();
    }
    assert_eq!(
        machine
            .samd_tc_irqs
            .iter()
            .map(|(line, _)| *line)
            .collect::<Vec<_>>(),
        [19, 20]
    );
    assert!(
        machine
            .samd_tc_irqs
            .iter()
            .all(|(_, handle)| handle.poll(SimTime::from_ticks(4)))
    );
}

#[test]
fn maps_tcc_pwm_and_rtc_mode0_to_vendor_irqs() {
    let mut machine = ArmMcuMachine::new(TargetId::Atsamd21e18).unwrap();

    for base in [0x4200_2000, 0x4200_2400, 0x4200_2800] {
        machine
            .bus
            .write(base + 0x40, AccessWidth::Word, 9, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base + 0x44, AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base + 0x28, AccessWidth::Word, 1 << 16, SimTime::ZERO)
            .unwrap();
        machine
            .bus
            .write(base, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
    }
    assert_eq!(
        machine
            .samd_tcc_irqs
            .iter()
            .map(|(line, _)| *line)
            .collect::<Vec<_>>(),
        [15, 16, 17]
    );
    assert!(
        machine
            .samd_tcc_irqs
            .iter()
            .all(|(_, handle)| handle.poll(SimTime::from_ticks(4)).unwrap())
    );

    machine
        .bus
        .write(0x4000_1418, AccessWidth::Word, 5, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_1407, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4000_1400, AccessWidth::HalfWord, 2, SimTime::ZERO)
        .unwrap();
    let rtc = machine.samd_rtc.as_ref().expect("SAM D21 RTC");
    assert!(!rtc.poll(SimTime::from_ticks(4)));
    assert!(rtc.poll(SimTime::from_ticks(5)));
    assert_eq!(rtc.count(SimTime::from_ticks(5)), 5);
}
