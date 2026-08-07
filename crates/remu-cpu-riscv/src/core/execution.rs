use super::*;

impl RiscVCpu {
    pub(super) fn execute32(
        &mut self,
        instruction: u32,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let opcode = instruction & 0x7f;
        let rd = ((instruction >> 7) & 0x1f) as u8;
        let funct3 = (instruction >> 12) & 0x7;
        let rs1 = ((instruction >> 15) & 0x1f) as u8;
        let rs2 = ((instruction >> 20) & 0x1f) as u8;
        let funct7 = instruction >> 25;
        let current_pc = self.pc;
        let mut next_pc = self.pc.wrapping_add(4);
        let mut reason = StepReason::Advanced;

        match opcode {
            0x37 => self.write_register(rd, instruction & 0xffff_f000), // LUI
            0x17 => self.write_register(rd, current_pc.wrapping_add(instruction & 0xffff_f000)),
            0x6f => {
                self.check_register(rd)?;
                let immediate = decode_j_immediate(instruction);
                self.write_register(rd, next_pc);
                next_pc = current_pc.wrapping_add_signed(immediate);
            }
            0x67 if funct3 == 0 => {
                let base = self.read_register(rs1)?;
                self.check_register(rd)?;
                let target = base.wrapping_add_signed(sign_extend(instruction >> 20, 12)) & !1;
                self.write_register(rd, next_pc);
                next_pc = target;
            }
            0x63 => {
                let left = self.read_register(rs1)?;
                let right = self.read_register(rs2)?;
                let branch = match funct3 {
                    0 => left == right,
                    1 => left != right,
                    4 => (left as i32) < (right as i32),
                    5 => (left as i32) >= (right as i32),
                    6 => left < right,
                    7 => left >= right,
                    _ => return self.illegal(instruction),
                };
                if branch {
                    next_pc = current_pc.wrapping_add_signed(decode_b_immediate(instruction));
                }
            }
            0x03 => {
                let base = self.read_register(rs1)?;
                self.check_register(rd)?;
                let address = base.wrapping_add_signed(sign_extend(instruction >> 20, 12));
                let value = match funct3 {
                    0 => sign_extend(self.load(bus, address, AccessWidth::Byte, now)?, 8) as u32,
                    1 => {
                        sign_extend(self.load(bus, address, AccessWidth::HalfWord, now)?, 16) as u32
                    }
                    2 => self.load(bus, address, AccessWidth::Word, now)?,
                    4 => self.load(bus, address, AccessWidth::Byte, now)?,
                    5 => self.load(bus, address, AccessWidth::HalfWord, now)?,
                    _ => return self.illegal(instruction),
                };
                self.write_register(rd, value);
            }
            0x23 => {
                let base = self.read_register(rs1)?;
                let value = self.read_register(rs2)?;
                let immediate =
                    sign_extend(((instruction >> 25) << 5) | ((instruction >> 7) & 0x1f), 12);
                let address = base.wrapping_add_signed(immediate);
                let width = match funct3 {
                    0 => AccessWidth::Byte,
                    1 => AccessWidth::HalfWord,
                    2 => AccessWidth::Word,
                    _ => return self.illegal(instruction),
                };
                self.store(bus, address, width, value, now)?;
            }
            0x13 => {
                let left = self.read_register(rs1)?;
                self.check_register(rd)?;
                let immediate = sign_extend(instruction >> 20, 12);
                let bitmanip = self
                    .profile
                    .extension_b
                    .then(|| execute_b_immediate(instruction, funct3, left, rs2))
                    .flatten();
                let value = if let Some(value) = bitmanip {
                    value
                } else {
                    match funct3 {
                        0 => left.wrapping_add_signed(immediate),
                        2 => u32::from((left as i32) < immediate),
                        3 => u32::from(left < immediate as u32),
                        4 => left ^ immediate as u32,
                        6 => left | immediate as u32,
                        7 => left & immediate as u32,
                        1 if funct7 == 0 => left << (rs2 & 0x1f),
                        5 if funct7 == 0 => left >> (rs2 & 0x1f),
                        5 if funct7 == 0x20 => ((left as i32) >> (rs2 & 0x1f)) as u32,
                        _ => return self.illegal(instruction),
                    }
                };
                self.write_register(rd, value);
            }
            0x33 => {
                let left = self.read_register(rs1)?;
                let right = self.read_register(rs2)?;
                self.check_register(rd)?;
                let value = if self.profile.extension_b {
                    if let Some(value) = execute_b_register(funct7, funct3, left, right) {
                        value
                    } else if funct7 == 1 {
                        if !self.profile.supports_m_operation(funct3) {
                            return self.illegal(instruction);
                        }
                        execute_m(funct3, left, right).ok_or_else(|| {
                            CpuFault::new(
                                CpuFaultKind::IllegalInstruction,
                                self.pc.into(),
                                format!("illegal M-extension instruction {instruction:#010x}"),
                            )
                        })?
                    } else {
                        match (funct7, funct3) {
                            (0x00, 0) => left.wrapping_add(right),
                            (0x20, 0) => left.wrapping_sub(right),
                            (0x00, 1) => left << (right & 0x1f),
                            (0x00, 2) => u32::from((left as i32) < (right as i32)),
                            (0x00, 3) => u32::from(left < right),
                            (0x00, 4) => left ^ right,
                            (0x00, 5) => left >> (right & 0x1f),
                            (0x20, 5) => ((left as i32) >> (right & 0x1f)) as u32,
                            (0x00, 6) => left | right,
                            (0x00, 7) => left & right,
                            _ => return self.illegal(instruction),
                        }
                    }
                } else if funct7 == 1 {
                    if !self.profile.supports_m_operation(funct3) {
                        return self.illegal(instruction);
                    }
                    execute_m(funct3, left, right).ok_or_else(|| {
                        CpuFault::new(
                            CpuFaultKind::IllegalInstruction,
                            self.pc.into(),
                            format!("illegal M-extension instruction {instruction:#010x}"),
                        )
                    })?
                } else {
                    match (funct7, funct3) {
                        (0x00, 0) => left.wrapping_add(right),
                        (0x20, 0) => left.wrapping_sub(right),
                        (0x00, 1) => left << (right & 0x1f),
                        (0x00, 2) => u32::from((left as i32) < (right as i32)),
                        (0x00, 3) => u32::from(left < right),
                        (0x00, 4) => left ^ right,
                        (0x00, 5) => left >> (right & 0x1f),
                        (0x20, 5) => ((left as i32) >> (right & 0x1f)) as u32,
                        (0x00, 6) => left | right,
                        (0x00, 7) => left & right,
                        _ => return self.illegal(instruction),
                    }
                };
                self.write_register(rd, value);
            }
            0x0f => {} // FENCE/FENCE.I are ordering no-ops in the functional model.
            0x73 => {
                let system_pc = self.pc;
                reason = self.execute_system(instruction, rd, rs1, funct3)?;
                if reason == StepReason::WaitForInterrupt {
                    self.waiting = true;
                } else if reason == StepReason::Halted {
                    self.halted = true;
                }
                if self.pc != system_pc {
                    next_pc = self.pc;
                }
            }
            0x2f if funct3 == 2 && self.profile.extension_a => {
                self.execute_atomic(instruction, rd, rs1, rs2, bus, now)?;
            }
            _ => return self.illegal(instruction),
        }

        if next_pc & if self.profile.extension_c { 1 } else { 3 } != 0 {
            return Err(CpuFault::new(
                CpuFaultKind::Architecture,
                current_pc.into(),
                format!("misaligned instruction target {next_pc:#010x}"),
            ));
        }
        self.pc = next_pc;
        Ok(reason)
    }

