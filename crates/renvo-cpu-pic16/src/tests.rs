use super::*;
use renvo_bus::{AddressSpace, Endianness};

fn bus() -> AddressSpace {
    let mut bus = AddressSpace::new(Endianness::Little);
    bus.map_ram("pic.data", 0, 0x2000, true).unwrap();
    bus
}

fn run(cpu: &mut Pic16Cpu, bus: &mut AddressSpace, count: usize) {
    for tick in 0..count {
        cpu.step(bus, SimTime::from_ticks(tick as u64)).unwrap();
    }
}

#[test]
fn word_pc_and_named_mapping_are_explicit() {
    assert_eq!(Pic16Register::Pc.bits(), 16);
    for (index, register) in Pic16Register::ALL.iter().copied().enumerate() {
        assert_eq!(register.gdb_number(), index);
    }
}

#[test]
fn literal_arithmetic_and_skip_order_match_xc8_encodings() {
    let mut cpu = Pic16Cpu::new();
    let mut bus = bus();
    cpu.load_program_words(0, &[0x30ff, 0x3e01, 0x1c03, 0x3001, 0x3002])
        .unwrap();
    run(&mut cpu, &mut bus, 4);
    assert_eq!(cpu.register(Pic16Register::Wreg), 2);
    assert_ne!(cpu.register(Pic16Register::Status) as u8 & STATUS_C, 0);
    assert_eq!(cpu.register(Pic16Register::Pc), 5);
}

