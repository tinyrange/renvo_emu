use super::*;
use remu_bus::AddressSpace;

#[test]
fn executes_thumb_arithmetic_and_halts() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x2000, true).unwrap();
    // movs r0,#7; adds r0,#5; bkpt #0
    bus.load(0, &[0x07, 0x20, 0x05, 0x30, 0x00, 0xbe]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
    cpu.set_direct_state(0x1000, 1).unwrap();
    for tick in 0..2 {
        cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
    }
    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 12);
    assert_eq!(
        cpu.step(&mut bus, SimTime::from_ticks(2)).unwrap().reason,
        StepReason::Breakpoint
    );
}

#[test]
fn it_conditionally_skips_without_becoming_wfi() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // movs r0,#2; cmp r0,#1; it cc; movcc r1,#7; movs r2,#9
    bus.load(
        0,
        &[0x02, 0x20, 0x01, 0x28, 0x38, 0xbf, 0x07, 0x21, 0x09, 0x22],
    )
    .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();

    for tick in 0..5 {
        cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
    }

    assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 0);
    assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 9);
    assert!(!cpu.waiting);
}

#[test]
fn push_and_pop_restore_a_register() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x2000, true).unwrap();
    // push {r0}; movs r0,#0; pop {r0}
    bus.load(0, &[0x01, 0xb4, 0x00, 0x20, 0x01, 0xbc]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
    cpu.set_direct_state(0x1000, 1).unwrap();
    cpu.registers[0] = 42;
    for tick in 0..3 {
        cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
    }
    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 42);
    assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x1000);
}

#[test]
fn ldmia_preserves_a_loaded_base_register() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x2000, true).unwrap();
    // ldmia r0, {r0, r1}
    bus.load(0, &[0x03, 0xc8]).unwrap();
    bus.load(0x100, &0x1234_5678_u32.to_le_bytes()).unwrap();
    bus.load(0x104, &0x9abc_def0_u32.to_le_bytes()).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
    cpu.set_direct_state(0x1000, 1).unwrap();
    cpu.registers[0] = 0x100;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 0x1234_5678);
    assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 0x9abc_def0);
}

#[test]
fn cbz_and_cbnz_branch_from_the_prefetched_pc() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // cbz r0,+4; movs r1,#1; movs r1,#2; cbnz r0,+4
    bus.load(0, &[0x10, 0xb1, 0x01, 0x21, 0x02, 0x21, 0x10, 0xb9])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 8);

    cpu.registers[15] = 6;
    cpu.registers[0] = 1;
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();
    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 14);
}

#[test]
fn thumb2_modified_immediate_subtracts_from_sp() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // sub.w sp,sp,#256
    bus.load(0, &[0xad, 0xf5, 0x80, 0x7d]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x2000, 1).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x1f00);
    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 4);
}

#[test]
fn thumb2_tst_and_bic_modified_immediates() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // tst.w r0,#2; bic.w r1,r1,#1
    bus.load(0, &[0x10, 0xf0, 0x02, 0x0f, 0x31, 0xf0, 0x01, 0x01])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[0] = 2;
    cpu.registers[1] = 3;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.xpsr & Z, 0);
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();
    assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 2);
}

#[test]
fn thumb2_movw_and_movt_form_a_constant() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // movw r0,#0xa0eb; movt r0,#0x1234
    bus.load(0, &[0x4a, 0xf2, 0xeb, 0x00, 0xc1, 0xf2, 0x34, 0x20])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 0x1234_a0eb);
}

#[test]
fn thumb2_subw_uses_the_unexpanded_twelve_bit_immediate() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // subw r2,r2,#0x8cb
    bus.load(0, &[0xa2, 0xf6, 0xcb, 0x02]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[2] = 0x1000;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0x735);
}

#[test]
fn thumb2_signed_halfword_multiply_accumulates() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // smlabb r0,r9,r0,ip
    bus.load(0, &[0x19, 0xfb, 0x00, 0xc0]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[0] = 3;
    cpu.registers[9] = 0x0000_fffe;
    cpu.registers[12] = 10;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 4);
}

