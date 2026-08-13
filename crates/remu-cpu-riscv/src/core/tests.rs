use super::*;
use remu_bus::AddressSpace;

fn cpu_and_bus(words: &[u32], profile: RiscVProfile) -> (RiscVCpu, AddressSpace) {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 4096, true).unwrap();
    let bytes = words
        .iter()
        .flat_map(|word| word.to_le_bytes())
        .collect::<Vec<_>>();
    bus.load(0, &bytes).unwrap();
    let mut cpu = RiscVCpu::new(profile).unwrap();
    cpu.set_pc(0).unwrap();
    (cpu, bus)
}

#[test]
fn executes_integer_program_and_halts() {
    // addi x1,x0,7; addi x2,x0,5; add x3,x1,x2; ebreak
    let words = [0x0070_0093, 0x0050_0113, 0x0020_81b3, 0x0010_0073];
    let (mut cpu, mut bus) = cpu_and_bus(&words, RiscVProfile::esp32c6());
    cpu.set_pc(0).unwrap();
    for _ in 0..3 {
        assert_eq!(
            cpu.step(&mut bus, SimTime::ZERO).unwrap().reason,
            StepReason::Advanced
        );
    }
    assert_eq!(cpu.register(RiscVRegister::Gp).unwrap(), 12);
    assert_eq!(
        cpu.step(&mut bus, SimTime::ZERO).unwrap().reason,
        StepReason::Halted
    );
}

#[test]
fn rv32e_rejects_high_registers() {
    // addi x16,x0,1
    let (mut cpu, mut bus) = cpu_and_bus(&[0x0010_0813], RiscVProfile::ch32v003());
    let fault = cpu.step(&mut bus, SimTime::ZERO).unwrap_err();
    assert_eq!(fault.kind, CpuFaultKind::IllegalInstruction);
}

#[test]
fn loads_and_stores_little_endian_words() {
    // addi x1,x0,64; addi x2,x0,42; sw x2,0(x1); lw x3,0(x1)
    let words = [0x0400_0093, 0x02a0_0113, 0x0020_a023, 0x0000_a183];
    let (mut cpu, mut bus) = cpu_and_bus(&words, RiscVProfile::esp32c6());
    for _ in words {
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
    }
    assert_eq!(cpu.register(RiscVRegister::Gp).unwrap(), 42);
}

#[test]
fn m_extension_division_corner_cases_match_riscv() {
    assert_eq!(execute_m(4, 7, 0), Some(u32::MAX));
    assert_eq!(execute_m(6, 7, 0), Some(7));
    assert_eq!(execute_m(4, 0x8000_0000, u32::MAX), Some(0x8000_0000));
}

#[test]
fn hazard3_zbkb_pack_combines_low_halfwords() {
    // pack x15,x19,x0
    let (mut cpu, mut bus) = cpu_and_bus(&[0x0809_c7b3], RiscVProfile::rp2350_hazard3());
    cpu.set_register(RiscVRegister::S3, 0x1234_abcd).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(RiscVRegister::A5).unwrap(), 0xabcd);
}

#[test]
fn zbb_signed_and_unsigned_min_max_encodings_are_distinct() {
    assert_eq!(execute_b_register(0x05, 4, u32::MAX, 1), Some(u32::MAX));
    assert_eq!(execute_b_register(0x05, 5, u32::MAX, 1), Some(1));
    assert_eq!(execute_b_register(0x05, 6, u32::MAX, 1), Some(1));
    assert_eq!(execute_b_register(0x05, 7, u32::MAX, 1), Some(u32::MAX));
    // Exact operands from the official RP2350 RISC-V MicroPython CDC path.
    assert_eq!(execute_b_register(0x05, 5, 65_490, 64), Some(64));
}

#[test]
fn zbb_sign_extend_byte_and_halfword_use_their_unary_encodings() {
    assert_eq!(
        execute_b_immediate(0x604a_1a13, 1, 0x0000_00a5, 4),
        Some(0xffff_ffa5)
    );
    assert_eq!(
        execute_b_immediate(0x605a_1a13, 1, 0x0000_a5a5, 5),
        Some(0xffff_a5a5)
    );
}

