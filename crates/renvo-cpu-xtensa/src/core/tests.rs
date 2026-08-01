use super::*;
use renvo_bus::AddressSpace;

#[test]
fn executes_compiler_density_sequence() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x1000, true).unwrap();
    // movi.n a8,7; movi.n a9,5; add.n a8,a8,a9; break 0,0
    bus.load(0, &[0x0c, 0x78, 0x0c, 0x59, 0x9a, 0x88, 0x00, 0x40, 0x00])
        .unwrap();
    let mut cpu = XtensaCpu::new();
    cpu.set_direct_state(0x800, 0);
    for tick in 0..3 {
        cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
    }
    assert_eq!(cpu.register(XtensaRegister::A8), 12);
    assert_eq!(
        cpu.step(&mut bus, SimTime::from_ticks(3)).unwrap().reason,
        StepReason::Breakpoint
    );
}

#[test]
fn narrow_nop_does_not_alias_mov() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.registers[0] = 0x1111_1111;
    cpu.registers[3] = 0x3333_3333;

    cpu.execute_narrow(0xf03d, &mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.pc, 2);
    assert_eq!(cpu.registers[3], 0x3333_3333);
}

#[test]
fn bany_branches_only_when_operands_share_a_set_bit() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.registers[9] = 1;
    cpu.registers[4] = 2;

    cpu.execute_wide(0x000d_8947, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.pc, 3);

    cpu.pc = 0;
    cpu.registers[9] = 3;
    cpu.execute_wide(0x000d_8947, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.pc, 17);
}

#[test]
fn special_register_round_trip_covers_vecbase_encoding() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.registers[8] = 0x4037_4000;

    // wsr.vecbase a8; rsr.vecbase a9
    cpu.execute_wide(0x13e7_80, &mut bus, SimTime::ZERO)
        .unwrap();
    cpu.execute_wide(0x03e7_90, &mut bus, SimTime::ZERO)
        .unwrap();

    assert_eq!(cpu.registers[9], 0x4037_4000);
}

#[test]
fn software_interrupt_survives_external_line_poll_until_guest_clear() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.registers[8] = 1;

    // wsr.intset a8; the machine then polls external interrupt line zero.
    cpu.execute_wide(0x13e2_80, &mut bus, SimTime::ZERO)
        .unwrap();
    cpu.set_interrupt(0, false).unwrap();
    assert_eq!(cpu.special_registers[226], 1);

    // wsr.intclear a8
    cpu.execute_wide(0x13e3_80, &mut bus, SimTime::ZERO)
        .unwrap();
    cpu.set_interrupt(0, false).unwrap();
    assert_eq!(cpu.special_registers[226], 0);
}

#[test]
fn s32c1i_returns_old_word_and_only_stores_on_compare() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    bus.load(0x40, &0x1122_3344_u32.to_le_bytes()).unwrap();
    let mut cpu = XtensaCpu::new();
    cpu.registers[8] = 0x40;
    cpu.registers[9] = 0xa5a5_5a5a;
    cpu.special_registers[12] = 0x1122_3344;

    // s32c1i a9,a8,0
    cpu.execute_wide(0x00e8_92, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.registers[9], 0x1122_3344);
    assert_eq!(
        bus.read(0x40, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0xa5a5_5a5a
    );

    cpu.registers[9] = 0xffff_ffff;
    cpu.special_registers[12] = 0;
    cpu.execute_wide(0x00e8_92, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.registers[9], 0xa5a5_5a5a);
    assert_eq!(
        bus.read(0x40, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)
            .unwrap(),
        0xa5a5_5a5a
    );
}

#[test]
fn signed_low_halfword_multiply_accumulates_in_40_bits() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.special_registers[16] = 7;
    cpu.registers[7] = 0x1234_fffe;
    cpu.registers[5] = 0xabcd_0003;

    // mula.aa.ll a7,a5
    cpu.execute_wide(0x7807_54, &mut bus, SimTime::ZERO)
        .unwrap();

    assert_eq!(cpu.special_registers[16], 1);
    assert_eq!(cpu.special_registers[17], 0);

    cpu.special_registers[16] = 0;
    cpu.special_registers[17] = 0;
    cpu.registers[7] = 0xffff;
    cpu.registers[5] = 1;
    cpu.execute_wide(0x7807_54, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.special_registers[16], u32::MAX);
    assert_eq!(cpu.special_registers[17], 0xff);
}

