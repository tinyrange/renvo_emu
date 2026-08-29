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
