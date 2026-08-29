use super::*;

#[test]
fn maps_ra4m1_rtc_calendar_and_alarm_handle() {
    const RTC_BASE: u64 = 0x4004_4000;
    let mut machine = ArmMcuMachine::new(TargetId::R7fa4m1ab3cfm).unwrap();

    machine
        .bus
        .write(RTC_BASE + 0x24, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    assert_eq!(
        machine
            .bus
            .read(
                RTC_BASE + 0x02,
                AccessWidth::Byte,
                AccessKind::Read,
                SimTime::from_ticks(3),
            )
            .unwrap(),
        3
    );

    machine
        .bus
        .write(RTC_BASE + 0x1e, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(RTC_BASE + 0x22, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    assert!(
        machine
            .ra_rtc
            .as_ref()
            .unwrap()
            .poll(SimTime::from_ticks(1))
    );
}

#[test]
fn maps_ra4m1_dac12_data_and_output_enable() {
    let mut machine = ArmMcuMachine::new(TargetId::R7fa4m1ab3cfm).unwrap();
    machine
        .bus
        .write(0x4005_e000, AccessWidth::HalfWord, 0x0abc, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4005_e004, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
        .unwrap();
    assert_eq!(machine.dac_value(), Some(0x0abc));
    assert_eq!(
        machine
            .bus
            .read(
                0x4005_e004,
                AccessWidth::Byte,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        0x5f
    );
}

#[test]
fn maps_ra4m1_cac_measurement_registers() {
    const CAC_BASE: u64 = 0x4004_4600;
    let mut machine = ArmMcuMachine::new(TargetId::R7fa4m1ab3cfm).unwrap();
    machine
        .bus
        .write(CAC_BASE + 0x06, AccessWidth::HalfWord, 100, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(CAC_BASE + 0x08, AccessWidth::HalfWord, 50, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(CAC_BASE, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    let cac = machine.cac().unwrap();
    cac.reference_edge(75);
    cac.reference_edge(75);
    assert_eq!(
        machine
            .bus
            .read(
                CAC_BASE + 0x0a,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
        75
    );
    assert_eq!(cac.flags(), (false, true, false));
}