#[test]
fn rp2350_gpio_coprocessor_routes_single_bit_output_and_enable_to_sio() {
    let mut bus = AddressSpace::default();
    bus.map_ram("code", 0, 0x100, true).unwrap();
    bus.map_ram("sio", 0xd000_0000, 0x200, false).unwrap();
    // mcrr p0,#4,r0,r6,c0; mcrr p0,#4,r0,r3,c4
    bus.load(0, &[0x46, 0xec, 0x40, 0x00, 0x43, 0xec, 0x44, 0x00])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[0] = 3;
    cpu.registers[6] = 1;
    cpu.registers[3] = 1;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

    assert_eq!(
        bus.read(
            0xd000_0018,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        )
        .unwrap(),
        1 << 3
    );
    assert_eq!(
        bus.read(
            0xd000_0038,
            AccessWidth::Word,
            AccessKind::Read,
            SimTime::ZERO
        )
        .unwrap(),
        1 << 3
    );
}

#[test]
fn rp2350_gpio_coprocessor_reads_low_input_bank() {
    let mut bus = AddressSpace::default();
    bus.map_ram("code", 0, 0x100, true).unwrap();
    bus.map_ram("sio", 0xd000_0000, 0x200, false).unwrap();
    // mrc p0,#0,r2,c0,c8
    bus.load(0, &[0x10, 0xee, 0x18, 0x20]).unwrap();
    bus.load(0xd000_0004, &0x5a5a_a5a5_u32.to_le_bytes())
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0x5a5a_a5a5);
}

#[test]
fn thumb2_post_indexed_load_updates_its_base() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x200, true).unwrap();
    // ldr.w r3,[r4],#4
    bus.load(0, &[0x54, 0xf8, 0x04, 0x3b]).unwrap();
    bus.load(0x100, &0x1234_5678_u32.to_le_bytes()).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x1f0, 1).unwrap();
    cpu.registers[4] = 0x100;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 0x1234_5678);
    assert_eq!(cpu.register(ArmRegister::R4).unwrap(), 0x104);
}

#[test]
fn thumb2_signed_byte_load_sign_extends() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x200, true).unwrap();
    // ldrsb.w r0,[r2]
    bus.load(0, &[0x92, 0xf9, 0x00, 0x00]).unwrap();
    bus.load(0x100, &[0x80]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x1f0, 1).unwrap();
    cpu.registers[2] = 0x100;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 0xffff_ff80);
}

#[test]
fn thumb2_literal_load_to_pc_follows_a_veneer() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x200, true).unwrap();
    // ldr.w pc,[pc]; literal is at pc+4
    bus.load(0, &[0x5f, 0xf8, 0x00, 0xf0, 0x81, 0x00, 0x00, 0x00])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x100, 1).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 0x80);
}

#[test]
fn thumb2_strd_predecrements_and_ldrd_restores_the_pair() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x200, true).unwrap();
    // strd ip,lr,[sp,#-16]!; ldrd r2,r3,[sp,#8]
    bus.load(0, &[0x6d, 0xe9, 0x04, 0xce, 0xdd, 0xe9, 0x02, 0x23])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x100, 1).unwrap();
    cpu.registers[12] = 0x1234_5678;
    cpu.registers[14] = 0x9abc_def0;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0xf0);
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

    assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0);
    assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 0);
    assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0xf0);
}

#[test]
fn armv8m_tt_reports_the_functional_nonsecure_address_space() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // tt r2,r2
    bus.load(0, &[0x42, 0xe8, 0x00, 0xf2]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[2] = 0x1000_0000;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0);
}

#[test]
fn armv8m_store_release_byte_has_ordered_store_effects() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // stlb r2,[r1]
    bus.load(0, &[0xc1, 0xe8, 0x8f, 0x2f]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[1] = 0x40;
    cpu.registers[2] = 0x1234_56ab;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(
        bus.read(0x40, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0xab
    );
}

#[test]
fn armv8m_byte_exclusive_pair_succeeds_on_one_core() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // ldaexb r1,[r3]; strexb r1,r2,[r3]
    bus.load(0, &[0xd3, 0xe8, 0xcf, 0x1f, 0xc3, 0xe8, 0x41, 0x2f])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[3] = 0x40;
    cpu.registers[2] = 0xab;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

    assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 0);
    assert_eq!(
        bus.read(0x40, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0xab
    );
}

