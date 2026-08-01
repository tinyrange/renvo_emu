use super::*;
use renvo_trace::{Timescale, VcdWriter};

#[test]
fn esp32c6_rom_systimer_period_is_visible_to_inlined_isr_reads() {
    let mut machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
    machine.cpu.set_pc(0x4000_03d8).unwrap();
    machine.cpu.set_register(RiscVRegister::A1, 0).unwrap();
    machine.cpu.set_register(RiscVRegister::A2, 1_000).unwrap();
    // set_alarm_period() has a 32-bit third argument; A3 must not leak
    // into the host-side period even if it contains unrelated state.
    machine
        .cpu
        .set_register(RiscVRegister::A3, u32::MAX)
        .unwrap();

    assert!(machine.service_functional_bootrom().unwrap());

    assert_eq!(machine.esp_systimer_periods[0], 1_000);
    assert_eq!(
        machine
            .bus
            .read(
                ESP32C6_SYSTIMER_TARGET_CONF,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap()
            & ((1 << 26) - 1),
        1_000
    );
}

#[test]
fn all_initial_riscv_modes_execute_and_halt_deterministically() {
    // addi x1,x0,7; addi x2,x0,5; add x3,x1,x2; ebreak
    let program = [0x0070_0093_u32, 0x0050_0113, 0x0020_81b3, 0x0010_0073]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect::<Vec<_>>();
    for target in [
        TargetId::Ch32v003,
        TargetId::Ch32v006,
        TargetId::Esp32c6,
        TargetId::Rp2350,
    ] {
        let entry = target_manifest(target).memory[0].start;
        let mut machine = RiscVMachine::new(target).unwrap();
        machine.load_bytes(entry, &program).unwrap();
        machine
            .set_entry(u32::try_from(entry).expect("initial addresses fit RV32"))
            .unwrap();
        let result = machine
            .run(
                RunLimits {
                    instructions: Some(16),
                    deadline: None,
                },
                None,
            )
            .unwrap();
        assert_eq!(result.reason, StopReason::Halted, "{target}");
        assert_eq!(result.cpu.registers[3].value, 12, "{target}");
    }
}

#[test]
fn gpio_facade_streams_valid_vcd() {
    // lui x1,0xffff0; addi x2,x0,1; sw x2,0(x1); sw x2,4(x1); ebreak
    let program = [
        0xffff_00b7_u32,
        0x0010_0113,
        0x0020_a023,
        0x0020_a223,
        0x0010_0073,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect::<Vec<_>>();
    let mut machine = RiscVMachine::new(TargetId::Ch32v003).unwrap();
    machine.load_bytes(0, &program).unwrap();
    machine.set_entry(0).unwrap();
    let mut vcd = VcdWriter::new(Vec::new(), Timescale::Nanosecond);
    let result = machine
        .run(
            RunLimits {
                instructions: Some(16),
                deadline: None,
            },
            Some(&mut vcd),
        )
        .unwrap();
    assert_eq!(result.reason, StopReason::Halted);
    let output = String::from_utf8(vcd.into_inner()).unwrap();
    assert!(output.contains("$enddefinitions $end"));
    assert!(output.contains("#3"));
}

#[test]
fn unsupported_targets_fail_explicitly() {
    assert!(matches!(
        RiscVMachine::new(TargetId::Rp2040),
        Err(MachineError::UnsupportedTarget(TargetId::Rp2040))
    ));
}