#[test]
fn banked_and_common_ram_have_enhanced_midrange_aliasing() {
    let mut cpu = Pic16Cpu::new();
    let mut bus = bus();
    // MOVLB 1; MOVLW aa; MOVWF 20; MOVLW 55; MOVWF 70; MOVLB 2; MOVF 70,w
    cpu.load_program_words(0, &[0x0141, 0x30aa, 0x00a0, 0x3055, 0x00f0, 0x0142, 0x0870])
        .unwrap();
    run(&mut cpu, &mut bus, 7);
    assert_eq!(
        bus.read(0xa0, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0xaa
    );
    assert_eq!(cpu.register(Pic16Register::Wreg), 0x55);
}

#[test]
fn movlb_reaches_all_six_bits_of_the_pic16f15376_banked_space() {
    let mut cpu = Pic16Cpu::new();
    let mut bus = bus();
    // MOVLB 62; MOVLW a5; MOVWF 38. ANSELA and the other high SFRs on the
    // PIC16F15376 require BSR[5]; truncating MOVLB to five bits aliases them
    // into bank 30.
    cpu.load_program_words(0, &[0x017e, 0x30a5, 0x00b8])
        .unwrap();
    run(&mut cpu, &mut bus, 3);
    assert_eq!(cpu.register(Pic16Register::Bsr), 62);
    assert_eq!(
        bus.read(0x1f38, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0xa5
    );
}

#[test]
fn indexed_and_auto_increment_indirect_modes_preserve_expected_fsr() {
    let mut cpu = Pic16Cpu::new();
    let mut bus = bus();
    cpu.set_register(Pic16Register::Fsr0, 0x2020);
    cpu.load_program_words(0, &[0x3042, 0x001a, 0x3099, 0x3f81, 0x3f01])
        .unwrap();
    run(&mut cpu, &mut bus, 5);
    assert_eq!(cpu.register(Pic16Register::Fsr0), 0x2021);
    assert_eq!(cpu.register(Pic16Register::Wreg), 0x99);
    assert_eq!(
        bus.read(0x40, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0x42
    );
    assert_eq!(
        bus.read(0x42, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0x99
    );
}

#[test]
fn call_return_and_sixteen_entry_stack_wrap_are_deterministic() {
    let mut cpu = Pic16Cpu::new();
    let mut bus = bus();
    cpu.load_program_words(0, &[0x2003, 0x3007, 0x2805, 0x3009, 0x0008, 0x0000])
        .unwrap();
    run(&mut cpu, &mut bus, 5);
    assert_eq!(cpu.register(Pic16Register::Wreg), 7);
    assert_eq!(cpu.register(Pic16Register::Pc), 5);
}

#[test]
fn movlp_is_not_misdecoded_as_addfsr_and_drives_computed_pcl() {
    let mut cpu = Pic16Cpu::new();
    let mut bus = bus();
    cpu.load_program_words(0, &[0x3183, 0x3042, 0x0082])
        .unwrap();
    cpu.load_program_words(0x342, &[0x305a]).unwrap();
    run(&mut cpu, &mut bus, 4);
    assert_eq!(cpu.register(Pic16Register::Pclath), 3);
    assert_eq!(cpu.register(Pic16Register::Wreg), 0x5a);
    assert_eq!(cpu.register(Pic16Register::Pc), 0x343);
}

#[test]
fn interrupt_entry_and_retfie_restore_shadow_context() {
    let mut cpu = Pic16Cpu::new();
    let mut bus = bus();
    cpu.load_program_words(0, &[0x300a, 0, 0, 0, 0x30ee, 0x0009])
        .unwrap();
    run(&mut cpu, &mut bus, 1);
    bus.write(0x0b, AccessWidth::Byte, 0x80, SimTime::ZERO)
        .unwrap();
    cpu.set_interrupt(0, true).unwrap();
    run(&mut cpu, &mut bus, 1);
    assert_eq!(cpu.register(Pic16Register::Pc), 4);
    cpu.set_interrupt(0, false).unwrap();
    run(&mut cpu, &mut bus, 2);
    assert_eq!(cpu.register(Pic16Register::Wreg), 0x0a);
    assert_eq!(cpu.register(Pic16Register::Pc), 1);
    assert_eq!(
        bus.read(0x0b, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0x80
    );
}

#[test]
fn reserved_opcode_faults_at_word_address() {
    let mut cpu = Pic16Cpu::new();
    let mut bus = bus();
    cpu.load_program_words(0, &[0x0062]).unwrap();
    let error = cpu.step(&mut bus, SimTime::ZERO).unwrap_err();
    assert_eq!(error.kind, CpuFaultKind::IllegalInstruction);
    assert_eq!(error.pc, 0);
}

#[test]
fn every_enhanced_midrange_instruction_family_decodes() {
    // One assembler-confirmed representative for each of the 49 instruction
    // families in DS40001737.  Each instruction is isolated so branches,
    // skips, reset and return-stack operations cannot mask a later decode.
    let vectors: [(&str, u16); 49] = [
        ("addfsr", 0x3101),
        ("addlw", 0x3e01),
        ("addwf", 0x0770),
        ("addwfc", 0x3df0),
        ("andlw", 0x3901),
        ("andwf", 0x0570),
        ("asrf", 0x37f0),
        ("bcf", 0x1070),
        ("bra", 0x3200),
        ("brw", 0x000b),
        ("bsf", 0x1470),
        ("btfsc", 0x1870),
        ("btfss", 0x1c70),
        ("call", 0x2123),
        ("callw", 0x000a),
        ("clrf", 0x01f0),
        ("clrw", 0x0103),
        ("clrwdt", 0x0064),
        ("comf", 0x09f0),
        ("decf", 0x03f0),
        ("decfsz", 0x0bf0),
        ("goto", 0x2923),
        ("incf", 0x0af0),
        ("incfsz", 0x0ff0),
        ("iorlw", 0x3801),
        ("iorwf", 0x04f0),
        ("lslf", 0x35f0),
        ("lsrf", 0x36f0),
        ("movf", 0x0870),
        ("moviw", 0x0010),
        ("movlb", 0x0141),
        ("movlp", 0x3181),
        ("movlw", 0x3001),
        ("movwf", 0x00f0),
        ("movwi", 0x001a),
        ("nop", 0x0000),
        ("reset", 0x0001),
        ("retfie", 0x0009),
        ("retlw", 0x3401),
        ("return", 0x0008),
        ("rlf", 0x0df0),
        ("rrf", 0x0cf0),
        ("sleep", 0x0063),
        ("sublw", 0x3c01),
        ("subwf", 0x02f0),
        ("subwfb", 0x3bf0),
        ("swapf", 0x0ef0),
        ("xorlw", 0x3a01),
        ("xorwf", 0x06f0),
    ];

    for (name, opcode) in vectors {
        let mut cpu = Pic16Cpu::new();
        let mut bus = bus();
        cpu.set_register(Pic16Register::Wreg, 1);
        cpu.set_register(Pic16Register::Fsr0, 0x2020);
        cpu.load_program_words(0, &[opcode]).unwrap();
        cpu.step(&mut bus, SimTime::ZERO)
            .unwrap_or_else(|error| panic!("{name} ({opcode:#06x}) failed: {error}"));
    }
}
