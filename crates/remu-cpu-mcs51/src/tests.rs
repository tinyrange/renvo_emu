use super::*;
use remu_bus::{AddressSpace, Endianness};

fn bus() -> AddressSpace {
    let mut bus = AddressSpace::new(Endianness::Little);
    bus.map_ram("efm8.xdata", 0, 0x1_0000, true).unwrap();
    bus.map_ram("efm8.sfr", SFR_BUS_BASE, 0x1_0000, true)
        .unwrap();
    bus
}

fn run(cpu: &mut Mcs51Cpu, bus: &mut AddressSpace, count: usize) {
    for tick in 0..count {
        cpu.step(bus, SimTime::from_ticks(tick as u64)).unwrap();
    }
}

#[test]
fn named_registers_and_split_spaces_are_explicit() {
    assert_eq!(Mcs51AddressSpace::Code, Mcs51AddressSpace::Code);
    assert_ne!(
        Mcs51AddressSpace::InternalData,
        Mcs51AddressSpace::ExternalData
    );
    assert_ne!(Mcs51AddressSpace::Sfr, Mcs51AddressSpace::Bit);
    assert_eq!(Mcs51Register::Pc.bits(), 16);
    assert_eq!(Mcs51Register::A.bits(), 8);
    for (number, register) in Mcs51Register::ALL.iter().copied().enumerate() {
        assert_eq!(register.gdb_number(), number);
        assert!(!register.name().is_empty());
    }
}

#[test]
fn register_banks_direct_ram_and_indirect_upper_ram_do_not_alias() {
    let mut cpu = Mcs51Cpu::new();
    let mut bus = bus();
    // MOV R0,#80; MOV @R0,#a5; MOV 00,#11; MOV PSW,#08; MOV R0,#22
    cpu.load_code(
        0,
        &[
            0x78, 0x80, 0x76, 0xa5, 0x75, 0x00, 0x11, 0x75, 0xd0, 0x08, 0x78, 0x22,
        ],
    )
    .unwrap();
    run(&mut cpu, &mut bus, 5);
    assert_eq!(cpu.idata[0x80], 0xa5);
    assert_eq!(cpu.idata[0], 0x11);
    assert_eq!(cpu.register(Mcs51Register::R0), 0x22);
    cpu.set_register(Mcs51Register::Psw, 0);
    assert_eq!(cpu.register(Mcs51Register::R0), 0x11);
}

#[test]
fn arithmetic_sets_carry_auxiliary_carry_and_overflow() {
    let mut cpu = Mcs51Cpu::new();
    let mut bus = bus();
    // MOV A,#7f; ADD A,#01; ADD A,#ff; SETB C; SUBB A,#7f
    cpu.load_code(0, &[0x74, 0x7f, 0x24, 0x01, 0x24, 0xff, 0xd3, 0x94, 0x7f])
        .unwrap();
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.a, 0x80);
    assert_eq!(cpu.psw & PSW_C, 0);
    assert_ne!(cpu.psw & PSW_AC, 0);
    assert_ne!(cpu.psw & PSW_OV, 0);
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.a, 0x7f);
    assert_ne!(cpu.psw & PSW_C, 0);
    assert_eq!(cpu.psw & PSW_AC, 0);
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.a, 0xff);
    assert_ne!(cpu.psw & PSW_C, 0);
    assert_eq!(cpu.psw & PSW_OV, 0);
}

#[test]
fn calls_stack_and_relative_branches_preserve_return_order() {
    let mut cpu = Mcs51Cpu::new();
    let mut bus = bus();
    // LCALL 0008; MOV A,#42; SJMP +5; subroutine: MOV A,#11; RET
    cpu.load_code(
        0,
        &[
            0x12, 0x00, 0x08, 0x74, 0x42, 0x80, 0x05, 0x00, 0x74, 0x11, 0x22,
        ],
    )
    .unwrap();
    run(&mut cpu, &mut bus, 5);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.pc, 12);
    assert_eq!(cpu.sp, 7);
}