    pub(super) fn execute_system(
        &mut self,
        instruction: u32,
        rd: u8,
        rs1: u8,
        funct3: u32,
    ) -> Result<StepReason, CpuFault> {
        if funct3 == 0 {
            return match instruction {
                0x0000_0073 => {
                    let cause = if self.privilege == RiscVPrivilege::User {
                        8
                    } else {
                        11
                    };
                    self.enter_trap(cause, 0, false);
                    Ok(StepReason::Advanced)
                }
                0x0010_0073 if self.profile.ebreak_halts => Ok(StepReason::Halted),
                0x0010_0073 => Ok(StepReason::Breakpoint),
                0x1050_0073 => Ok(StepReason::WaitForInterrupt),
                0x3020_0073 => {
                    if self.privilege != RiscVPrivilege::Machine {
                        self.enter_trap(2, instruction, false);
                        return Ok(StepReason::Advanced);
                    }
                    let status = self.csrs[usize::from(CSR_MSTATUS)];
                    let restored_ie = (status & MSTATUS_MPIE) >> 4;
                    let return_privilege = match (status & MSTATUS_MPP) >> 11 {
                        0 if self.profile.user_mode => RiscVPrivilege::User,
                        _ => RiscVPrivilege::Machine,
                    };
                    self.csrs[usize::from(CSR_MSTATUS)] =
                        ((status | MSTATUS_MPIE) & !(MSTATUS_MIE | MSTATUS_MPP)) | restored_ie;
                    self.privilege = return_privilege;
                    if self.profile.interrupt_model == InterruptModel::Hazard3
                        && self.csrs[0xbe5] & 1 != 0
                    {
                        self.hazard3_external_active = false;
                        self.csrs[0xbe5] &= !1;
                    } else if self.profile.esp32c6_memory_protection_csrs {
                        self.esp32c6_active_interrupts.pop();
                    }
                    self.pc = self.csrs[usize::from(CSR_MEPC)];
                    Ok(StepReason::Advanced)
                }
                0x0020_0073 if self.profile.user_mode => {
                    let status = self.csrs[usize::from(CSR_USTATUS)];
                    let restored_ie = (status & USTATUS_UPIE) >> 4;
                    self.csrs[usize::from(CSR_USTATUS)] =
                        ((status | USTATUS_UPIE) & !USTATUS_UIE) | restored_ie;
                    self.privilege = RiscVPrivilege::User;
                    if self.profile.esp32c6_memory_protection_csrs {
                        self.esp32c6_active_interrupts.pop();
                    }
                    self.pc = self.csrs[usize::from(CSR_UEPC)];
                    Ok(StepReason::Advanced)
                }
                _ => self.illegal(instruction),
            };
        }
        if !self.profile.extension_zicsr {
            return self.illegal(instruction);
        }
        let csr_address = (instruction >> 20) as u16;
        let required_privilege = ((csr_address >> 8) & 3) as u8;
        if (self.privilege as u8) < required_privilege {
            self.enter_trap(2, instruction, false);
            return Ok(StepReason::Advanced);
        }
        let source = match funct3 {
            1..=3 => self.read_register(rs1)?,
            5..=7 => u32::from(rs1),
            _ => return self.illegal(instruction),
        };
        if self.profile.interrupt_model == InterruptModel::Hazard3
            && matches!(csr_address, CSR_MEIEA | CSR_MEIPA | CSR_MEIFA | CSR_MEIPRA)
        {
            let old = self.hazard3_irqarray_access(csr_address, source, funct3)?;
            self.check_register(rd)?;
            self.write_register(rd, old);
            return Ok(StepReason::Advanced);
        }
        let old = self.read_csr(csr_address)?;
        let write = match funct3 {
            1 | 5 => Some(source),
            2 | 6 if source != 0 => Some(old | source),
            3 | 7 if source != 0 => Some(old & !source),
            2 | 3 | 6 | 7 => None,
            _ => return self.illegal(instruction),
        };
        if let Some(value) = write {
            self.write_csr(csr_address, value)?;
        }
        self.check_register(rd)?;
        self.write_register(rd, old);
        Ok(StepReason::Advanced)
    }