#[test]
fn thumb2_table_branch_byte_indexes_from_prefetched_pc() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // tbb [pc,r3]; table bytes 1,3
    bus.load(0, &[0xdf, 0xe8, 0x03, 0xf0, 1, 3]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[3] = 1;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 10);
}

#[test]
fn cortex_m33_vstr_and_vldr_preserve_a_double_register() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x200, true).unwrap();
    // vstr d8,[r0,#48]; vldr d9,[r0,#48]
    bus.load(0, &[0x80, 0xed, 0x0c, 0x8b, 0x90, 0xed, 0x0c, 0x9b])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x1f0, 1).unwrap();
    cpu.registers[0] = 0x80;
    cpu.fpu_registers[8] = 0x1234_5678_9abc_def0;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

    assert_eq!(cpu.fpu_registers[9], 0x1234_5678_9abc_def0);
}

#[test]
fn cortex_m4f_is_distinct_and_executes_fpv4_load_store() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x200, true).unwrap();
    // vstr d8,[r0,#48]; vldr d9,[r0,#48]
    bus.load(0, &[0x80, 0xed, 0x0c, 0x8b, 0x90, 0xed, 0x0c, 0x9b])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM4F);
    assert_eq!(cpu.profile(), ArmProfile::CortexM4F);
    assert_eq!(cpu.profile().name(), "cortex-m4f-armv7em");
    cpu.set_direct_state(0x1f0, 1).unwrap();
    cpu.registers[0] = 0x80;
    cpu.fpu_registers[8] = 0x1234_5678_9abc_def0;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

    assert_eq!(cpu.fpu_registers[9], 0x1234_5678_9abc_def0);
}

#[test]
fn cortex_m4f_converts_single_precision_to_signed_fixed_point() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // vcvt.s32.f32 s15,s15,#2; vmov r3,s15
    bus.load(0, &[0xfe, 0xee, 0xcf, 0x7a, 0x17, 0xee, 0x90, 0x3a])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM4F);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.set_single_register(15, 6.5_f32.to_bits());

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

    assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 26);
}

#[test]
fn cortex_m33_vpush_and_vpop_round_trip_a_double_register() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x200, true).unwrap();
    // vpush {d8}; vpop {d8}
    bus.load(0, &[0x2d, 0xed, 0x02, 0x8b, 0xbd, 0xec, 0x02, 0x8b])
        .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x180, 1).unwrap();
    cpu.fpu_registers[8] = 0x1234_5678_9abc_def0;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x178);
    cpu.fpu_registers[8] = 0;
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

    assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x180);
    assert_eq!(cpu.fpu_registers[8], 0x1234_5678_9abc_def0);
}

#[test]
fn thumb2_ldmia_restores_high_registers_and_writes_back() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x200, true).unwrap();
    // ldmia.w sp!,{r4,lr}
    bus.load(0, &[0xbd, 0xe8, 0x10, 0x40]).unwrap();
    bus.load(0x100, &0x1234_u32.to_le_bytes()).unwrap();
    bus.load(0x104, &0x81_u32.to_le_bytes()).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x100, 1).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R4).unwrap(), 0x1234);
    assert_eq!(cpu.register(ArmRegister::Lr).unwrap(), 0x81);
    assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x108);
}

#[test]
fn thumb2_bics_shifted_register_updates_flags() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // bics.w r2,r3,r2
    bus.load(0, &[0x33, 0xea, 0x02, 0x02]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[2] = 0x0f;
    cpu.registers[3] = 0xff;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0xf0);
    assert_eq!(cpu.xpsr & Z, 0);
}

#[test]
fn thumb2_mov_shifted_register_alias_does_not_use_pc() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // mov.w r8,r3,lsl #2 (ORR with Rn=PC)
    bus.load(0, &[0x4f, 0xea, 0x83, 0x08]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[3] = 5;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R8).unwrap(), 20);
}

#[test]
fn thumb2_unconditional_wide_branch_does_not_link() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // b.w from 0 to 0x20
    bus.load(0, &[0x00, 0xf0, 0x0e, 0xb8]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[14] = 0x55;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 0x20);
    assert_eq!(cpu.register(ArmRegister::Lr).unwrap(), 0x55);
}