#[test]
fn esp32c6_memory_protection_csrs_persist_and_reset() {
    let mut cpu = RiscVCpu::new(RiscVProfile::esp32c6()).unwrap();
    let mut bus = AddressSpace::default();

    for (address, value) in [
        (CSR_PMPCFG0, 0x9f18_0f00),
        (CSR_PMPADDR15, 0x3fff_ffff),
        (CSR_PMACFG0, 0x0000_001f),
        (CSR_PMAADDR15, 0x4000_0000),
        (CSR_ESP_PCER_MACHINE, 0x0000_00ff),
        (CSR_ESP_PCMR_MACHINE, 0x0000_0001),
    ] {
        cpu.write_csr(address, value).unwrap();
        assert_eq!(cpu.read_csr(address).unwrap(), value);
    }
    cpu.write_csr(CSR_ESP_PCCR_MACHINE, 1234).unwrap();
    assert_eq!(cpu.read_csr(CSR_ESP_PCCR_MACHINE).unwrap(), 1234);
    assert_eq!(cpu.read_csr(CSR_ESP_PCCR_USER).unwrap(), 1234);
    cpu.write_csr(CSR_ESP_PCCR_USER, 5678).unwrap();
    assert_eq!(cpu.read_csr(CSR_ESP_PCCR_MACHINE).unwrap(), 5678);
    cpu.write_csr(CSR_ESP_PCER_USER, 0x55aa).unwrap();
    cpu.write_csr(CSR_ESP_PCMR_USER, 3).unwrap();
    assert_eq!(cpu.read_csr(CSR_ESP_PCER_MACHINE).unwrap(), 0x55aa);
    assert_eq!(cpu.read_csr(CSR_ESP_PCER_USER).unwrap(), 0x55aa);
    assert_eq!(cpu.read_csr(CSR_ESP_PCMR_MACHINE).unwrap(), 3);
    assert_eq!(cpu.read_csr(CSR_ESP_PCMR_USER).unwrap(), 3);

    cpu.reset(ResetKind::PowerOn, &mut bus).unwrap();
    assert_eq!(cpu.read_csr(CSR_PMPCFG0).unwrap(), 0);
    assert_eq!(cpu.read_csr(CSR_PMPADDR15).unwrap(), 0);
    assert_eq!(cpu.read_csr(CSR_PMACFG0).unwrap(), 0);
    assert_eq!(cpu.read_csr(CSR_PMAADDR15).unwrap(), 0);
    assert_eq!(cpu.read_csr(CSR_ESP_PCER_MACHINE).unwrap(), 0);
    assert_eq!(cpu.read_csr(CSR_ESP_PCER_USER).unwrap(), 0);
    assert_eq!(cpu.read_csr(CSR_ESP_PCMR_MACHINE).unwrap(), 0);
    assert_eq!(cpu.read_csr(CSR_ESP_PCMR_USER).unwrap(), 0);
    assert_eq!(cpu.read_csr(CSR_ESP_PCCR_MACHINE).unwrap(), 0);
    assert_eq!(cpu.read_csr(CSR_ESP_PCCR_USER).unwrap(), 0);
}

#[test]
fn esp32c6_memory_protection_csrs_are_profile_gated() {
    let cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
    let fault = cpu.read_csr(CSR_PMAADDR0).unwrap_err();
    assert_eq!(fault.kind, CpuFaultKind::Unsupported);
}

#[test]
fn esp32c6_pmp_execute_only_napot_region_traps_user_loads() {
    // lw x1,0(x0) lies in a 4 KiB execute-only NAPOT region.
    let (mut cpu, mut bus) = cpu_and_bus(&[0x0000_2083], RiscVProfile::esp32c6());
    cpu.write_csr(CSR_PMPADDR0, 0x1ff).unwrap();
    cpu.write_csr(CSR_PMPCFG0, 0x1c).unwrap();
    cpu.write_csr(CSR_MTVEC, 0x100).unwrap();
    cpu.write_csr(CSR_MEPC, 0).unwrap();
    cpu.write_csr(CSR_MSTATUS, 0).unwrap();
    cpu.execute_system(0x3020_0073, 0, 0, 0).unwrap();

    assert_eq!(
        cpu.step(&mut bus, SimTime::ZERO).unwrap().reason,
        StepReason::Advanced
    );
    assert_eq!(cpu.privilege(), RiscVPrivilege::Machine);
    assert_eq!(cpu.pc(), 0x100);
    assert_eq!(cpu.read_csr(CSR_MCAUSE).unwrap(), 5);
    assert_eq!(cpu.read_csr(CSR_MTVAL).unwrap(), 0);
    assert_eq!(cpu.read_csr(CSR_MEPC).unwrap(), 0);
}