#[test]
fn signed_shift_and_high_multiply_match_lx7_arithmetic() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.registers[2] = 0xffff_ff80;
    cpu.registers[8] = 0x4000_0000;

    // srai a9,a2,31; mulsh a8,a2,a8
    cpu.execute_wide(0x319f_20, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.registers[9], u32::MAX);
    cpu.execute_wide(0xb282_80, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.registers[8], 0xffff_ffe0);
}

#[test]
fn extui_decodes_rri5_destination_source_shift_and_width() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.registers[9] = 0x4058_13ed;
    cpu.registers[6] = 0x0001_0000;

    // extui a6,a9,20,3
    cpu.execute_wide(0x2564_90, &mut bus, SimTime::ZERO)
        .unwrap();

    assert_eq!(cpu.registers[6], 5);
    assert_eq!(cpu.registers[9], 0x4058_13ed);
}

#[test]
fn rsil_does_not_alias_extui_and_returns_previous_ps() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.ps = 0x23;

    // rsil a10,3
    cpu.execute_wide(0x0063_a0, &mut bus, SimTime::ZERO)
        .unwrap();

    assert_eq!(cpu.registers[10], 0x23);
    assert_eq!(cpu.ps & 0xf, 3);
}

#[test]
fn single_precision_coprocessor_preserves_payloads_and_converts_unsigned_values() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x1000, true).unwrap();
    let mut cpu = XtensaCpu::new();
    cpu.registers[8] = 16_777_217;

    // ufloat.s f0,a8,0; rfr a9,f0; ssi f0,a1,0; lsi f1,a1,0
    cpu.execute_wide(0xda08_00, &mut bus, SimTime::ZERO)
        .unwrap();
    cpu.execute_wide(0xfa90_40, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.registers[9], (16_777_217_u32 as f32).to_bits());

    cpu.registers[1] = 0x100;
    cpu.execute_wide(0x0041_03, &mut bus, SimTime::ZERO)
        .unwrap();
    cpu.execute_wide(0x0001_13, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.floating_registers[1], cpu.floating_registers[0]);
}

#[test]
fn const_s_loads_the_four_architectural_constants() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();

    // const.s f3,0; const.s f4,1; const.s f5,2; const.s f6,3
    cpu.execute_wide(0x00fa_3030, &mut bus, SimTime::ZERO)
        .unwrap();
    cpu.execute_wide(0x00fa_4130, &mut bus, SimTime::ZERO)
        .unwrap();
    cpu.execute_wide(0x00fa_5230, &mut bus, SimTime::ZERO)
        .unwrap();
    cpu.execute_wide(0x00fa_6330, &mut bus, SimTime::ZERO)
        .unwrap();

    assert_eq!(cpu.floating_registers[3], 0.0_f32.to_bits());
    assert_eq!(cpu.floating_registers[4], 1.0_f32.to_bits());
    assert_eq!(cpu.floating_registers[5], 2.0_f32.to_bits());
    assert_eq!(cpu.floating_registers[6], 0.5_f32.to_bits());
}

#[test]
fn utrunc_s_converts_and_scales_an_unsigned_value() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.floating_registers[0] = 3.75_f32.to_bits();

    // utrunc.s a7,f0,0
    cpu.execute_wide(0x00ea_7000, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.registers[7], 3);

    // utrunc.s a8,f0,2
    cpu.execute_wide(0x00ea_8020, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.registers[8], 15);
}

#[test]
fn integer_conditioned_float_moves_preserve_or_replace_payloads() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.floating_registers[0] = 0xdead_beef;
    cpu.floating_registers[1] = 0x3f80_0000;
    cpu.registers[10] = 0;

    // moveqz.s f0,f1,a10
    cpu.execute_wide(0x008b_01a0, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.floating_registers[0], 0x3f80_0000);

    cpu.floating_registers[2] = 0xcafe_babe;
    cpu.floating_registers[3] = 0x4000_0000;
    cpu.registers[11] = 0;
    // movnez.s f2,f3,a11
    cpu.execute_wide(0x009b_23b0, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.floating_registers[2], 0xcafe_babe);
}