#[test]
fn thumb2_conditional_wide_branch_uses_the_current_flags() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x400, true).unwrap();
    // bne.w +416
    bus.load(0, &[0x40, 0xf0, 0xd0, 0x80]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x300, 1).unwrap();

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 420);
}

#[test]
fn thumb2_unsigned_division_matches_cortex_m33() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // udiv r4,r4,r1
    bus.load(0, &[0xb4, 0xfb, 0xf1, 0xf4]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[4] = 100;
    cpu.registers[1] = 6;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R4).unwrap(), 16);
}

#[test]
fn thumb2_unsigned_bitfield_extracts_the_requested_width() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // ubfx r4,r4,#0,#12
    bus.load(0, &[0xc4, 0xf3, 0x0b, 0x04]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[4] = 0x1234_5abc;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R4).unwrap(), 0xabc);
}

#[test]
fn thumb2_bitfield_insert_replaces_only_the_field() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // bfi r3,r0,#0,#1
    bus.load(0, &[0x60, 0xf3, 0x00, 0x03]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[0] = 1;
    cpu.registers[3] = 0xffff_fffe;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R3).unwrap(), u32::MAX);
}

#[test]
fn thumb2_clz_counts_all_leading_zeroes() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // clz r7,r2
    bus.load(0, &[0xb2, 0xfa, 0x82, 0xf7]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[2] = 0x0000_0800;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R7).unwrap(), 20);
}

#[test]
fn thumb2_register_controlled_shift_uses_low_byte_of_amount() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // lsl.w ip,lr,r7
    bus.load(0, &[0x0e, 0xfa, 0x07, 0xfc]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[14] = 3;
    cpu.registers[7] = 4;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R12).unwrap(), 48);
}

#[test]
fn thumb2_uxtb_accepts_a_high_source_register() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // uxtb.w r7,r11
    bus.load(0, &[0x5f, 0xfa, 0x8b, 0xf7]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[11] = 0x1234_56ab;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R7).unwrap(), 0xab);
}

#[test]
fn thumb2_uxtah_extends_then_adds() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // uxtah r5,r7,r5
    bus.load(0, &[0x17, 0xfa, 0x85, 0xf5]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[5] = 0x1234_ffff;
    cpu.registers[7] = 2;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R5).unwrap(), 0x1_0001);
}

#[test]
fn thumb2_mls_subtracts_a_product_from_the_accumulator() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // mls ip,lr,r8,ip
    bus.load(0, &[0x0e, 0xfb, 0x18, 0xcc]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[14] = 4;
    cpu.registers[8] = 5;
    cpu.registers[12] = 100;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R12).unwrap(), 80);
}

#[test]
fn thumb2_umull_writes_both_halves() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // umull r3,r1,r3,r2
    bus.load(0, &[0xa3, 0xfb, 0x02, 0x31]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[3] = u32::MAX;
    cpu.registers[2] = 2;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 0xffff_fffe);
    assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 1);
}

#[test]
fn thumb2_umlal_accumulates_into_both_halves() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // umlal r2,r3,r4,r1
    bus.load(0, &[0xe4, 0xfb, 0x01, 0x23]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[1] = 3;
    cpu.registers[2] = u32::MAX;
    cpu.registers[3] = 1;
    cpu.registers[4] = 2;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 5);
    assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 2);
}

#[test]
fn thumb2_subtract_shifted_register_handles_high_registers() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // sub.w r3,r3,r9
    bus.load(0, &[0xa3, 0xeb, 0x09, 0x03]).unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x80, 1).unwrap();
    cpu.registers[3] = 100;
    cpu.registers[9] = 23;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 77);
}