#[test]
fn esp32c6_pmp_locked_config_and_tor_lower_bound_are_immutable() {
    let mut cpu = RiscVCpu::new(RiscVProfile::esp32c6()).unwrap();
    cpu.write_csr(CSR_PMPADDR0, 0x100).unwrap();
    cpu.write_csr(CSR_PMPADDR0 + 1, 0x200).unwrap();
    // Entry 1 is a locked TOR region, which also locks entry 0's address.
    cpu.write_csr(CSR_PMPCFG0, 0x89_00).unwrap();
    cpu.write_csr(CSR_PMPADDR0, 0x300).unwrap();
    cpu.write_csr(CSR_PMPADDR0 + 1, 0x400).unwrap();
    cpu.write_csr(CSR_PMPCFG0, 0).unwrap();

    assert_eq!(cpu.read_csr(CSR_PMPADDR0).unwrap(), 0x100);
    assert_eq!(cpu.read_csr(CSR_PMPADDR0 + 1).unwrap(), 0x200);
    assert_eq!(cpu.read_csr(CSR_PMPCFG0).unwrap(), 0x89_00);
}

#[test]
fn esp32c6_pmp_checks_a_complete_instruction_and_machine_bypasses_unlocked_entries() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 4096, true).unwrap();
    bus.load(2, &0x0010_0073_u32.to_le_bytes()).unwrap();

    let mut machine = RiscVCpu::new(RiscVProfile::esp32c6()).unwrap();
    machine.write_csr(CSR_PMPADDR0, 0).unwrap();
    machine.write_csr(CSR_PMPCFG0, 0x10).unwrap();
    machine.set_pc(2).unwrap();
    assert_eq!(
        machine.step(&mut bus, SimTime::ZERO).unwrap().reason,
        StepReason::Halted
    );

    let mut user = RiscVCpu::new(RiscVProfile::esp32c6()).unwrap();
    user.write_csr(CSR_PMPADDR0, 0).unwrap();
    user.write_csr(CSR_PMPCFG0, 0x15).unwrap();
    user.write_csr(CSR_MTVEC, 0x100).unwrap();
    user.write_csr(CSR_MEPC, 2).unwrap();
    user.write_csr(CSR_MSTATUS, 0).unwrap();
    user.execute_system(0x3020_0073, 0, 0, 0).unwrap();
    user.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(user.read_csr(CSR_MCAUSE).unwrap(), 1);
    assert_eq!(user.read_csr(CSR_MTVAL).unwrap(), 2);
}

#[test]
fn esp32c6_user_mode_ecall_and_privileged_csr_access_trap_to_machine() {
    let mut cpu = RiscVCpu::new(RiscVProfile::esp32c6()).unwrap();
    cpu.write_csr(CSR_MTVEC, 0x100).unwrap();
    cpu.write_csr(CSR_MEPC, 0x40).unwrap();
    cpu.write_csr(CSR_MSTATUS, 0).unwrap();

    cpu.execute_system(0x3020_0073, 0, 0, 0).unwrap();
    assert_eq!(cpu.privilege(), RiscVPrivilege::User);
    assert_eq!(cpu.pc(), 0x40);

    cpu.execute_system(0x0000_0073, 0, 0, 0).unwrap();
    assert_eq!(cpu.privilege(), RiscVPrivilege::Machine);
    assert_eq!(cpu.pc(), 0x100);
    assert_eq!(cpu.read_csr(CSR_MCAUSE).unwrap(), 8);
    assert_eq!(cpu.read_csr(CSR_MEPC).unwrap(), 0x40);
    assert_eq!(cpu.read_csr(CSR_MSTATUS).unwrap() & MSTATUS_MPP, 0);

    cpu.write_csr(CSR_MEPC, 0x44).unwrap();
    cpu.execute_system(0x3020_0073, 0, 0, 0).unwrap();
    assert_eq!(cpu.privilege(), RiscVPrivilege::User);
    // csrrs x1,mstatus,x0 is illegal from user mode.
    let instruction = 0x3000_20f3;
    cpu.execute_system(instruction, 1, 0, 2).unwrap();
    assert_eq!(cpu.privilege(), RiscVPrivilege::Machine);
    assert_eq!(cpu.pc(), 0x100);
    assert_eq!(cpu.read_csr(CSR_MCAUSE).unwrap(), 2);
    assert_eq!(cpu.read_csr(CSR_MTVAL).unwrap(), instruction);
    assert_eq!(cpu.read_csr(CSR_MEPC).unwrap(), 0x44);
}

