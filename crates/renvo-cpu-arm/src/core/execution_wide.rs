use super::*;

impl ArmCpu {
    pub(super) fn execute_wide(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
        pc: u32,
        next: u32,
    ) -> Result<StepReason, CpuFault> {
        let second = self.read(bus, next, AccessWidth::HalfWord, AccessKind::Execute, now)? as u16;
        if instruction == 0xee10 && second == 0x0430 {
            // RP2350's optional double-precision coprocessor uses this MRC as RCMP to clear
            // its engaged flag during per-core startup. The functional arithmetic model has
            // no persistent engaged state.
            self.registers[0] = 0;
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if self.profile == ArmProfile::CortexM33
            && instruction & 0xff10 == 0xee00
            && second & 0x0f10 == 0x0010
            && instruction & 0xf == 0
        {
            // RP2350 GPIO coprocessor MCR. The coprocessor is an
            // architectural alias for the SIO GPIO registers; route the
            // low GPIO bank through the bus so the device model remains
            // the single owner of signal state and VCD changes.
            let operation = (instruction >> 5) & 7;
            let source = usize::from((second >> 12) & 0xf);
            let bank = second & 0xf;
            let offset = match (bank, operation) {
                (0, 0) => Some(0x10),
                (0, 1) => Some(0x28),
                (0, 2) => Some(0x18),
                (0, 3) => Some(0x20),
                (4, 0) => Some(0x30),
                (4, 1) => Some(0x48),
                (4, 2) => Some(0x38),
                (4, 3) => Some(0x40),
                (0, 5) => Some(0x28),
                (0, 6) => Some(0x18),
                (0, 7) => Some(0x20),
                (4, 5) => Some(0x48),
                (4, 6) => Some(0x38),
                (4, 7) => Some(0x40),
                _ => None,
            };
            if let Some(offset) = offset {
                let value = if operation >= 5 {
                    1_u32.checked_shl(self.registers[source]).unwrap_or(0)
                } else {
                    self.registers[source]
                };
                if value != 0 || operation < 5 {
                    self.write(bus, 0xd000_0000 + offset, AccessWidth::Word, value, now)?;
                }
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if self.profile == ArmProfile::CortexM33
            && instruction & 0xff10 == 0xee10
            && second & 0x0f10 == 0x0010
            && instruction & 0xf == 0
            && (instruction >> 5) & 7 == 0
        {
            // RP2350 GPIO coprocessor MRC.
            let destination = usize::from((second >> 12) & 0xf);
            let offset = match second & 0xf {
                0 => Some(0x10),
                4 => Some(0x30),
                8 => Some(0x04),
                1 | 5 | 9 => None,
                _ => None,
            };
            self.registers[destination] = if let Some(offset) = offset {
                self.read(
                    bus,
                    0xd000_0000 + offset,
                    AccessWidth::Word,
                    AccessKind::Read,
                    now,
                )?
            } else {
                0
            };
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if self.profile == ArmProfile::CortexM33
            && instruction & 0xfff0 == 0xec40
            && second & 0x0f00 == 0
        {
            // RP2350 GPIO coprocessor MCRR. The implemented low bank
            // covers all 30 package GPIOs used by Pico 2; high-bank
            // values concern QSPI and USB pads outside this signal model.
            let first = self.registers[usize::from((second >> 12) & 0xf)];
            let second_value = self.registers[usize::from(instruction & 0xf)];
            let operation = (second >> 4) & 0xf;
            let bank = second & 0xf;
            let register_offsets = match bank {
                0 => Some([0x10, 0x28, 0x18, 0x20]),
                4 => Some([0x30, 0x48, 0x38, 0x40]),
                _ => None,
            };
            if let Some(offsets) = register_offsets {
                match operation {
                    0..=3 => self.write(
                        bus,
                        0xd000_0000 + offsets[usize::from(operation)],
                        AccessWidth::Word,
                        first,
                        now,
                    )?,
                    4..=7 => {
                        let mask = 1_u32.checked_shl(first).unwrap_or(0);
                        let selected = if operation == 4 {
                            if second_value == 0 {
                                offsets[3]
                            } else {
                                offsets[2]
                            }
                        } else {
                            offsets[usize::from(operation - 4)]
                        };
                        if mask != 0 && (operation == 4 || second_value != 0) {
                            self.write(bus, 0xd000_0000 + selected, AccessWidth::Word, mask, now)?;
                        }
                    }
                    8..=11 if second_value == 0 => self.write(
                        bus,
                        0xd000_0000 + offsets[usize::from(operation - 8)],
                        AccessWidth::Word,
                        first,
                        now,
                    )?,
                    _ => {}
                }
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if self.profile == ArmProfile::CortexM33
            && instruction & 0xfff0 == 0xec50
            && second & 0x0f00 == 0
            && (second >> 4) & 0xf == 0
        {
            // RP2350 GPIO coprocessor MRRC.
            let low_register = usize::from((second >> 12) & 0xf);
            let high_register = usize::from(instruction & 0xf);
            let offset = match second & 0xf {
                0 => Some(0x10),
                4 => Some(0x30),
                8 => Some(0x04),
                _ => None,
            };
            self.registers[low_register] = if let Some(offset) = offset {
                self.read(
                    bus,
                    0xd000_0000 + offset,
                    AccessWidth::Word,
                    AccessKind::Read,
                    now,
                )?
            } else {
                0
            };
            self.registers[high_register] = 0;
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffe0 == 0xee00 && second & 0x0f7f == 0x0a10 {
            let core_register = usize::from((second >> 12) & 0xf);
            let single_register =
                usize::from(instruction & 0xf) * 2 + usize::from((second >> 7) & 1);
            if instruction & 0x10 == 0 {
                self.set_single_register(single_register, self.registers[core_register]);
            } else {
                self.registers[core_register] = self.single_register(single_register);
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffbf == 0xeeb8 && second & 0x0f50 == 0x0a40 {
            let destination =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let source = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
            let integer = self.single_register(source);
            let value = if second & 0x80 != 0 {
                integer as i32 as f32
            } else {
                integer as f32
            };
            self.set_single_register(destination, value.to_bits());
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffbe == 0xeebc && second & 0x0f50 == 0x0a40 {
            let destination =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let source = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
            let value = f32::from_bits(self.single_register(source));
            let integer = if instruction & 1 != 0 {
                (value as i32) as u32
            } else {
                value as u32
            };
            self.set_single_register(destination, integer);
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfffe == 0xeefe && second & 0x0fc0 == 0x0ac0 {
            let register =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let encoded_fraction = u32::from(second & 0xf) * 2 + u32::from((second >> 5) & 1);
            let fraction_bits = 32_u32.saturating_sub(encoded_fraction);
            let scaled =
                f32::from_bits(self.single_register(register)) * (2_u32.pow(fraction_bits) as f32);
            let integer = if instruction & 1 != 0 {
                scaled as u32
            } else {
                (scaled as i32) as u32
            };
            self.set_single_register(register, integer);
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffb0 == 0xeeb0 && second & 0x0ff0 == 0x0a00 {
            let destination =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let immediate = ((u32::from(instruction & 0xf)) << 4) | u32::from(second & 0xf);
            let bit6 = immediate & 0x40 != 0;
            let exponent = (u32::from(!bit6) << 7)
                | ((if bit6 { 0x1f } else { 0 }) << 2)
                | ((immediate >> 4) & 3);
            let bits = ((immediate & 0x80) << 24) | (exponent << 23) | ((immediate & 0xf) << 19);
            self.set_single_register(destination, bits);
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffbf == 0xeeb0 && second & 0x0f50 == 0x0a40 {
            let destination =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let source = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
            let mut value = self.single_register(source);
            if second & 0x80 != 0 {
                value &= 0x7fff_ffff;
            }
            self.set_single_register(destination, value);
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffbf == 0xeeb1 && second & 0x0f50 == 0x0a40 {
            let destination =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let source = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
            self.set_single_register(destination, self.single_register(source) ^ N);
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(instruction & 0xffb0, 0xee20 | 0xee30 | 0xee80) && second & 0x0f10 == 0x0a00 {
            let destination =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let left_register = usize::from(instruction & 0xf) * 2 + usize::from((second >> 7) & 1);
            let right_register = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
            let left = f32::from_bits(self.single_register(left_register));
            let right = f32::from_bits(self.single_register(right_register));
            let value = match instruction & 0xffb0 {
                0xee20 => left * right,
                0xee30 if second & 0x40 == 0 => left + right,
                0xee30 => left - right,
                0xee80 => left / right,
                _ => unreachable!(),
            };
            self.set_single_register(destination, value.to_bits());
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffb0 == 0xeea0 && second & 0x0f10 == 0x0a00 {
            let destination =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let left_register = usize::from(instruction & 0xf) * 2 + usize::from((second >> 7) & 1);
            let right_register = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
            let accumulator = f32::from_bits(self.single_register(destination));
            let left = f32::from_bits(self.single_register(left_register));
            let right = f32::from_bits(self.single_register(right_register));
            let value = if second & 0x40 == 0 {
                left.mul_add(right, accumulator)
            } else {
                (-left).mul_add(right, accumulator)
            };
            self.set_single_register(destination, value.to_bits());
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffbf == 0xeeb5 && second & 0x0f3f == 0x0a00 {
            let left_register =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let left = f32::from_bits(self.single_register(left_register));
            self.xpsr &= !(N | Z | C | V);
            if left.is_nan() {
                self.xpsr |= C | V;
            } else if left == 0.0 {
                self.xpsr |= Z | C;
            } else if left < 0.0 {
                self.xpsr |= N;
            } else {
                self.xpsr |= C;
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffbf == 0xeeb4 && second & 0x0f50 == 0x0a40 {
            let left_register =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let right_register = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
            let left = f32::from_bits(self.single_register(left_register));
            let right = f32::from_bits(self.single_register(right_register));
            self.xpsr &= !(N | Z | C | V);
            if left.is_nan() || right.is_nan() {
                self.xpsr |= C | V;
            } else if left == right {
                self.xpsr |= Z | C;
            } else if left < right {
                self.xpsr |= N;
            } else {
                self.xpsr |= C;
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction == 0xeef1 && second == 0xfa10 {
            // VMRS APSR_nzcv,FPSCR. Functional VFP comparisons materialize their flags
            // directly in APSR, so this architectural transfer has no additional work.
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xff00 == 0xfe00 && second & 0x0f10 == 0x0a00 {
            let destination =
                usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
            let true_register = usize::from(instruction & 0xf) * 2 + usize::from((second >> 7) & 1);
            let false_register = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
            let condition = match (instruction >> 4) & 3 {
                0 => 0,  // EQ
                1 => 6,  // VS
                2 => 10, // GE
                3 => 12, // GT
                _ => unreachable!(),
            };
            let source = if self.condition(condition) {
                true_register
            } else {
                false_register
            };
            self.set_single_register(destination, self.single_register(source));
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xff00 == 0xed00
            && second & 0x0f00 == 0x0a00
            && !matches!(instruction, 0xed2d | 0xecbd)
        {
            let base_register = usize::from(instruction & 0xf);
            let base = if base_register == 15 {
                pc.wrapping_add(4) & !3
            } else {
                self.registers[base_register]
            };
            let offset = u32::from(second & 0xff) << 2;
            let address = if instruction & 0x0080 != 0 {
                base.wrapping_add(offset)
            } else {
                base.wrapping_sub(offset)
            };
            let single_register =
                (usize::from((second >> 12) & 0xf) << 1) | usize::from((instruction >> 6) & 1);
            let double_register = single_register / 2;
            let high_half = single_register & 1 != 0;
            if instruction & 0x10 != 0 {
                let value = self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                let existing = self.fpu_registers[double_register];
                self.fpu_registers[double_register] = if high_half {
                    (existing & 0x0000_0000_ffff_ffff) | (u64::from(value) << 32)
                } else {
                    (existing & 0xffff_ffff_0000_0000) | u64::from(value)
                };
            } else {
                let pair = self.fpu_registers[double_register];
                let value = if high_half {
                    (pair >> 32) as u32
                } else {
                    pair as u32
                };
                self.write(bus, address, AccessWidth::Word, value, now)?;
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xff00 == 0xed00
            && second & 0x0f00 == 0x0b00
            && !matches!(instruction, 0xed2d | 0xecbd)
        {
            let base = self.registers[usize::from(instruction & 0xf)];
            let offset = u32::from(second & 0xff) << 2;
            let address = if instruction & 0x0080 != 0 {
                base.wrapping_add(offset)
            } else {
                base.wrapping_sub(offset)
            };
            let register =
                usize::from((second >> 12) & 0xf) | (usize::from((instruction >> 6) & 1) << 4);
            if instruction & 0x10 != 0 {
                let low = self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                let high = self.read(
                    bus,
                    address.wrapping_add(4),
                    AccessWidth::Word,
                    AccessKind::Read,
                    now,
                )?;
                self.fpu_registers[register] = u64::from(low) | (u64::from(high) << 32);
            } else {
                let value = self.fpu_registers[register];
                self.write(bus, address, AccessWidth::Word, value as u32, now)?;
                self.write(
                    bus,
                    address.wrapping_add(4),
                    AccessWidth::Word,
                    (value >> 32) as u32,
                    now,
                )?;
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(instruction, 0xed2d | 0xecbd) && second & 0x0f00 == 0x0b00 {
            let load = instruction == 0xecbd;
            let first_register =
                usize::from((second >> 12) & 0xf) | (usize::from((instruction >> 6) & 1) << 4);
            let words = u32::from(second & 0xff);
            if words == 0 || words & 1 != 0 {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    "VPOP/VPUSH double-register list has invalid size",
                ));
            }
            let count = usize::try_from(words / 2).expect("VFP list count fits usize");
            if first_register + count > self.fpu_registers.len() {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    "VPOP/VPUSH register list exceeds D31",
                ));
            }
            let mut address = if load {
                self.registers[13]
            } else {
                self.registers[13].wrapping_sub(words * 4)
            };
            for register in first_register..first_register + count {
                if load {
                    let low = self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                    let high = self.read(
                        bus,
                        address.wrapping_add(4),
                        AccessWidth::Word,
                        AccessKind::Read,
                        now,
                    )?;
                    self.fpu_registers[register] = u64::from(low) | (u64::from(high) << 32);
                } else {
                    let value = self.fpu_registers[register];
                    self.write(bus, address, AccessWidth::Word, value as u32, now)?;
                    self.write(
                        bus,
                        address.wrapping_add(4),
                        AccessWidth::Word,
                        (value >> 32) as u32,
                        now,
                    )?;
                }
                address = address.wrapping_add(8);
            }
            self.registers[13] = if load {
                self.registers[13].wrapping_add(words * 4)
            } else {
                self.registers[13].wrapping_sub(words * 4)
            };
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xe840 && second & 0xf0f0 == 0xf000 {
            // Armv8-M Test Target reports the security/MPU attribution of an address.
            // Renvo currently has one non-secure functional address space, represented by
            // an all-zero TT response. SDK code uses bit 22 of this result to select its
            // boot-ROM lookup mask.
            let destination = usize::from((second >> 8) & 0xf);
            self.registers[destination] = 0;
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xe8c0 && second & 0x0fff == 0x0f8f {
            // Store-release byte has ordinary byte-store data effects in the
            // single-threaded interpreter; all earlier accesses are already complete.
            let address = self.registers[usize::from(instruction & 0xf)];
            let source = usize::from((second >> 12) & 0xf);
            self.write(bus, address, AccessWidth::Byte, self.registers[source], now)?;
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xe8d0 && second & 0x0fff == 0x0fcf {
            let address = self.registers[usize::from(instruction & 0xf)];
            let destination = usize::from((second >> 12) & 0xf);
            self.registers[destination] =
                self.read(bus, address, AccessWidth::Byte, AccessKind::Read, now)?;
            self.exclusive_address = Some(address);
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xe8d0 && second & 0xffe0 == 0xf000 {
            let base_register = usize::from(instruction & 0xf);
            let index_register = usize::from(second & 0xf);
            let halfword = second & 0x10 != 0;
            let base = if base_register == 15 {
                pc.wrapping_add(4)
            } else {
                self.registers[base_register]
            };
            let index = self.registers[index_register] << u32::from(halfword);
            let width = if halfword {
                AccessWidth::HalfWord
            } else {
                AccessWidth::Byte
            };
            let displacement =
                self.read(bus, base.wrapping_add(index), width, AccessKind::Read, now)?;
            self.registers[15] = pc
                .wrapping_add(4)
                .wrapping_add(displacement.wrapping_mul(2));
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xe8c0 && second & 0x0ff0 == 0x0f40 {
            let address = self.registers[usize::from(instruction & 0xf)];
            let value_register = usize::from((second >> 12) & 0xf);
            let status_register = usize::from(second & 0xf);
            if self.exclusive_address == Some(address) {
                self.write(
                    bus,
                    address,
                    AccessWidth::Byte,
                    self.registers[value_register],
                    now,
                )?;
                self.registers[status_register] = 0;
            } else {
                self.registers[status_register] = 1;
            }
            self.exclusive_address = None;
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfe40 == 0xe840 && instruction & 0x0120 != 0 {
            let base_register = usize::from(instruction & 0xf);
            let first_register = usize::from((second >> 12) & 0xf);
            let second_register = usize::from((second >> 8) & 0xf);
            let offset = u32::from(second & 0xff) << 2;
            let base = self.registers[base_register];
            let adjusted = if instruction & 0x0080 != 0 {
                base.wrapping_add(offset)
            } else {
                base.wrapping_sub(offset)
            };
            let address = if instruction & 0x0100 != 0 {
                adjusted
            } else {
                base
            };
            if instruction & 0x10 != 0 {
                self.registers[first_register] =
                    self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                self.registers[second_register] = self.read(
                    bus,
                    address.wrapping_add(4),
                    AccessWidth::Word,
                    AccessKind::Read,
                    now,
                )?;
            } else {
                self.write(
                    bus,
                    address,
                    AccessWidth::Word,
                    self.registers[first_register],
                    now,
                )?;
                self.write(
                    bus,
                    address.wrapping_add(4),
                    AccessWidth::Word,
                    self.registers[second_register],
                    now,
                )?;
            }
            if instruction & 0x20 != 0 {
                self.registers[base_register] = adjusted;
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(instruction & 0xfff0, 0xfb90 | 0xfbb0) && second & 0xf0f0 == 0xf0f0 {
            let numerator = self.registers[usize::from(instruction & 0xf)];
            let denominator = self.registers[usize::from(second & 0xf)];
            let destination = usize::from((second >> 8) & 0xf);
            self.registers[destination] = if denominator == 0 {
                0
            } else if instruction & 0x20 != 0 {
                numerator / denominator
            } else {
                (numerator as i32).wrapping_div(denominator as i32) as u32
            };
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xfab0 && second & 0xf0f0 == 0xf080 {
            let source = self.registers[usize::from(second & 0xf)];
            let destination = usize::from((second >> 8) & 0xf);
            self.registers[destination] = source.leading_zeros();
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xfa90 && second & 0xf0f0 == 0xf0a0 {
            let source = self.registers[usize::from(second & 0xf)];
            let destination = usize::from((second >> 8) & 0xf);
            self.registers[destination] = source.reverse_bits();
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xff80 == 0xfa00 && second & 0xf0f0 == 0xf000 {
            let source = self.registers[usize::from(instruction & 0xf)];
            let amount = self.registers[usize::from(second & 0xf)] & 0xff;
            let destination = usize::from((second >> 8) & 0xf);
            self.registers[destination] = match (instruction >> 5) & 3 {
                0 => source.checked_shl(amount).unwrap_or(0),
                1 => source.checked_shr(amount).unwrap_or(0),
                2 => (source as i32)
                    .checked_shr(amount)
                    .unwrap_or(if source & N == 0 { 0 } else { -1 }) as u32,
                3 => source.rotate_right(amount & 31),
                _ => unreachable!(),
            };
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(instruction, 0xfa0f | 0xfa1f | 0xfa4f | 0xfa5f) && second & 0xf0c0 == 0xf080 {
            let source = self.registers[usize::from(second & 0xf)]
                .rotate_right(u32::from((second >> 4) & 3) * 8);
            let destination = usize::from((second >> 8) & 0xf);
            self.registers[destination] = match instruction {
                0xfa0f => i32::from(source as u16 as i16) as u32,
                0xfa1f => source & 0xffff,
                0xfa4f => i32::from(source as u8 as i8) as u32,
                0xfa5f => source & 0xff,
                _ => unreachable!(),
            };
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(instruction & 0xfff0, 0xfa00 | 0xfa10 | 0xfa40 | 0xfa50)
            && second & 0xf0c0 == 0xf080
        {
            let operation = instruction & 0xfff0;
            let left = self.registers[usize::from(instruction & 0xf)];
            let source = self.registers[usize::from(second & 0xf)]
                .rotate_right(u32::from((second >> 4) & 3) * 8);
            let extended = match operation {
                0xfa00 => i32::from(source as u16 as i16) as u32,
                0xfa10 => source & 0xffff,
                0xfa40 => i32::from(source as u8 as i8) as u32,
                0xfa50 => source & 0xff,
                _ => unreachable!(),
            };
            let destination = usize::from((second >> 8) & 0xf);
            self.registers[destination] = left.wrapping_add(extended);
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xfb00 && second & 0x00e0 == 0 {
            let left = self.registers[usize::from(instruction & 0xf)];
            let right = self.registers[usize::from(second & 0xf)];
            let accumulator = usize::from((second >> 12) & 0xf);
            let destination = usize::from((second >> 8) & 0xf);
            let product = left.wrapping_mul(right);
            self.registers[destination] = if second & 0x10 != 0 {
                self.registers[accumulator].wrapping_sub(product)
            } else if accumulator == 15 {
                product
            } else {
                product.wrapping_add(self.registers[accumulator])
            };
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xfb10 && second & 0x00c0 == 0 {
            let left_word = self.registers[usize::from(instruction & 0xf)];
            let right_word = self.registers[usize::from(second & 0xf)];
            let left = if second & 0x20 == 0 {
                left_word as u16 as i16
            } else {
                (left_word >> 16) as u16 as i16
            };
            let right = if second & 0x10 == 0 {
                right_word as u16 as i16
            } else {
                (right_word >> 16) as u16 as i16
            };
            let accumulator = usize::from((second >> 12) & 0xf);
            let destination = usize::from((second >> 8) & 0xf);
            let product = i32::from(left).wrapping_mul(i32::from(right)) as u32;
            self.registers[destination] = if accumulator == 15 {
                product
            } else {
                product.wrapping_add(self.registers[accumulator])
            };
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(instruction & 0xfff0, 0xfb80 | 0xfba0) && second & 0x00f0 == 0 {
            let left = self.registers[usize::from(instruction & 0xf)];
            let right = self.registers[usize::from(second & 0xf)];
            let product = if instruction & 0x20 != 0 {
                u64::from(left) * u64::from(right)
            } else {
                ((i64::from(left as i32) * i64::from(right as i32)) as u64) & u64::MAX
            };
            let low = usize::from((second >> 12) & 0xf);
            let high = usize::from((second >> 8) & 0xf);
            self.registers[low] = product as u32;
            self.registers[high] = (product >> 32) as u32;
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(instruction & 0xfff0, 0xfbc0 | 0xfbe0) && second & 0x00f0 == 0 {
            let left = self.registers[usize::from(instruction & 0xf)];
            let right = self.registers[usize::from(second & 0xf)];
            let low = usize::from((second >> 12) & 0xf);
            let high = usize::from((second >> 8) & 0xf);
            let accumulator =
                (u64::from(self.registers[high]) << 32) | u64::from(self.registers[low]);
            let product = if instruction & 0x20 != 0 {
                u64::from(left) * u64::from(right)
            } else {
                (i64::from(left as i32) * i64::from(right as i32)) as u64
            };
            let result = accumulator.wrapping_add(product);
            self.registers[low] = result as u32;
            self.registers[high] = (result >> 32) as u32;
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(instruction & 0xfff0, 0xf340 | 0xf3c0) && second & 0x8000 == 0 {
            let source = self.registers[usize::from(instruction & 0xf)];
            let destination = usize::from((second >> 8) & 0xf);
            let least_significant = u32::from(((second >> 12) & 7) << 2 | ((second >> 6) & 3));
            let width = u32::from(second & 0x1f) + 1;
            let mask = if width == 32 {
                u32::MAX
            } else {
                (1_u32 << width) - 1
            };
            let extracted = (source >> least_significant) & mask;
            self.registers[destination] = if instruction & 0x0080 != 0 {
                extracted
            } else if extracted & (1 << (width - 1)) != 0 {
                extracted | !mask
            } else {
                extracted
            };
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xf360 && second & 0x8000 == 0 {
            let source_register = usize::from(instruction & 0xf);
            let destination = usize::from((second >> 8) & 0xf);
            let least_significant = u32::from(((second >> 12) & 7) << 2 | ((second >> 6) & 3));
            let most_significant = u32::from(second & 0x1f);
            if most_significant < least_significant {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    "BFI most-significant bit precedes least-significant bit",
                ));
            }
            let width = most_significant - least_significant + 1;
            let field_mask = if width == 32 {
                u32::MAX
            } else {
                ((1_u32 << width) - 1) << least_significant
            };
            let source = if source_register == 15 {
                0
            } else {
                self.registers[source_register]
            };
            self.registers[destination] = (self.registers[destination] & !field_mask)
                | ((source << least_significant) & field_mask);
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        let shifted_arithmetic_operation = instruction & 0xffe0;
        if matches!(
            shifted_arithmetic_operation,
            0xeb00 | 0xeb40 | 0xeb60 | 0xeba0 | 0xebc0
        ) {
            let left = self.registers[usize::from(instruction & 0xf)];
            let right_register = usize::from(second & 0xf);
            let shift_amount = u32::from(((second >> 12) & 7) << 2 | ((second >> 6) & 3));
            let right = match (second >> 4) & 3 {
                0 => self.registers[right_register].wrapping_shl(shift_amount),
                1 => self.registers[right_register]
                    .checked_shr(if shift_amount == 0 { 32 } else { shift_amount })
                    .unwrap_or(0),
                2 => {
                    ((self.registers[right_register] as i32)
                        .checked_shr(if shift_amount == 0 { 32 } else { shift_amount })
                        .unwrap_or(if self.registers[right_register] & N == 0 {
                            0
                        } else {
                            -1
                        })) as u32
                }
                3 => self.registers[right_register].rotate_right(shift_amount),
                _ => unreachable!(),
            };
            let carry = u32::from(self.xpsr & C != 0);
            let result = match shifted_arithmetic_operation {
                0xeb00 => left.wrapping_add(right),
                0xeb40 => left.wrapping_add(right).wrapping_add(carry),
                0xeb60 => left.wrapping_sub(right).wrapping_sub(1 - carry),
                0xeba0 => left.wrapping_sub(right),
                0xebc0 => right.wrapping_sub(left),
                _ => unreachable!(),
            };
            let destination = usize::from((second >> 8) & 0xf);
            if destination != 15 {
                self.registers[destination] = result;
            }
            if instruction & 0x10 != 0 {
                match shifted_arithmetic_operation {
                    0xeb00 => self.add_flags(left, right, result),
                    0xeba0 => self.sub_flags(left, right, result),
                    0xebc0 => self.sub_flags(right, left, result),
                    0xeb40 | 0xeb60 => self.nz(result),
                    _ => unreachable!(),
                }
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        let shifted_logical_operation = (instruction >> 5) & 0xf;
        if instruction & 0xfe00 == 0xea00 && shifted_logical_operation <= 4 {
            let source = usize::from(instruction & 0xf);
            let destination = usize::from((second >> 8) & 0xf);
            let right_register = usize::from(second & 0xf);
            let shift_amount = u32::from(((second >> 12) & 7) << 2 | ((second >> 6) & 3));
            let right = match (second >> 4) & 3 {
                0 => self.registers[right_register].wrapping_shl(shift_amount),
                1 => self.registers[right_register]
                    .checked_shr(if shift_amount == 0 { 32 } else { shift_amount })
                    .unwrap_or(0),
                2 => {
                    ((self.registers[right_register] as i32)
                        .checked_shr(if shift_amount == 0 { 32 } else { shift_amount })
                        .unwrap_or(if self.registers[right_register] & N == 0 {
                            0
                        } else {
                            -1
                        })) as u32
                }
                3 if shift_amount == 0 => {
                    (u32::from(self.xpsr & C != 0) << 31) | (self.registers[right_register] >> 1)
                }
                3 => self.registers[right_register].rotate_right(shift_amount),
                _ => unreachable!(),
            };
            let left = if source == 15 && matches!(shifted_logical_operation, 2 | 3) {
                0
            } else {
                self.registers[source]
            };
            let result = match shifted_logical_operation {
                0 => left & right,
                1 => left & !right,
                2 => left | right,
                3 => left | !right,
                4 => left ^ right,
                _ => unreachable!(),
            };
            if destination != 15 {
                self.registers[destination] = result;
            }
            if instruction & 0x10 != 0 {
                self.nz(result);
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        let multiple_operation = instruction & 0xffd0;
        if matches!(multiple_operation, 0xe880 | 0xe890 | 0xe900 | 0xe910) {
            let load = multiple_operation & 0x10 != 0;
            let decrement_before = multiple_operation & 0x0100 != 0;
            let writeback = instruction & 0x20 != 0;
            let base_register = usize::from(instruction & 0xf);
            let count = second.count_ones();
            let base = self.registers[base_register];
            let mut address = if decrement_before {
                base.wrapping_sub(count * 4)
            } else {
                base
            };
            let mut loaded_pc = None;
            for register in 0..16 {
                if second & (1 << register) == 0 {
                    continue;
                }
                if load {
                    let value =
                        self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                    if register == 15 {
                        loaded_pc = Some(value);
                    } else {
                        self.registers[register] = value;
                    }
                } else {
                    self.write(
                        bus,
                        address,
                        AccessWidth::Word,
                        self.registers[register],
                        now,
                    )?;
                }
                address = address.wrapping_add(4);
            }
            if writeback {
                self.registers[base_register] = if decrement_before {
                    base.wrapping_sub(count * 4)
                } else {
                    base.wrapping_add(count * 4)
                };
            }
            if let Some(target) = loaded_pc {
                self.branch_exchange(target, bus, now)?;
            } else {
                self.registers[15] = pc.wrapping_add(4);
            }
            return Ok(StepReason::Advanced);
        }
        let memory_operation = instruction & 0xfff0;
        if matches!(memory_operation, 0xf910 | 0xf930 | 0xf990 | 0xf9b0) {
            let base_register = usize::from(instruction & 0xf);
            let base = self.registers[base_register];
            let (address, writeback) = if instruction & 0x0080 != 0 {
                (base.wrapping_add(u32::from(second & 0x0fff)), None)
            } else if second & 0x0fc0 == 0 {
                let offset_register = usize::from(second & 0xf);
                let shift = u32::from((second >> 4) & 3);
                (
                    base.wrapping_add(self.registers[offset_register] << shift),
                    None,
                )
            } else {
                let offset = u32::from(second & 0xff);
                let adjusted = if second & 0x0200 != 0 {
                    base.wrapping_add(offset)
                } else {
                    base.wrapping_sub(offset)
                };
                (
                    if second & 0x0400 != 0 { adjusted } else { base },
                    (second & 0x0100 != 0).then_some(adjusted),
                )
            };
            let destination = usize::from((second >> 12) & 0xf);
            self.registers[destination] = if matches!(memory_operation, 0xf910 | 0xf990) {
                let value = self.read(bus, address, AccessWidth::Byte, AccessKind::Read, now)?;
                i32::from(value as u8 as i8) as u32
            } else {
                let value =
                    self.read(bus, address, AccessWidth::HalfWord, AccessKind::Read, now)?;
                i32::from(value as u16 as i16) as u32
            };
            if let Some(updated) = writeback {
                self.registers[base_register] = updated;
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(
            memory_operation,
            0xf800
                | 0xf810
                | 0xf820
                | 0xf830
                | 0xf840
                | 0xf850
                | 0xf880
                | 0xf890
                | 0xf8a0
                | 0xf8b0
                | 0xf8c0
                | 0xf8d0
        ) {
            let load = memory_operation & 0x10 != 0;
            let width = match memory_operation & 0x60 {
                0x00 => AccessWidth::Byte,
                0x20 => AccessWidth::HalfWord,
                0x40 => AccessWidth::Word,
                _ => unreachable!(),
            };
            let base_register = usize::from(instruction & 0xf);
            let transfer_register = usize::from((second >> 12) & 0xf);
            let base = self.registers[base_register];
            let (address, writeback) = if base_register == 15 {
                let aligned_pc = pc.wrapping_add(4) & !3;
                let offset = u32::from(second & 0x0fff);
                (
                    if instruction & 0x0080 != 0 {
                        aligned_pc.wrapping_add(offset)
                    } else {
                        aligned_pc.wrapping_sub(offset)
                    },
                    None,
                )
            } else if instruction & 0x0080 != 0 {
                (base.wrapping_add(u32::from(second & 0x0fff)), None)
            } else if second & 0x0fc0 == 0 {
                let offset_register = usize::from(second & 0xf);
                let shift = u32::from((second >> 4) & 3);
                (
                    base.wrapping_add(self.registers[offset_register] << shift),
                    None,
                )
            } else {
                let offset = u32::from(second & 0xff);
                let adjusted = if second & 0x0200 != 0 {
                    base.wrapping_add(offset)
                } else {
                    base.wrapping_sub(offset)
                };
                let address = if second & 0x0400 != 0 { adjusted } else { base };
                (address, (second & 0x0100 != 0).then_some(adjusted))
            };
            if load {
                let value = self.read(bus, address, width, AccessKind::Read, now)?;
                if transfer_register == 15 {
                    self.branch_exchange(value, bus, now)?;
                } else {
                    self.registers[transfer_register] = value;
                }
            } else {
                self.write(bus, address, width, self.registers[transfer_register], now)?;
            }
            if let Some(updated) = writeback {
                self.registers[base_register] = updated;
            }
            if !(load && transfer_register == 15) {
                self.registers[15] = pc.wrapping_add(4);
            }
            return Ok(StepReason::Advanced);
        }
        if instruction == 0xf3ef && second & 0xf000 == 0x8000 {
            let destination = usize::from((second >> 8) & 0xf);
            let system_register = second & 0xff;
            self.registers[destination] = match system_register {
                0 | 1 | 8 => self.registers[13],
                5 => self.xpsr & 0x1ff,
                6 => self.xpsr & 0x0700_fc00,
                7 => self.xpsr & 0x0700_fdff,
                0x10 => u32::from(self.primask),
                0x11 | 0x12 | 0x13 | 0x14 => 0,
                _ => {
                    return Err(self.fault(
                        CpuFaultKind::Unsupported,
                        format!("MRS system register {system_register:#04x} is not modeled"),
                    ));
                }
            };
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfff0 == 0xf380 && second & 0xff00 == 0x8800 {
            let source = usize::from(instruction & 0xf);
            let system_register = second & 0xff;
            match system_register {
                0 | 1 | 8 => self.registers[13] = self.registers[source],
                0x10 => self.primask = self.registers[source] & 1 != 0,
                0x11 | 0x12 | 0x13 | 0x14 => {}
                _ => {
                    return Err(self.fault(
                        CpuFaultKind::Unsupported,
                        format!("MSR system register {system_register:#04x} is not modeled"),
                    ));
                }
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction == 0xf3bf && matches!(second, 0x8f4f | 0x8f5f | 0x8f6f) {
            // DSB, DMB, and ISB preserve ordering. The interpreter completes every bus
            // operation synchronously, so each architectural barrier is already satisfied.
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(instruction & 0xfbf0, 0xf240 | 0xf2c0) && second & 0x8000 == 0 {
            let destination = usize::from((second >> 8) & 0xf);
            let immediate = (u32::from(instruction & 0xf) << 12)
                | (u32::from((instruction >> 10) & 1) << 11)
                | (u32::from((second >> 12) & 7) << 8)
                | u32::from(second & 0xff);
            if instruction & 0x0080 == 0 {
                self.registers[destination] = immediate;
            } else {
                self.registers[destination] =
                    (self.registers[destination] & 0xffff) | (immediate << 16);
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if matches!(instruction & 0xfbf0, 0xf200 | 0xf2a0) && second & 0x8000 == 0 {
            let source_register = usize::from(instruction & 0xf);
            let source = if source_register == 15 {
                // ADDW/SUBW with PC is the ADR form and observes the architecturally
                // visible, word-aligned PC rather than the current instruction address.
                pc.wrapping_add(4) & !3
            } else {
                self.registers[source_register]
            };
            let destination = usize::from((second >> 8) & 0xf);
            let immediate = (u32::from((instruction >> 10) & 1) << 11)
                | (u32::from((second >> 12) & 7) << 8)
                | u32::from(second & 0xff);
            self.registers[destination] = if instruction & 0x0080 != 0 {
                source.wrapping_sub(immediate)
            } else {
                source.wrapping_add(immediate)
            };
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfbef == 0xf06f && second & 0x8000 == 0 {
            let destination = usize::from((second >> 8) & 0xf);
            let immediate = (u32::from((instruction >> 10) & 1) << 11)
                | (u32::from((second >> 12) & 7) << 8)
                | u32::from(second & 0xff);
            let result = !thumb_expand_immediate(immediate);
            self.registers[destination] = result;
            self.nz(result);
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfbef == 0xf04f && second & 0x8000 == 0 {
            let destination = usize::from((second >> 8) & 0xf);
            let immediate = (u32::from((instruction >> 10) & 1) << 11)
                | (u32::from((second >> 12) & 7) << 8)
                | u32::from(second & 0xff);
            let result = thumb_expand_immediate(immediate);
            self.registers[destination] = result;
            if instruction & 0x10 != 0 {
                self.nz(result);
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        let arithmetic_operation = instruction & 0xfbe0;
        if matches!(
            arithmetic_operation,
            0xf100 | 0xf140 | 0xf160 | 0xf1a0 | 0xf1c0
        ) && second & 0x8000 == 0
        {
            let source = usize::from(instruction & 0xf);
            let destination = usize::from((second >> 8) & 0xf);
            let immediate = (u32::from((instruction >> 10) & 1) << 11)
                | (u32::from((second >> 12) & 7) << 8)
                | u32::from(second & 0xff);
            let operand = thumb_expand_immediate(immediate);
            let left = self.registers[source];
            let carry = u32::from(self.xpsr & C != 0);
            let result = match arithmetic_operation {
                0xf100 => left.wrapping_add(operand),
                0xf140 => left.wrapping_add(operand).wrapping_add(carry),
                0xf160 => left.wrapping_sub(operand).wrapping_sub(1 - carry),
                0xf1a0 => left.wrapping_sub(operand),
                0xf1c0 => operand.wrapping_sub(left),
                _ => unreachable!(),
            };
            self.registers[destination] = result;
            if instruction & 0x10 != 0 {
                match arithmetic_operation {
                    0xf100 => self.add_flags(left, operand, result),
                    0xf140 => {
                        self.nz(result);
                        self.xpsr &= !(C | V);
                        if u64::from(left) + u64::from(operand) + u64::from(carry)
                            > u64::from(u32::MAX)
                        {
                            self.xpsr |= C;
                        }
                        let signed =
                            i64::from(left as i32) + i64::from(operand as i32) + i64::from(carry);
                        if signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX) {
                            self.xpsr |= V;
                        }
                    }
                    0xf160 => {
                        self.nz(result);
                        self.xpsr &= !(C | V);
                        let subtrahend = u64::from(operand) + u64::from(1 - carry);
                        if u64::from(left) >= subtrahend {
                            self.xpsr |= C;
                        }
                        let signed = i64::from(left as i32)
                            - i64::from(operand as i32)
                            - i64::from(1 - carry);
                        if signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX) {
                            self.xpsr |= V;
                        }
                    }
                    0xf1a0 => self.sub_flags(left, operand, result),
                    0xf1c0 => self.sub_flags(operand, left, result),
                    _ => unreachable!(),
                }
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        let logical_operation = (instruction >> 5) & 0xf;
        if instruction & 0xfa00 == 0xf000 && logical_operation <= 4 && second & 0x8000 == 0 {
            let source = usize::from(instruction & 0xf);
            let destination = usize::from((second >> 8) & 0xf);
            let immediate = (u32::from((instruction >> 10) & 1) << 11)
                | (u32::from((second >> 12) & 7) << 8)
                | u32::from(second & 0xff);
            let operand = thumb_expand_immediate(immediate);
            let left = self.registers[source];
            let result = match logical_operation {
                0 => left & operand,
                1 => left & !operand,
                2 => left | operand,
                3 => left | !operand,
                4 => left ^ operand,
                _ => unreachable!(),
            };
            if destination != 15 {
                self.registers[destination] = result;
            }
            if instruction & 0x10 != 0 {
                self.nz(result);
            }
            self.registers[15] = pc.wrapping_add(4);
            return Ok(StepReason::Advanced);
        }
        let branch_kind = second & 0xd000;
        if instruction & 0xf800 == 0xf000 && branch_kind == 0x8000 {
            let condition = (instruction >> 6) & 0xf;
            if condition >= 0xe {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    "invalid condition in 32-bit Thumb branch",
                ));
            }
            let encoded = (u32::from((instruction >> 10) & 1) << 20)
                | (u32::from((second >> 11) & 1) << 19)
                | (u32::from((second >> 13) & 1) << 18)
                | (u32::from(instruction & 0x3f) << 12)
                | (u32::from(second & 0x7ff) << 1);
            self.registers[15] = if self.condition(condition) {
                pc.wrapping_add(4)
                    .wrapping_add_signed(sign_extend(encoded, 21))
            } else {
                pc.wrapping_add(4)
            };
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xf800 != 0xf000 || !matches!(branch_kind, 0x9000 | 0xd000) {
            return Err(self.fault(
                CpuFaultKind::IllegalInstruction,
                "unsupported 32-bit Thumb encoding",
            ));
        }
        let s = u32::from((instruction >> 10) & 1);
        let j1 = u32::from((second >> 13) & 1);
        let j2 = u32::from((second >> 11) & 1);
        let i1 = (!(j1 ^ s)) & 1;
        let i2 = (!(j2 ^ s)) & 1;
        let encoded = (s << 24)
            | (i1 << 23)
            | (i2 << 22)
            | (u32::from(instruction & 0x03ff) << 12)
            | (u32::from(second & 0x07ff) << 1);
        if branch_kind == 0xd000 {
            self.registers[14] = pc.wrapping_add(4) | 1;
        }
        self.registers[15] = pc
            .wrapping_add(4)
            .wrapping_add_signed(sign_extend(encoded, 25));
        return Ok(StepReason::Advanced);
    }
}