    fn execute_atomic(
        &mut self,
        instruction: u32,
        rd: u8,
        rs1: u8,
        rs2: u8,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        let operation = (instruction >> 27) & 0x1f;
        let address = self.read_register(rs1)?;
        self.check_register(rd)?;
        if address & 3 != 0 {
            return Err(CpuFault::new(
                CpuFaultKind::Architecture,
                self.pc.into(),
                format!("misaligned atomic word address {address:#010x}"),
            ));
        }
        if operation == 0x02 {
            if rs2 != 0 {
                return self.illegal(instruction);
            }
            let old = self.load(bus, address, AccessWidth::Word, now)?;
            self.reservation = Some(address);
            self.write_register(rd, old);
            return Ok(());
        }
        if operation == 0x03 {
            let success = self.reservation == Some(address);
            self.reservation = None;
            if success {
                let value = self.read_register(rs2)?;
                self.store(bus, address, AccessWidth::Word, value, now)?;
            }
            self.write_register(rd, u32::from(!success));
            return Ok(());
        }
        let old = self.load(bus, address, AccessWidth::Word, now)?;
        let operand = self.read_register(rs2)?;
        let value = match operation {
            0x00 => old.wrapping_add(operand),
            0x01 => operand,
            0x04 => old ^ operand,
            0x08 => old | operand,
            0x0c => old & operand,
            0x10 => (old as i32).min(operand as i32) as u32,
            0x14 => (old as i32).max(operand as i32) as u32,
            0x18 => old.min(operand),
            0x1c => old.max(operand),
            _ => return self.illegal(instruction),
        };
        self.store(bus, address, AccessWidth::Word, value, now)?;
        self.write_register(rd, old);
        Ok(())
    }