#[test]
fn bit_movc_and_movx_operations_use_their_own_spaces() {
    let mut cpu = Mcs51Cpu::new();
    let mut bus = bus();
    // SETB bit 00; MOV C,bit 00; MOV bit 01,C; MOV DPTR,#1234;
    // MOV A,#5a; MOVX @DPTR,A; CLR A; MOVX A,@DPTR
    cpu.load_code(
        0,
        &[
            0xd2, 0x00, 0xa2, 0x00, 0x92, 0x01, 0x90, 0x12, 0x34, 0x74, 0x5a, 0xf0, 0xe4, 0xe0,
        ],
    )
    .unwrap();
    run(&mut cpu, &mut bus, 8);
    assert_eq!(cpu.idata[0x20] & 3, 3);
    assert_eq!(cpu.a, 0x5a);
    assert_eq!(
        bus.read(0x1234, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0x5a
    );

    let mut movc = Mcs51Cpu::new();
    movc.load_code(
        0,
        &[0x90, 0x00, 0x08, 0x74, 0x01, 0x93, 0x00, 0x00, 0xaa, 0x5c],
    )
    .unwrap();
    run(&mut movc, &mut bus, 3);
    assert_eq!(movc.a, 0x5c);
}

#[test]
fn multiply_divide_and_decimal_adjust_match_base_mcs51() {
    let mut cpu = Mcs51Cpu::new();
    let mut bus = bus();
    cpu.load_code(
        0,
        &[
            0x74, 25, 0x75, 0xf0, 10, 0xa4, 0x75, 0xf0, 7, 0x84, 0x74, 0x09, 0x24, 0x09, 0xd4,
        ],
    )
    .unwrap();
    run(&mut cpu, &mut bus, 3);
    assert_eq!((cpu.b, cpu.a), (0, 250));
    assert_eq!(cpu.psw & PSW_OV, 0);
    run(&mut cpu, &mut bus, 2);
    assert_eq!((cpu.a, cpu.b), (35, 5));
    run(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.a, 0x18);
}

#[test]
fn interrupts_obey_priority_and_reti_restores_nesting() {
    let mut cpu = Mcs51Cpu::new();
    let mut bus = bus();
    cpu.load_code(0, &[0x00]).unwrap();
    cpu.load_code(0x0b, &[0x32]).unwrap();
    cpu.load_code(0x23, &[0x32]).unwrap();
    cpu.set_interrupt(0, true).unwrap();
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.pc, 0x0b);
    cpu.set_interrupt(0, false).unwrap();
    cpu.set_interrupt(4, true).unwrap();
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.pc, 0x23);
    cpu.set_interrupt(4, false).unwrap();
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.pc, 0);
    assert_eq!(cpu.active_priority, None);
}

#[test]
fn spi0_interrupt_uses_the_efm8_vector_at_low_or_high_priority() {
    let mut cpu = Mcs51Cpu::new();
    let mut bus = bus();
    cpu.load_code(0x33, &[0x32]).unwrap();

    cpu.set_interrupt(6, true).unwrap();
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.pc, 0x33);
    cpu.set_interrupt(6, false).unwrap();
    run(&mut cpu, &mut bus, 1);

    cpu.set_interrupt(7, true).unwrap();
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.pc, 0x33);
}

#[test]
fn every_base_opcode_except_reserved_a5_decodes() {
    for opcode in 0_u8..=u8::MAX {
        let mut cpu = Mcs51Cpu::new();
        let mut bus = bus();
        cpu.load_code(0, &[opcode, 0, 0, 0]).unwrap();
        let result = cpu.step(&mut bus, SimTime::ZERO);
        if opcode == 0xa5 {
            assert_eq!(result.unwrap_err().kind, CpuFaultKind::IllegalInstruction);
        } else if let Err(error) = result {
            panic!("legal opcode {opcode:#04x} did not decode: {error}");
        }
    }
}

#[test]
fn reset_and_illegal_opcode_are_deterministic() {
    let mut cpu = Mcs51Cpu::new();
    let mut bus = bus();
    cpu.set_register(Mcs51Register::A, 0x55);
    cpu.load_code(0, &[0xa5]).unwrap();
    cpu.reset(ResetKind::External, &mut bus).unwrap();
    assert_eq!(cpu.register(Mcs51Register::A), 0);
    assert_eq!(cpu.register(Mcs51Register::Sp), 7);
    let error = cpu.step(&mut bus, SimTime::ZERO).unwrap_err();
    assert_eq!(error.kind, CpuFaultKind::IllegalInstruction);
    assert_eq!(error.pc, 0);
}