#[test]
fn official_rp2350_strlen_sequence_returns_a_plain_length() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x400, true).unwrap();
    bus.load(
        0,
        &[
            0x20, 0xf0, 0x03, 0x01, 0x10, 0xf0, 0x03, 0x00, 0xc0, 0xf1, 0x00, 0x00, 0x51, 0xf8,
            0x04, 0x3b, 0x00, 0xf1, 0x04, 0x0c, 0x4f, 0xea, 0xcc, 0x0c, 0x6f, 0xf0, 0x00, 0x02,
            0x1c, 0xbf, 0x22, 0xfa, 0x0c, 0xf2, 0x13, 0x43, 0x4f, 0xf0, 0x01, 0x0c, 0x4c, 0xea,
            0x0c, 0x2c, 0x4c, 0xea, 0x0c, 0x4c, 0xa3, 0xeb, 0x0c, 0x02, 0x22, 0xea, 0x03, 0x02,
            0x12, 0xea, 0xcc, 0x12, 0x04, 0xbf, 0x51, 0xf8, 0x04, 0x3b, 0x04, 0x30, 0xf4, 0xd0,
            0xc2, 0xf1, 0x00, 0x01, 0x02, 0xea, 0x01, 0x02, 0xb2, 0xfa, 0x82, 0xf2, 0xc2, 0xf1,
            0x1f, 0x02, 0x00, 0xeb, 0xd2, 0x00, 0x70, 0x47,
        ],
    )
    .unwrap();
    bus.load(0x100, b"rp2.py\0").unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_direct_state(0x300, 1).unwrap();
    cpu.registers[0] = 0x100;
    cpu.registers[14] = 0x201;

    for tick in 0..100 {
        if cpu.registers[15] == 0x200 {
            break;
        }
        cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
    }

    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 0x200);
    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 6);
}

#[test]
fn architectural_barriers_advance_after_synchronous_bus_work() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // dmb sy; dsb sy; isb sy
    bus.load(
        0,
        &[
            0xbf, 0xf3, 0x5f, 0x8f, 0xbf, 0xf3, 0x4f, 0x8f, 0xbf, 0xf3, 0x6f, 0x8f,
        ],
    )
    .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
    cpu.set_direct_state(0x80, 1).unwrap();
    for tick in 0..3 {
        cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
    }
    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 12);
}

#[test]
fn primask_moves_and_cps_are_observable() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // mrs r0,primask; cpsid i; mrs r1,primask; msr primask,r0; mrs r2,primask
    bus.load(
        0,
        &[
            0xef, 0xf3, 0x10, 0x80, 0x72, 0xb6, 0xef, 0xf3, 0x10, 0x81, 0x80, 0xf3, 0x10, 0x88,
            0xef, 0xf3, 0x10, 0x82,
        ],
    )
    .unwrap();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
    cpu.set_direct_state(0x80, 1).unwrap();
    for tick in 0..5 {
        cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
    }
    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 0);
    assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 1);
    assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0);
}

#[test]
fn high_add_to_pc_branches_using_the_prefetched_pc() {
    let mut bus = AddressSpace::default();
    let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
    cpu.registers[15] = 0x100;
    cpu.registers[1] = 6;

    // add pc, r1
    cpu.execute(0x448f, &mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.registers[15], 0x10a);
}

#[test]
fn external_interrupt_stacks_and_returns() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x1000, true).unwrap();
    bus.load(16 * 4, &0x121_u32.to_le_bytes()).unwrap();
    bus.load(0x100, &[0x30, 0xbf, 0x00, 0xbe]).unwrap(); // wfi; bkpt
    bus.load(0x120, &[0x2a, 0x20, 0x70, 0x47]).unwrap(); // movs r0,#42; bx lr
    let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
    cpu.set_vector_base(0);
    cpu.set_direct_state(0x800, 0x101).unwrap();
    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    cpu.set_interrupt(0, true).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(2)).unwrap();
    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 42);
    cpu.set_interrupt(0, false).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(3)).unwrap();
    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 0);
    assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x800);
    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 0x102);
}

#[test]
fn systick_uses_exception_vector_fifteen_and_returns() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x1000, true).unwrap();
    bus.load(15 * 4, &0x121_u32.to_le_bytes()).unwrap();
    bus.load(0x100, &[0x30, 0xbf, 0x00, 0xbe]).unwrap(); // wfi; bkpt
    bus.load(0x120, &[0x2a, 0x20, 0x70, 0x47]).unwrap(); // movs r0,#42; bx lr
    let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
    cpu.set_vector_base(0);
    cpu.set_direct_state(0x800, 0x101).unwrap();
    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    cpu.set_systick_interrupt(true);
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(2)).unwrap();
    assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 42);
    cpu.step(&mut bus, SimTime::from_ticks(3)).unwrap();
    assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x800);
    assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 0x102);
}
