#[test]
fn esp32c6_completed_invalid_channel_has_distinct_error() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    program_esp32c6_wifi_rf(&mut machine, 6, 56);
    machine
        .bus
        .write(
            0x600a_00c0,
            AccessWidth::Word,
            0x4284_4000 | (1 << 14) | 0x123,
            machine.now,
        )
        .unwrap();
    write_gain_table(&mut machine, 56);
    let MachineError::RadioLegality(error) = machine.c6_wifi_rf_airtime().unwrap_err() else {
        panic!("completed invalid channel selection should be a hard legality error");
    };
    assert_eq!(error.rule, remu_radio::RadioLegalityRule::RfChannel);
}

#[test]
fn esp32c6_structured_rf_state_fuzz_reaches_every_causal_error_path() {
    use remu_radio::RadioLegalityRule;
    use std::collections::BTreeSet;

    let mut reached = BTreeSet::new();
    let mut state = 0x356c_6f72_6163_6c65_u64;
    for iteration in 0..2048_u32 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let mutation = iteration % 8;
        let channel = [1_u8, 6, 11][state as usize % 3];
        let power = [32_i16, 56, 80][(state >> 8) as usize % 3];
        let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();

        if mutation == 0 || mutation >= 3 {
            program_esp32c6_wifi_rf(&mut machine, channel, power);
        }
        match mutation {
            0 => {
                assert!(machine.c6_wifi_rf_airtime().is_ok());
                continue;
            }
            1 => {}
            2 => {
                machine
                    .bus
                    .write(
                        0x600a_9814,
                        AccessWidth::Word,
                        (1 << 9) | (1 << 10),
                        machine.now,
                    )
                    .unwrap();
            }
            3 => {
                machine
                    .bus
                    .write(
                        0x600a_00c0,
                        AccessWidth::Word,
                        0x4284_4000 | (1 << 14) | 0x123,
                        machine.now,
                    )
                    .unwrap();
                write_gain_table(&mut machine, power);
            }
            4 => {
                for (address, value) in [
                    (0x600a_08cc, state),
                    (0x600a_08d0, state.rotate_left(17)),
                    (0x600a_08d4, 0xfe),
                ] {
                    machine
                        .bus
                        .write(address, AccessWidth::Word, value, machine.now)
                        .unwrap();
                }
            }
            5 => program_esp32c6_wifi_rf(&mut machine, channel, 34),
            6 => {
                machine
                    .bus
                    .write(
                        0x600a_00c0,
                        AccessWidth::Word,
                        0x4684_4000 | (1 << 14) | 0x380 + u64::from(channel) * 0x280,
                        machine.now,
                    )
                    .unwrap();
                write_gain_table(&mut machine, power);
            }
            7 => {
                machine
                    .bus
                    .write(0x600a_0910, AccessWidth::Word, 0x200, machine.now)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let MachineError::RadioLegality(error) = machine.c6_wifi_rf_airtime().unwrap_err() else {
            panic!("seeded mutation {mutation} unexpectedly escaped RF legality");
        };
        let expected = match mutation {
            1 => RadioLegalityRule::DomainReady,
            2 => RadioLegalityRule::RfPllLock,
            3 => RadioLegalityRule::RfChannel,
            4 => RadioLegalityRule::RfCalibration,
            5 => RadioLegalityRule::RfPower,
            6 => RadioLegalityRule::RfBandwidth,
            7 => RadioLegalityRule::RfFrontend,
            _ => unreachable!("valid mutation exits before legality comparison"),
        };
        assert_eq!(error.rule, expected, "seeded mutation {mutation}");
        reached.insert(error.rule);
    }
    assert_eq!(
        reached,
        BTreeSet::from([
            RadioLegalityRule::DomainReady,
            RadioLegalityRule::RfPllLock,
            RadioLegalityRule::RfCalibration,
            RadioLegalityRule::RfChannel,
            RadioLegalityRule::RfPower,
            RadioLegalityRule::RfBandwidth,
            RadioLegalityRule::RfFrontend,
        ])
    );
}

fn write_gain_table(machine: &mut RiscVMachine, power: i16) {
    for entry in 0..43_u64 {
        let final_word = if entry == 0 {
            0xfe
        } else if entry == 42 {
            u64::from(((power as i32 - 133) * 128) as u32)
        } else {
            entry
        };
        for (address, value) in [
            (0x600a_08cc, entry),
            (0x600a_08d0, entry),
            (0x600a_08d4, final_word),
        ] {
            machine
                .bus
                .write(address, AccessWidth::Word, value, machine.now)
                .unwrap();
        }
    }
}