    pub(super) fn execute16(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let quadrant = instruction & 0x3;
        let funct3 = instruction >> 13;
        let current_pc = self.pc;
        let mut next_pc = self.pc.wrapping_add(2);
        let mut reason = StepReason::Advanced;

        match (quadrant, funct3) {
            (0, 0) => {
                let immediate = decode_c_addi4spn(instruction);
                if immediate == 0 {
                    return self.illegal16(instruction);
                }
                let rd = compact_register((instruction >> 2) & 0x7);
                self.write_register(rd, self.registers[2].wrapping_add(immediate));
            }
            (0, 1) if self.profile.extension_xw => {
                self.execute_qingke_xw(instruction, QingKeXwOperation::LoadByteCompact, bus, now)?;
            }
            (0, 2) => {
                let rd = compact_register((instruction >> 2) & 0x7);
                let rs1 = compact_register((instruction >> 7) & 0x7);
                let address = self
                    .read_register(rs1)?
                    .wrapping_add(decode_c_lw_sw(instruction));
                let value = self.load(bus, address, AccessWidth::Word, now)?;
                self.write_register(rd, value);
            }
            (0, 4) if self.profile.extension_xw => self.execute_qingke_xw(
                instruction,
                match (instruction >> 5) & 3 {
                    0 => QingKeXwOperation::LoadByteStack,
                    1 => QingKeXwOperation::LoadHalfStack,
                    2 => QingKeXwOperation::StoreByteStack,
                    3 => QingKeXwOperation::StoreHalfStack,
                    _ => unreachable!(),
                },
                bus,
                now,
            )?,
            (0, 4) if self.profile.extension_b => {
                let operation = (instruction >> 10) & 3;
                let register = compact_register((instruction >> 2) & 7);
                let base = self.read_register(compact_register((instruction >> 7) & 7))?;
                match operation {
                    0 => {
                        let immediate = u32::from((instruction >> 6) & 1)
                            | (u32::from((instruction >> 5) & 1) << 1);
                        let address = base.wrapping_add(immediate);
                        let value = self.load(bus, address, AccessWidth::Byte, now)?;
                        self.write_register(register, value);
                    }
                    1 => {
                        let address = base.wrapping_add(u32::from((instruction >> 5) & 1) * 2);
                        let value = self.load(bus, address, AccessWidth::HalfWord, now)?;
                        self.write_register(
                            register,
                            if instruction & (1 << 6) != 0 {
                                i32::from(value as u16 as i16) as u32
                            } else {
                                value
                            },
                        );
                    }
                    2 => {
                        let immediate = u32::from((instruction >> 6) & 1)
                            | (u32::from((instruction >> 5) & 1) << 1);
                        let address = base.wrapping_add(immediate);
                        self.store(
                            bus,
                            address,
                            AccessWidth::Byte,
                            self.read_register(register)?,
                            now,
                        )?;
                    }
                    3 => {
                        let address = base.wrapping_add(u32::from((instruction >> 5) & 1) * 2);
                        self.store(
                            bus,
                            address,
                            AccessWidth::HalfWord,
                            self.read_register(register)?,
                            now,
                        )?;
                    }
                    _ => unreachable!(),
                }
            }
            (0, 5) if self.profile.extension_xw => {
                self.execute_qingke_xw(instruction, QingKeXwOperation::StoreByteCompact, bus, now)?;
            }
            (0, 6) => {
                let rs2 = compact_register((instruction >> 2) & 0x7);
                let rs1 = compact_register((instruction >> 7) & 0x7);
                let address = self
                    .read_register(rs1)?
                    .wrapping_add(decode_c_lw_sw(instruction));
                self.store(
                    bus,
                    address,
                    AccessWidth::Word,
                    self.read_register(rs2)?,
                    now,
                )?;
            }
            (1, 0) => {
                let rd = ((instruction >> 7) & 0x1f) as u8;
                self.check_register(rd)?;
                let immediate = decode_c_imm6(instruction);
                self.write_register(rd, self.read_register(rd)?.wrapping_add_signed(immediate));
            }
            (1, 1) => {
                // RV32 C.JAL
                self.write_register(1, next_pc);
                next_pc = current_pc.wrapping_add_signed(decode_c_jump(instruction));
            }
            (1, 2) => {
                let rd = ((instruction >> 7) & 0x1f) as u8;
                self.check_register(rd)?;
                self.write_register(rd, decode_c_imm6(instruction) as u32);
            }
            (1, 3) => {
                let rd = ((instruction >> 7) & 0x1f) as u8;
                if rd == 2 {
                    let immediate = decode_c_addi16sp(instruction);
                    if immediate == 0 {
                        return self.illegal16(instruction);
                    }
                    self.registers[2] = self.registers[2].wrapping_add_signed(immediate);
                } else {
                    self.check_register(rd)?;
                    let immediate = decode_c_imm6(instruction);
                    if rd == 0 || immediate == 0 {
                        return self.illegal16(instruction);
                    }
                    self.write_register(rd, (immediate as u32) << 12);
                }
            }
            (1, 4) => self.execute_c_alu(instruction)?,
            (1, 5) => next_pc = current_pc.wrapping_add_signed(decode_c_jump(instruction)),
            (1, 6 | 7) => {
                let rs1 = compact_register((instruction >> 7) & 0x7);
                let zero = self.read_register(rs1)? == 0;
                if (funct3 == 6 && zero) || (funct3 == 7 && !zero) {
                    next_pc = current_pc.wrapping_add_signed(decode_c_branch(instruction));
                }
            }
            (2, 0) => {
                let rd = ((instruction >> 7) & 0x1f) as u8;
                self.check_register(rd)?;
                let shift = ((instruction >> 2) & 0x1f) as u32;
                if rd == 0 || instruction & (1 << 12) != 0 {
                    return self.illegal16(instruction);
                }
                self.write_register(rd, self.read_register(rd)? << shift);
            }
            (2, 1) if self.profile.extension_xw => {
                self.execute_qingke_xw(instruction, QingKeXwOperation::LoadHalfCompact, bus, now)?;
            }
            (2, 2) => {
                let rd = ((instruction >> 7) & 0x1f) as u8;
                self.check_register(rd)?;
                if rd == 0 {
                    return self.illegal16(instruction);
                }
                let address = self.registers[2].wrapping_add(decode_c_lwsp(instruction));
                let value = self.load(bus, address, AccessWidth::Word, now)?;
                self.write_register(rd, value);
            }
            (2, 4) => {
                let rd_rs1 = ((instruction >> 7) & 0x1f) as u8;
                let rs2 = ((instruction >> 2) & 0x1f) as u8;
                self.check_register(rd_rs1)?;
                self.check_register(rs2)?;
                let bit12 = instruction & (1 << 12) != 0;
                match (bit12, rd_rs1, rs2) {
                    (false, 0, _) => return self.illegal16(instruction),
                    (false, _, 0) => next_pc = self.read_register(rd_rs1)? & !1,
                    (false, _, _) => self.write_register(rd_rs1, self.read_register(rs2)?),
                    (true, 0, 0) if self.profile.ebreak_halts => {
                        self.halted = true;
                        reason = StepReason::Halted;
                    }
                    (true, 0, 0) => reason = StepReason::Breakpoint,
                    (true, _, 0) => {
                        self.write_register(1, next_pc);
                        next_pc = self.read_register(rd_rs1)? & !1;
                    }
                    (true, _, _) => self.write_register(
                        rd_rs1,
                        self.read_register(rd_rs1)?
                            .wrapping_add(self.read_register(rs2)?),
                    ),
                }
            }
            (2, 5) if self.profile.extension_xw => {
                self.execute_qingke_xw(instruction, QingKeXwOperation::StoreHalfCompact, bus, now)?;
            }
            (2, 5) if self.profile.extension_zcmp && instruction & 0xf800 == 0xb800 => {
                if let Some(return_address) = self.execute_zcmp(instruction, bus, now)? {
                    next_pc = return_address;
                }
            }
            (2, 6) => {
                let rs2 = ((instruction >> 2) & 0x1f) as u8;
                self.check_register(rs2)?;
                let address = self.registers[2].wrapping_add(decode_c_swsp(instruction));
                self.store(
                    bus,
                    address,
                    AccessWidth::Word,
                    self.read_register(rs2)?,
                    now,
                )?;
            }
            _ => return self.illegal16(instruction),
        }
        if next_pc & 1 != 0 {
            return Err(CpuFault::new(
                CpuFaultKind::Architecture,
                current_pc.into(),
                format!("misaligned compressed instruction target {next_pc:#010x}"),
            ));
        }
        self.pc = next_pc;
        Ok(reason)
    }

