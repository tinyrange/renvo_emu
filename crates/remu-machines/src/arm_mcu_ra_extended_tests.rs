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

#[test]
fn maps_all_ra4m1_gpt_channels_with_native_counter_widths() {
    let mut machine = ArmMcuMachine::new(TargetId::R7fa4m1ab3cfm).unwrap();
    for index in 1_u64..=7 {
        let base = 0x4007_8000 + index * 0x100;
        machine
            .bus
            .write(base + 0x64, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        let expected = if index <= 2 { 0x1234_5678 } else { 0x5678 };
        assert_eq!(
            machine
                .bus
                .read(
                    base + 0x64,
                    AccessWidth::Word,
                    AccessKind::Read,
                    SimTime::ZERO,
                )
                .unwrap(),
            expected,
            "GPT{index} period width"
        );
    }
    assert_eq!(
        machine
            .ra_gpt
            .iter()
            .map(|(event, _)| *event)
            .collect::<Vec<_>>(),
        [0x065, 0x06d, 0x075, 0x07d, 0x085, 0x08d, 0x095]
    );
}

#[test]
fn maps_ra4m1_kint_and_routes_selected_pin_through_icu() {
    let mut machine = ArmMcuMachine::new(TargetId::R7fa4m1ab3cfm).unwrap();
    machine
        .bus
        .write(0x4008_0000, AccessWidth::Byte, 0x81, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(0x4008_0008, AccessWidth::Byte, 1, SimTime::ZERO)
        .unwrap();
    machine
        .bus
        .write(
            0x4000_6324,
            AccessWidth::Word,
            u64::from(RA4M1_EVENT_KINT),
            SimTime::ZERO,
        )
        .unwrap();
    let kint = machine.ra_kint.as_ref().unwrap();
    assert!(!kint.poll(0));
    assert!(kint.poll(1));
    assert_eq!(
        machine
            .ra_icu
            .as_ref()
            .unwrap()
            .route_event(RA4M1_EVENT_KINT),
        vec![9]
    );
}