#[test]
fn user_mode_machine_interrupt_saves_user_privilege_in_mpp() {
    let mut cpu = RiscVCpu::new(RiscVProfile::esp32c6()).unwrap();
    cpu.write_csr(CSR_MTVEC, 0x100).unwrap();
    cpu.write_csr(CSR_MEPC, 0x40).unwrap();
    cpu.execute_system(0x3020_0073, 0, 0, 0).unwrap();
    assert_eq!(cpu.privilege(), RiscVPrivilege::User);

    cpu.set_machine_interrupt_enabled(7, true).unwrap();
    cpu.set_interrupt(7, true).unwrap();
    assert_eq!(cpu.pending_interrupt(), Some(7));
    cpu.take_interrupt(7);

    assert_eq!(cpu.privilege(), RiscVPrivilege::Machine);
    assert_eq!(cpu.pc(), 0x100);
    assert_eq!(cpu.read_csr(CSR_MCAUSE).unwrap(), 0x8000_0007);
    assert_eq!(cpu.read_csr(CSR_MSTATUS).unwrap() & MSTATUS_MPP, 0);
}

#[test]
fn esp32c6_delegated_user_interrupt_enters_utvec_and_uret_restores_user_code() {
    let mut cpu = RiscVCpu::new(RiscVProfile::esp32c6()).unwrap();
    cpu.write_csr(CSR_MEPC, 0x40).unwrap();
    cpu.execute_system(0x3020_0073, 0, 0, 0).unwrap();
    cpu.write_csr(CSR_UTVEC, 0x200).unwrap();
    cpu.write_csr(CSR_MIDELEG, 1 << 4).unwrap();
    cpu.write_csr(CSR_UIE, 1 << 4).unwrap();
    cpu.write_csr(CSR_USTATUS, USTATUS_UIE).unwrap();
    cpu.set_interrupt(4, true).unwrap();

    assert_eq!(cpu.pending_interrupt(), Some(4));
    cpu.take_interrupt(4);
    assert_eq!(cpu.privilege(), RiscVPrivilege::User);
    assert_eq!(cpu.pc(), 0x200);
    assert_eq!(cpu.read_csr(CSR_UCAUSE).unwrap(), 0x8000_0004);
    assert_eq!(cpu.read_csr(CSR_UEPC).unwrap(), 0x40);
    cpu.set_interrupt(4, false).unwrap();
    cpu.execute_system(0x0020_0073, 0, 0, 0).unwrap();
    assert_eq!(cpu.pc(), 0x40);
    assert_eq!(
        cpu.read_csr(CSR_USTATUS).unwrap() & USTATUS_UIE,
        USTATUS_UIE
    );
}

#[test]
fn hazard3_zcb_lhu_loads_a_compact_halfword() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 4096, true).unwrap();
    // c.lhu s1,2(a2)
    bus.load(0, &[0x24, 0x86]).unwrap();
    bus.load(0x102, &[0xcd, 0xab]).unwrap();
    let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
    cpu.set_register(RiscVRegister::A2, 0x100).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(RiscVRegister::S1).unwrap(), 0xabcd);
}

#[test]
fn hazard3_zcb_byte_load_and_store_decode_both_immediate_bits() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 4096, true).unwrap();
    // c.lbu s1,3(a2); c.sb s1,3(a2)
    bus.load(0, &[0x64, 0x82, 0x64, 0x8a]).unwrap();
    bus.load(0x103, &[0xa5]).unwrap();
    let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
    cpu.set_register(RiscVRegister::A2, 0x100).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(RiscVRegister::S1).unwrap(), 0xa5);

    cpu.set_register(RiscVRegister::S1, 0x5a).unwrap();
    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(
        bus.read(
            0x103,
            AccessWidth::Byte,
            remu_core::AccessKind::Read,
            SimTime::ZERO,
        )
        .unwrap(),
        0x5a
    );
}

#[test]
fn hazard3_level_interrupt_cannot_preempt_its_own_handler() {
    let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
    cpu.write_csr(CSR_MSTATUS, MSTATUS_MIE).unwrap();
    cpu.write_csr(CSR_MIE, 1 << 11).unwrap();
    cpu.set_hazard3_external_interrupt(14, true).unwrap();
    cpu.hazard3_irqarray_access(CSR_MEIEA, 1 << 30, 2).unwrap();

    assert_eq!(cpu.pending_interrupt(), Some(11));
    cpu.take_interrupt(11);
    cpu.write_csr(CSR_MSTATUS, MSTATUS_MIE).unwrap();
    assert_eq!(cpu.pending_interrupt(), None);

    cpu.csrs[usize::from(CSR_MSTATUS)] = MSTATUS_MPIE;
    cpu.execute_system(0x3020_0073, 0, 0, 0).unwrap();
    assert_eq!(cpu.pending_interrupt(), Some(11));
}