#[test]
fn call8_window_maps_arguments_and_return_value_back_to_caller() {
    let mut cpu = XtensaCpu::new();
    cpu.pc = 0x1000;
    cpu.registers[1] = 0x3fce_0000;
    cpu.registers[10] = 41;

    cpu.window_call(8, 0x2000, 0x1003);
    assert_eq!(cpu.registers[1], 0x3fce_0000);
    assert_eq!(cpu.registers[2], 41);
    cpu.registers[2] += 1;
    cpu.window_return().unwrap();

    assert_eq!(cpu.pc, 0x1003);
    assert_eq!(cpu.registers[10], 42);
}

#[test]
fn ssa8b_prepares_a_byte_position_for_sll() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.registers[11] = 2;
    cpu.registers[9] = 0x41;

    // ssa8b a11; sll a9,a9
    cpu.execute_wide(0x0040_3b00, &mut bus, SimTime::ZERO)
        .unwrap();
    cpu.execute_wide(0x00a1_9900, &mut bus, SimTime::from_ticks(1))
        .unwrap();

    assert_eq!(cpu.registers[9], 0x0041_0000);
}

#[test]
fn src_with_zero_encoded_sar_selects_the_high_word() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.sar = 0;
    cpu.registers[2] = 0x1111_2222;
    cpu.registers[3] = 0x3333_4444;

    // src a3,a3,a2
    cpu.execute_wide(0x0081_3320, &mut bus, SimTime::ZERO)
        .unwrap();

    assert_eq!(cpu.registers[3], 0x3333_4444);
}

#[test]
fn branch_to_stale_loop_end_does_not_reenter_old_body() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // beqz.n a12,+15 -> address 19
    bus.load(0, &[0x8c, 0xfc]).unwrap();
    let mut cpu = XtensaCpu::new();
    cpu.loop_begin = 4;
    cpu.loop_end = 19;
    cpu.loop_count = 3;
    cpu.registers[12] = 0;

    cpu.step(&mut bus, SimTime::ZERO).unwrap();

    assert_eq!(cpu.pc, 19);
    assert_eq!(cpu.loop_count, 0);
}

#[test]
fn boolean_branches_do_not_alias_zero_overhead_loops() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();

    // bf b0,+2
    cpu.execute_wide(0x0002_0076, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.pc, 6);
    assert_eq!(cpu.loop_count, 0);

    cpu.pc = 0;
    cpu.boolean_registers = 1 << 3;
    // bt b3,-1
    cpu.execute_wide(0x00ff_1376, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.pc, 3);
    assert_eq!(cpu.loop_count, 0);
}

#[test]
fn conditional_zero_overhead_loops_skip_non_positive_counts() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();

    cpu.registers[5] = 0;
    // loopnez a5,+1
    cpu.execute_wide(0x0001_9576, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.pc, 5);
    assert_eq!(cpu.loop_count, 0);

    cpu.pc = 0;
    cpu.registers[6] = u32::MAX;
    // loopgtz a6,+1
    cpu.execute_wide(0x0001_a676, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.pc, 5);
    assert_eq!(cpu.loop_count, 0);
}

#[test]
fn rsync_does_not_alias_movsp_or_modify_general_registers() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.registers[0] = 0xc202_d350;
    cpu.registers[2] = 0x3fcb_36d8;

    // rsync
    cpu.execute_wide(0x0000_2010, &mut bus, SimTime::ZERO)
        .unwrap();

    assert_eq!(cpu.registers[2], 0x3fcb_36d8);
    assert_eq!(cpu.pc, 3);
}

#[test]
fn entry_rotates_a_fresh_freertos_call4_task_frame() {
    let mut bus = AddressSpace::default();
    let mut cpu = XtensaCpu::new();
    cpu.registers[1] = 0x3fca_1000;
    cpu.registers[6] = 0x4200_1234;
    cpu.registers[7] = 0x3fc6_abcd;
    cpu.ps = 1 << 16;

    // entry a1, 32
    cpu.execute_wide(0x0041_36, &mut bus, SimTime::ZERO)
        .unwrap();

    assert_eq!(cpu.registers[1], 0x3fca_0fe0);
    assert_eq!(cpu.registers[2], 0x4200_1234);
    assert_eq!(cpu.registers[3], 0x3fc6_abcd);
    assert_eq!(cpu.ps & (3 << 16), 0);
}