    fn execute_qingke_xw(
        &mut self,
        instruction: u16,
        operation: QingKeXwOperation,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        let data_register = compact_register((instruction >> 2) & 7);
        let compact_base = compact_register((instruction >> 7) & 7);
        let compact_byte_offset = u32::from((instruction >> 12) & 1)
            | (u32::from((instruction >> 5) & 3) << 1)
            | (u32::from((instruction >> 10) & 3) << 3);
        let compact_half_offset =
            (u32::from((instruction >> 5) & 3) << 1) | (u32::from((instruction >> 10) & 7) << 3);
        let stack_byte_offset = u32::from((instruction >> 7) & 0xf);
        let stack_half_offset =
            (u32::from((instruction >> 8) & 7) << 1) | (u32::from((instruction >> 7) & 1) << 4);

        let (base, register, offset, width, load) = match operation {
            QingKeXwOperation::LoadByteCompact => (
                compact_base,
                data_register,
                compact_byte_offset,
                AccessWidth::Byte,
                true,
            ),
            QingKeXwOperation::LoadHalfCompact => (
                compact_base,
                data_register,
                compact_half_offset,
                AccessWidth::HalfWord,
                true,
            ),
            QingKeXwOperation::StoreByteCompact => (
                compact_base,
                data_register,
                compact_byte_offset,
                AccessWidth::Byte,
                false,
            ),
            QingKeXwOperation::StoreHalfCompact => (
                compact_base,
                data_register,
                compact_half_offset,
                AccessWidth::HalfWord,
                false,
            ),
            QingKeXwOperation::LoadByteStack => {
                (2, data_register, stack_byte_offset, AccessWidth::Byte, true)
            }
            QingKeXwOperation::LoadHalfStack => (
                2,
                data_register,
                stack_half_offset,
                AccessWidth::HalfWord,
                true,
            ),
            QingKeXwOperation::StoreByteStack => (
                2,
                data_register,
                stack_byte_offset,
                AccessWidth::Byte,
                false,
            ),
            QingKeXwOperation::StoreHalfStack => (
                2,
                data_register,
                stack_half_offset,
                AccessWidth::HalfWord,
                false,
            ),
        };
        let address = self.read_register(base)?.wrapping_add(offset);
        if load {
            let value = self.load(bus, address, width, now)?;
            self.write_register(register, value);
        } else {
            self.store(bus, address, width, self.read_register(register)?, now)?;
        }
        Ok(())
    }