#[test]
fn qingke_pfic_interrupt_uses_mode_three_vector_table() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 4096, true).unwrap();
    bus.load(38 * 4, &0x0000_0200_u32.to_le_bytes()).unwrap();
    let mut cpu = RiscVCpu::new(RiscVProfile::ch32v003()).unwrap();
    cpu.write_csr(CSR_QINGKE_INTSYSCR, 3).unwrap();
    assert_eq!(cpu.read_csr(CSR_QINGKE_INTSYSCR).unwrap(), 3);
    cpu.write_csr(CSR_MTVEC, 3).unwrap();
    cpu.write_csr(CSR_MSTATUS, MSTATUS_MIE).unwrap();
    cpu.set_qingke_external_interrupt(38, true).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.pc(), 0x200);
    assert_eq!(cpu.read_csr(CSR_MCAUSE).unwrap(), 0x8000_0026);
    assert_eq!(cpu.read_csr(CSR_MEPC).unwrap(), 0);
}

#[test]
fn hazard3_external_irq_is_filtered_by_windowed_enable_array() {
    let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
    cpu.write_csr(CSR_MSTATUS, MSTATUS_MIE).unwrap();
    cpu.write_csr(CSR_MIE, 1 << 11).unwrap();
    cpu.set_hazard3_external_interrupt(14, true).unwrap();

    assert_eq!(cpu.pending_interrupt(), None);
    assert_eq!(
        cpu.hazard3_irqarray_access(CSR_MEIPA, 0, 2).unwrap(),
        1 << 30
    );

    cpu.hazard3_irqarray_access(CSR_MEIEA, 1 << 30, 2).unwrap();
    assert_eq!(cpu.pending_interrupt(), Some(11));
    assert_eq!(
        cpu.hazard3_irqarray_access(CSR_MEIEA, 0, 2).unwrap(),
        1 << 30
    );

    cpu.hazard3_irqarray_access(CSR_MEIEA, 1 << 30, 3).unwrap();
    assert_eq!(cpu.pending_interrupt(), None);
}

#[test]
fn hazard3_meinext_update_publishes_current_irq_context() {
    let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
    cpu.set_hazard3_external_interrupt(14, true).unwrap();
    cpu.hazard3_irqarray_access(CSR_MEIEA, 1 << 30, 2).unwrap();

    assert_eq!(cpu.read_csr(CSR_MEINEXT).unwrap(), 14 * 4);
    cpu.write_csr(CSR_MEINEXT, 1).unwrap();
    let context = cpu.read_csr(CSR_MEICONTEXT).unwrap();
    assert_eq!((context >> 4) & 0x1ff, 14);
    assert_eq!(context & 0x8000, 0);
    assert_eq!((context >> 16) & 0x1f, 1);
}

#[test]
fn esp32c6_level_interrupt_cannot_preempt_its_own_handler() {
    let mut cpu = RiscVCpu::new(RiscVProfile::esp32c6()).unwrap();
    cpu.write_csr(CSR_MSTATUS, MSTATUS_MIE).unwrap();
    cpu.write_csr(CSR_MIE, 1 << 13).unwrap();
    cpu.set_interrupt(13, true).unwrap();

    assert_eq!(cpu.pending_interrupt(), Some(13));
    cpu.take_interrupt(13);
    cpu.write_csr(CSR_MSTATUS, MSTATUS_MIE).unwrap();
    assert_eq!(cpu.pending_interrupt(), None);

    cpu.csrs[usize::from(CSR_MSTATUS)] = MSTATUS_MPIE;
    cpu.execute_system(0x3020_0073, 0, 0, 0).unwrap();
    assert_eq!(cpu.pending_interrupt(), Some(13));
}

#[test]
fn hazard3_irq_array_access_selects_nonzero_windows_atomically() {
    let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
    cpu.set_hazard3_external_interrupt(33, true).unwrap();
    let select_window_two = 2;

    assert_eq!(
        cpu.hazard3_irqarray_access(CSR_MEIPA, select_window_two, 2)
            .unwrap(),
        1 << 17
    );
    cpu.hazard3_irqarray_access(CSR_MEIEA, (1 << 17) | select_window_two, 2)
        .unwrap();
    assert_eq!(
        cpu.hazard3_irqarray_access(CSR_MEIEA, select_window_two, 2)
            .unwrap(),
        1 << 17
    );
}