#[test]
fn freertos_task_context_can_migrate_between_cores() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x1000, true).unwrap();
    // rfe
    bus.load(0, &[0x00, 0x30, 0x00]).unwrap();

    let mut cpu0 = XtensaCpu::new();
    let mut cpu1 = XtensaCpu::new();
    cpu1.share_task_contexts_from(&cpu0);
    cpu0.set_direct_state(0x800, 0x100);
    cpu1.set_direct_state(0x700, 0);

    let task = 0x1234;
    cpu0.thread_pointer = task;
    cpu0.registers[2] = 0xfeed_beef;
    cpu0.special_registers[226] = 1;
    cpu0.special_registers[228] = 1;
    cpu0.step(&mut bus, SimTime::ZERO).unwrap();

    cpu1.thread_pointer = task;
    cpu1.special_registers[177] = 0x80;
    cpu1.step(&mut bus, SimTime::from_ticks(1)).unwrap();

    assert_eq!(cpu1.register(XtensaRegister::A2), 0xfeed_beef);
    assert_eq!(cpu1.pc(), 0x80);
}

#[test]
fn threadptr_switch_restores_window_stack_saved_by_voluntary_yield() {
    let mut bus = AddressSpace::default();
    bus.map_ram("stack", 0, 0x1000, false).unwrap();
    let mut cpu = XtensaCpu::new();

    cpu.thread_pointer = 1;
    cpu.window_call(4, 0x100, 0x80);
    cpu.registers[4] = 0x1111_1111;
    // rur.threadptr a2
    cpu.execute_wide(0x00e3_2e70, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.register(XtensaRegister::A2), 1);

    // The interrupt vector executes RUR after the architectural entry
    // path has cleared the active logical windows. It must not replace
    // the solicited-yield snapshot captured above.
    cpu.ps = 0x10;
    cpu.window_stack.clear();
    cpu.execute_wide(0x00e3_2e70, &mut bus, SimTime::ZERO)
        .unwrap();
    cpu.ps = 0;

    cpu.thread_pointer = 2;
    cpu.window_stack.clear();
    cpu.window_call(4, 0x200, 0x180);
    cpu.window_call(4, 0x300, 0x280);
    cpu.registers[4] = 0x2222_2222;
    cpu.execute_wide(0x00e3_2e70, &mut bus, SimTime::ZERO)
        .unwrap();

    cpu.registers[3] = 1;
    // wur.threadptr a3
    cpu.execute_wide(0x00f3_e730, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.window_stack.len(), 1);
    assert_eq!(cpu.registers[4], 0x1111_1111);

    cpu.registers[3] = 2;
    cpu.execute_wide(0x00f3_e730, &mut bus, SimTime::ZERO)
        .unwrap();
    assert_eq!(cpu.window_stack.len(), 2);
    assert_eq!(cpu.registers[4], 0x2222_2222);
}

#[test]
fn entry_materializes_caller_roots_in_the_reserved_spill_area() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x1000, true).unwrap();
    let mut cpu = XtensaCpu::new();
    cpu.registers[1] = 0x800;
    cpu.registers[2] = 0x1111_1111;
    cpu.registers[3] = 0x2222_2222;
    cpu.registers[4] = 0x3333_3333;
    cpu.registers[5] = 0x4444_4444;
    cpu.window_call(4, 0x100, 0x80);

    // entry a1, 32
    cpu.execute_wide(0x0041_36, &mut bus, SimTime::ZERO)
        .unwrap();

    for (index, expected) in [0x1111_1111_u64, 0x2222_2222, 0x4000_0080, 0x4444_4444]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            bus.read(
                0x7f0 + (index as u64) * 4,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
            expected
        );
    }
}

#[test]
fn ccount_advances_with_functional_instruction_ticks() {
    let mut bus = AddressSpace::default();
    bus.map_ram("memory", 0, 0x100, true).unwrap();
    // nop.n; nop.n
    bus.load(0, &[0x3d, 0xf0, 0x3d, 0xf0]).unwrap();
    let mut cpu = XtensaCpu::new();
    cpu.set_direct_state(0x80, 0);

    cpu.step(&mut bus, SimTime::ZERO).unwrap();
    cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

    assert_eq!(cpu.special_registers[234], 2);
}