    fn execute_zcmp(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<Option<u32>, CpuFault> {
        let rlist = ((instruction >> 4) & 0xf) as u8;
        if rlist < 4 {
            return self.illegal16(instruction);
        }
        let registers: &[u8] = match rlist {
            4 => &[1],
            5 => &[1, 8],
            6 => &[1, 8, 9],
            7 => &[1, 8, 9, 18],
            8 => &[1, 8, 9, 18, 19],
            9 => &[1, 8, 9, 18, 19, 20],
            10 => &[1, 8, 9, 18, 19, 20, 21],
            11 => &[1, 8, 9, 18, 19, 20, 21, 22],
            12 => &[1, 8, 9, 18, 19, 20, 21, 22, 23],
            13 => &[1, 8, 9, 18, 19, 20, 21, 22, 23, 24],
            14 => &[1, 8, 9, 18, 19, 20, 21, 22, 23, 24, 25],
            15 => &[1, 8, 9, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27],
            _ => unreachable!(),
        };
        let stack_base = match rlist {
            4..=7 => 16,
            8..=11 => 32,
            12..=14 => 48,
            15 => 64,
            _ => unreachable!(),
        };
        let stack_adjust = stack_base + u32::from((instruction >> 2) & 3) * 16;
        let operation = (instruction >> 8) & 7;

        if operation == 0 {
            let old_sp = self.registers[2];
            for (index, register) in registers.iter().rev().enumerate() {
                let address = old_sp.wrapping_sub(((index + 1) * 4) as u32);
                self.store(
                    bus,
                    address,
                    AccessWidth::Word,
                    self.read_register(*register)?,
                    now,
                )?;
            }
            self.registers[2] = old_sp.wrapping_sub(stack_adjust);
            return Ok(None);
        }

        if !matches!(operation, 2 | 4 | 6) {
            return self.illegal16(instruction);
        }
        let old_sp = self.registers[2];
        for (index, register) in registers.iter().rev().enumerate() {
            let address = old_sp
                .wrapping_add(stack_adjust)
                .wrapping_sub(((index + 1) * 4) as u32);
            let value = self.load(bus, address, AccessWidth::Word, now)?;
            self.write_register(*register, value);
        }
        self.registers[2] = old_sp.wrapping_add(stack_adjust);
        if operation == 4 {
            self.write_register(10, 0);
        }
        Ok((operation >= 4).then(|| self.registers[1] & !1))
    }

    fn execute_c_alu(&mut self, instruction: u16) -> Result<(), CpuFault> {
        let rd = compact_register((instruction >> 7) & 0x7);
        let operation = (instruction >> 10) & 0x3;
        let immediate = decode_c_imm6(instruction);
        let left = self.read_register(rd)?;
        let value = match operation {
            0 => {
                if instruction & (1 << 12) != 0 {
                    return self.illegal16(instruction);
                }
                left >> (u32::from(instruction >> 2) & 0x1f)
            }
            1 => {
                if instruction & (1 << 12) != 0 {
                    return self.illegal16(instruction);
                }
                ((left as i32) >> (u32::from(instruction >> 2) & 0x1f)) as u32
            }
            2 => left & immediate as u32,
            3 => {
                if instruction & (1 << 12) != 0 {
                    return self.illegal16(instruction);
                }
                let right = self.read_register(compact_register((instruction >> 2) & 0x7))?;
                match (instruction >> 5) & 0x3 {
                    0 => left.wrapping_sub(right),
                    1 => left ^ right,
                    2 => left | right,
                    3 => left & right,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        };
        self.write_register(rd, value);
        Ok(())
    }

    pub(super) fn illegal<T>(&self, instruction: u32) -> Result<T, CpuFault> {
        Err(CpuFault::new(
            CpuFaultKind::IllegalInstruction,
            self.pc.into(),
            format!(
                "instruction {instruction:#010x} is not valid for {}",
                self.profile.name
            ),
        ))
    }

    pub(super) fn illegal16<T>(&self, instruction: u16) -> Result<T, CpuFault> {
        Err(CpuFault::new(
            CpuFaultKind::IllegalInstruction,
            self.pc.into(),
            format!(
                "compressed instruction {instruction:#06x} is not valid for {}",
                self.profile.name
            ),
        ))
    }
}