#[test]
fn hazard3_zcmp_push_and_popret_preserve_return_address() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 4096, true).unwrap();
    // cm.push {ra},-16; cm.popret {ra},16
    bus.load(0, &[0x42, 0xb8, 0x42, 0xbe]).unwrap();
    let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
    cpu.set_register(RiscVRegister::Ra, 0x44).unwrap();
    cpu.set_register(RiscVRegister::Sp, 0x100).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(RiscVRegister::Sp).unwrap(), 0xf0);
    cpu.set_register(RiscVRegister::Ra, 0).unwrap();
    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(RiscVRegister::Ra).unwrap(), 0x44);
    assert_eq!(cpu.register(RiscVRegister::Sp).unwrap(), 0x100);
    assert_eq!(cpu.pc(), 0x44);
}

#[test]
fn compressed_addi_executes_on_qingke_profile() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 64, true).unwrap();
    // c.addi x1, 1; c.ebreak
    bus.load(0, &[0x85, 0x00, 0x02, 0x90]).unwrap();
    let mut cpu = RiscVCpu::new(RiscVProfile::ch32v003()).unwrap();
    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(RiscVRegister::Ra).unwrap(), 1);
    assert_eq!(
        cpu.step(&mut bus, SimTime::ZERO).unwrap().reason,
        StepReason::Halted
    );
}

#[test]
fn qingke_xw_executes_all_eight_compressed_memory_operations() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 4096, true).unwrap();
    // c.sb/c.sh using x8 as the compact base and x9 as data, then the
    // stack-relative forms, followed by the corresponding unsigned loads.
    let instructions = [
        0xbc44_u16, 0xb426, 0x86c4, 0x85e4, 0x3c44, 0x3426, 0x8684, 0x85a4,
    ];
    let bytes = instructions
        .iter()
        .flat_map(|instruction| instruction.to_le_bytes())
        .collect::<Vec<_>>();
    bus.load(0, &bytes).unwrap();
    let mut cpu = RiscVCpu::new(RiscVProfile::ch32v003()).unwrap();
    cpu.set_register(RiscVRegister::S0, 0x100).unwrap();
    cpu.set_register(RiscVRegister::S1, 0xa1b2_c3d4).unwrap();
    cpu.set_register(RiscVRegister::Sp, 0x180).unwrap();

    for _ in 0..4 {
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
    }
    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(RiscVRegister::S1).unwrap(), 0xd4);
    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(RiscVRegister::S1).unwrap(), 0xc3d4);
    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(RiscVRegister::S1).unwrap(), 0xd4);
    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(RiscVRegister::S1).unwrap(), 0xc3d4);
}

#[test]
fn qingke_xw_opcode_is_profile_gated() {
    let (mut cpu, mut bus) = cpu_and_bus(&[0x0000_3c44], RiscVProfile::esp32c6());
    let fault = cpu.step(&mut bus, SimTime::ZERO).unwrap_err();
    assert_eq!(fault.kind, CpuFaultKind::IllegalInstruction);
}

#[test]
fn qingke_v2c_zmmul_accepts_multiply_but_rejects_divide() {
    // mul x3,x1,x2; div x3,x1,x2
    let words = [0x0220_81b3, 0x0220_c1b3];
    let (mut cpu, mut bus) = cpu_and_bus(&words, RiscVProfile::ch32v006());
    cpu.set_register(RiscVRegister::Ra, 6).unwrap();
    cpu.set_register(RiscVRegister::Sp, 7).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(RiscVRegister::Gp).unwrap(), 42);
    let fault = cpu.step(&mut bus, SimTime::ZERO).unwrap_err();
    assert_eq!(fault.kind, CpuFaultKind::IllegalInstruction);

    let (mut v2a, mut v2a_bus) = cpu_and_bus(&words[..1], RiscVProfile::ch32v003());
    v2a.set_register(RiscVRegister::Ra, 6).unwrap();
    v2a.set_register(RiscVRegister::Sp, 7).unwrap();
    assert_eq!(
        v2a.step(&mut v2a_bus, SimTime::ZERO).unwrap_err().kind,
        CpuFaultKind::IllegalInstruction
    );
}
