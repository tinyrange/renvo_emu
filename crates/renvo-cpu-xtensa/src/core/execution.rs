use super::*;

impl XtensaCpu {
    pub(super) fn execute_narrow(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let op = instruction & 0xf;
        let next = self.pc.wrapping_add(2);
        if op == 0xc && instruction & 0x0080 != 0 {
            let register = usize::from((instruction >> 8) & 0xf);
            let encoded =
                u32::from((instruction >> 12) & 0xf) | (u32::from((instruction >> 4) & 3) << 4);
            let branch_if_nonzero = instruction & 0x0040 != 0;
            let condition = (self.registers[register] != 0) == branch_if_nonzero;
            self.pc = if condition {
                self.pc.wrapping_add(4).wrapping_add(encoded)
            } else {
                next
            };
            return Ok(StepReason::Advanced);
        }
        match op {
            0x8 | 0x9 => {
                let register = usize::from((instruction >> 4) & 0xf);
                let base = usize::from((instruction >> 8) & 0xf);
                let address =
                    self.registers[base].wrapping_add(u32::from((instruction >> 12) & 0xf) << 2);
                if op == 0x8 {
                    self.registers[register] =
                        self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                } else {
                    self.write(
                        bus,
                        address,
                        AccessWidth::Word,
                        self.registers[register],
                        now,
                    )?;
                }
            }
            0xa => {
                let destination = usize::from((instruction >> 12) & 0xf);
                let left = usize::from((instruction >> 8) & 0xf);
                let right = usize::from((instruction >> 4) & 0xf);
                self.registers[destination] =
                    self.registers[left].wrapping_add(self.registers[right]);
            }
            0xb => {
                let destination = usize::from((instruction >> 12) & 0xf);
                let source = usize::from((instruction >> 8) & 0xf);
                let encoded = u32::from((instruction >> 4) & 0xf);
                let immediate = if encoded == 0 { u32::MAX } else { encoded };
                self.registers[destination] = self.registers[source].wrapping_add(immediate);
            }
            0xc if instruction & 0x0080 == 0 => {
                let destination = usize::from((instruction >> 8) & 0xf);
                let encoded =
                    (u32::from((instruction >> 4) & 7) << 4) | u32::from((instruction >> 12) & 0xf);
                // MOVI.N's compact simm7 encoding reserves 0x40..0x5f for
                // positive 64..95; only encodings with both high bits set are
                // negative.
                self.registers[destination] = if encoded & 0x60 == 0x60 {
                    encoded | !0x7f
                } else {
                    encoded
                };
            }
            0xd if instruction == 0xf03d => {}
            0xd if instruction == 0xf00d => {
                self.pc = self.registers[0];
                return Ok(StepReason::Advanced);
            }
            0xd => {
                let destination = usize::from((instruction >> 4) & 0xf);
                let source = usize::from((instruction >> 8) & 0xf);
                self.registers[destination] = self.registers[source];
            }
            _ => {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    format!("Xtensa density instruction {instruction:#06x} is not implemented"),
                ));
            }
        }
        self.pc = next;
        Ok(StepReason::Advanced)
    }

    pub(super) fn execute_wide(
        &mut self,
        instruction: u32,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let next = self.pc.wrapping_add(3);
        if instruction == 0x0000_0090 {
            self.window_return()?;
            return Ok(StepReason::Advanced);
        }
        // RFE returns from a level-one exception using EPC1 and current PS. FreeRTOS
        // writes the saved task PS directly before this instruction; RFE
        // clears EXCM while retaining that value. This is also how a fresh
        // task's synthetic CALLINC reaches its first ENTRY.
        if instruction == 0x0000_3000 {
            self.pc = self.special_registers[177];
            self.ps &= !0x10;
            let (registers, window_stack) = {
                let contexts = self
                    .task_contexts
                    .lock()
                    .expect("Xtensa task-context lock poisoned");
                (
                    contexts.registers.get(&self.thread_pointer).copied(),
                    contexts
                        .window_stacks
                        .get(&self.thread_pointer)
                        .cloned()
                        .unwrap_or_default(),
                )
            };
            if let Some(registers) = registers {
                self.registers = registers;
            }
            self.window_stack = window_stack;
            if std::env::var_os("RENVO_DEBUG_XTENSA_CONTEXT").is_some() {
                eprintln!(
                    "rfe pc={:#010x} ps={:#010x} tp={:#010x} depth={} a2={:#010x} a6={:#010x}",
                    self.pc,
                    self.ps,
                    self.thread_pointer,
                    self.window_stack.len(),
                    self.registers[2],
                    self.registers[6],
                );
            }
            return Ok(StepReason::Advanced);
        }
        // WAITI atomically establishes an interrupt level and sleeps until an
        // enabled interrupt is observed.
        if instruction == 0x0000_7000 {
            self.ps &= !0xf;
            self.waiting = true;
            self.pc = next;
            return Ok(StepReason::WaitForInterrupt);
        }
        // ROTW changes the physical register-window base while FreeRTOS
        // flushes an interrupted task. Renvo preserves each task's logical
        // window frames separately, so the physical rotation has no further
        // visible effect in the functional register model.
        if instruction & 0x00ff_ff0f == 0x0040_8000 {
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // XSR atomically exchanges a general and special register.
        if instruction & 0x00ff_000f == 0x0061_0000 {
            let register = usize::from(((instruction >> 4) & 0xf) as u8);
            let special = usize::from(((instruction >> 8) & 0xff) as u8);
            let previous = match special {
                3 => self.sar,
                230 => self.ps,
                _ => self.special_registers[special],
            };
            let value = self.registers[register];
            match special {
                3 => self.sar = value & 0x1f,
                230 => self.ps = value,
                _ => self.special_registers[special] = value,
            }
            self.registers[register] = previous;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // THREADPTR is a user register (UR 231), accessed with RUR/WUR
        // rather than the ordinary special-register opcodes. FreeRTOS saves
        // and restores it on every task switch for newlib/TLS state.
        if instruction & 0x00ff_ff0f == 0x00f3_e700 {
            let register = usize::from(((instruction >> 4) & 0xf) as u8);
            let next_thread_pointer = self.registers[register];
            // Interrupt returns restore the logical window state at RFE after
            // the guest context frame has selected THREADPTR. A solicited
            // FreeRTOS yield has no RFE. Its dispatcher marks the compact
            // solicited frame with a zero first word, so switch the host-side
            // logical context only for that path.
            let solicited_frame = self
                .read(
                    bus,
                    self.registers[1],
                    AccessWidth::Word,
                    AccessKind::Read,
                    now,
                )
                .is_ok_and(|marker| marker == 0);
            if next_thread_pointer != self.thread_pointer && solicited_frame {
                let (restored_registers, restored_windows) = {
                    let contexts = self
                        .task_contexts
                        .lock()
                        .expect("Xtensa task-context lock poisoned");
                    (
                        contexts.registers.get(&next_thread_pointer).copied(),
                        contexts.window_stacks.get(&next_thread_pointer).cloned(),
                    )
                };
                if let Some(restored_registers) = restored_registers {
                    self.registers = restored_registers;
                }
                if let Some(restored_windows) = restored_windows {
                    self.window_stack = restored_windows;
                }
            }
            self.thread_pointer = next_thread_pointer;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0x00ff_0fff == 0x00e3_0e70 {
            let register = usize::from(((instruction >> 12) & 0xf) as u8);
            self.registers[register] = self.thread_pointer;
            if self.ps & 0x10 == 0 {
                let mut contexts = self
                    .task_contexts
                    .lock()
                    .expect("Xtensa task-context lock poisoned");
                contexts
                    .window_stacks
                    .insert(self.thread_pointer, self.window_stack.clone());
                contexts
                    .registers
                    .insert(self.thread_pointer, self.registers);
            }
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // L32E/S32E address the exception spill area at offsets -64..-4.
        // The compact immediate is a four-byte slot biased by 64 bytes.
        if instruction & 0x00ff_000f == 0x0009_0000 {
            let destination = usize::from(((instruction >> 4) & 0xf) as u8);
            let base = usize::from(((instruction >> 8) & 0xf) as u8);
            let offset = (((instruction >> 12) & 0xf) as i32) * 4 - 64;
            let address = self.registers[base].wrapping_add_signed(offset);
            self.registers[destination] =
                self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0x00ff_000f == 0x0049_0000 {
            let source = usize::from(((instruction >> 4) & 0xf) as u8);
            let base = usize::from(((instruction >> 8) & 0xf) as u8);
            let offset = (((instruction >> 12) & 0xf) as i32) * 4 - 64;
            let address = self.registers[base].wrapping_add_signed(offset);
            self.write(bus, address, AccessWidth::Word, self.registers[source], now)?;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // RSR/WSR use an eight-bit special-register number. Keep the full
        // architected bank so startup code can establish VECBASE, EXCSAVE and
        // window state without target-specific CPU shortcuts.
        if instruction & 0x00ff_000f == 0x0003_0000 {
            let destination = usize::from(((instruction >> 4) & 0xf) as u8);
            let special = usize::from(((instruction >> 8) & 0xff) as u8);
            self.registers[destination] = match special {
                3 => self.sar,
                230 => self.ps,
                _ => self.special_registers[special],
            };
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // Functional address spaces are already coherent, so ITLB/DTLB write
        // instructions are acknowledged as maintenance operations.
        if instruction & 0x00ff_000f == 0x0050_0000 {
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0x00ff_000f == 0x0013_0000 {
            let source = usize::from(((instruction >> 4) & 0xf) as u8);
            let special = usize::from(((instruction >> 8) & 0xff) as u8);
            let value = self.registers[source];
            match special {
                3 => self.sar = value & 0x1f,
                230 => self.ps = value,
                // Guest-written software interrupts coexist with externally
                // asserted peripheral lines. Machine polling must not erase
                // an INTSET bit before the following instruction can take it.
                226 => {
                    self.software_interrupts |= value;
                    self.special_registers[226] |= value;
                }
                227 => {
                    self.software_interrupts &= !value;
                    self.special_registers[226] &= !value;
                }
                _ => self.special_registers[special] = value,
            }
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // RSIL returns PS while atomically replacing its interrupt level.
        if instruction & 0x00ff_f00f == 0x0000_6000 {
            let destination = usize::from(((instruction >> 4) & 0xf) as u8);
            let level = (instruction >> 8) & 0xf;
            self.registers[destination] = self.ps;
            self.ps = (self.ps & !0xf) | level;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // ADDI at, as, imm8 (RRI8).
        if instruction & 0x0000_f00f == 0x0000_c002 {
            let destination = usize::from(((instruction >> 4) & 0xf) as u8);
            let source = usize::from(((instruction >> 8) & 0xf) as u8);
            let immediate = sign_extend((instruction >> 16) & 0xff, 8);
            self.registers[destination] = self.registers[source].wrapping_add_signed(immediate);
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // Direct CALL0/4/8/12 share the aligned-PC displacement encoding.
        if matches!(instruction & 0x3f, 0x05 | 0x15 | 0x25 | 0x35) {
            let displacement = sign_extend((instruction >> 6) & 0x3ffff, 18);
            let target = (self.pc & !3)
                .wrapping_add(4)
                .wrapping_add_signed(displacement.wrapping_mul(4));
            let call_increment = usize::try_from((instruction >> 4) & 3).unwrap_or_default() * 4;
            if call_increment == 0 {
                self.registers[0] = next;
                self.pc = target;
            } else {
                self.window_call(call_increment, target, next);
            }
            return Ok(StepReason::Advanced);
        }
        let op2_op1 = (instruction >> 16) & 0xff;
        let destination = usize::from(((instruction >> 12) & 0xf) as u8);
        let source = usize::from(((instruction >> 8) & 0xf) as u8);
        let right = usize::from(((instruction >> 4) & 0xf) as u8);
        // ESP32-S3 single-precision coprocessor. Registers retain their raw
        // IEEE-754 payload so integer transfers, NaNs and signed zero remain
        // bit exact; host arithmetic is used for the functional result.
        if instruction & 0xf == 0 && matches!(op2_op1, 0xca | 0xda) {
            let scale = i32::try_from(right).expect("four-bit scale fits");
            let value = if op2_op1 == 0xca {
                self.registers[source] as i32 as f32
            } else {
                self.registers[source] as f32
            };
            self.floating_registers[destination] = (value * 2_f32.powi(-scale)).to_bits();
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // WFR/RFR move raw payloads between address and float registers.
        if instruction & 0xff == 0x50 && op2_op1 == 0xfa {
            self.floating_registers[destination] = self.registers[source];
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xff == 0x40 && op2_op1 == 0xfa {
            self.registers[destination] = self.floating_registers[source];
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xff == 0x00 && op2_op1 == 0xfa {
            self.floating_registers[destination] = self.floating_registers[source];
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xff == 0x10 && op2_op1 == 0xfa {
            self.floating_registers[destination] = f32::from_bits(self.floating_registers[source])
                .abs()
                .to_bits();
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xff == 0x60 && op2_op1 == 0xfa {
            self.floating_registers[destination] = self.floating_registers[source] ^ (1 << 31);
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // CONST.S selects one of the four architectural single-precision
        // constants using the source field. Encodings 4..15 are reserved.
        if instruction & 0xff == 0x30 && op2_op1 == 0xfa && source < 4 {
            const VALUES: [u32; 4] = [
                0.0_f32.to_bits(),
                1.0_f32.to_bits(),
                2.0_f32.to_bits(),
                0.5_f32.to_bits(),
            ];
            self.floating_registers[destination] = VALUES[source];
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xf == 0 && matches!(op2_op1, 0x9a | 0xea) {
            let scale = i32::try_from(right).expect("four-bit scale fits");
            let value = f32::from_bits(self.floating_registers[source]) * 2_f32.powi(scale);
            self.registers[destination] = if op2_op1 == 0x9a {
                (value as i32) as u32
            } else {
                value as u32
            };
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xf == 0 && matches!(op2_op1, 0x0a | 0x1a | 0x2a) {
            let left_value = f32::from_bits(self.floating_registers[source]);
            let right_value = f32::from_bits(self.floating_registers[right]);
            let result = match op2_op1 {
                0x0a => left_value + right_value,
                0x1a => left_value - right_value,
                0x2a => left_value * right_value,
                _ => unreachable!(),
            };
            self.floating_registers[destination] = result.to_bits();
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xf == 0 && matches!(op2_op1, 0x4a | 0x5a | 0x6a | 0x7a) {
            let accumulator = f32::from_bits(self.floating_registers[destination]);
            let left_value = f32::from_bits(self.floating_registers[source]);
            let right_value = f32::from_bits(self.floating_registers[right]);
            // MADDN.S differs only in its rounding-mode override, which is
            // host round-to-nearest in the functional model. DIVN.S is the
            // final multiply-add of Xtensa's expanded divide/square-root
            // sequence; ordinary IEEE arithmetic gives the intended
            // functional result once its operands have been prepared.
            let result = if op2_op1 == 0x5a {
                accumulator - left_value * right_value
            } else {
                accumulator + left_value * right_value
            };
            self.floating_registers[destination] = result.to_bits();
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // Single-precision comparisons write one boolean register. The
        // "unordered" variants include a NaN operand in their true set.
        if instruction & 0xf == 0
            && matches!(op2_op1, 0x1b | 0x2b | 0x3b | 0x4b | 0x5b | 0x6b | 0x7b)
        {
            let left_value = f32::from_bits(self.floating_registers[source]);
            let right_value = f32::from_bits(self.floating_registers[right]);
            let unordered = left_value.is_nan() || right_value.is_nan();
            let result = match op2_op1 {
                0x1b => unordered,
                0x2b => left_value == right_value,
                0x3b => unordered || left_value == right_value,
                0x4b => left_value < right_value,
                0x5b => unordered || left_value < right_value,
                0x6b => left_value <= right_value,
                0x7b => unordered || left_value <= right_value,
                _ => unreachable!(),
            };
            let mask = 1_u16 << destination;
            if result {
                self.boolean_registers |= mask;
            } else {
                self.boolean_registers &= !mask;
            }
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // Integer-conditioned single-precision moves.
        if instruction & 0xf == 0 && matches!(op2_op1, 0x8b | 0x9b | 0xab | 0xbb) {
            let condition_value = self.registers[right] as i32;
            let move_value = match op2_op1 {
                0x8b => condition_value == 0,
                0x9b => condition_value != 0,
                0xab => condition_value < 0,
                0xbb => condition_value >= 0,
                _ => unreachable!(),
            };
            if move_value {
                self.floating_registers[destination] = self.floating_registers[source];
            }
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // LSI/SSI use a word-scaled unsigned byte displacement.
        if instruction & 0xf == 3 && instruction & 0x0000_3000 == 0 {
            let float_register = right;
            let address =
                self.registers[source].wrapping_add(((instruction >> 16) & 0xff).wrapping_mul(4));
            if instruction & 0x0000_4000 == 0 {
                self.floating_registers[float_register] =
                    self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
            } else {
                self.write(
                    bus,
                    address,
                    AccessWidth::Word,
                    self.floating_registers[float_register],
                    now,
                )?;
            }
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // Architectural synchronization instructions share enough low bits
        // with MOVSP to match its compact field pattern. Decode them first:
        // they order hardware state, but have no additional effect in this
        // deterministic interpreter.
        if matches!(
            instruction,
            0x0000_20c0 | 0x0000_2010 | 0x0000_2000 | 0x0000_2020 | 0x0000_2030
        ) {
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // MOVSP is a constrained stack-pointer move used for variable-sized
        // windowed-ABI frames. The functional model does not need the
        // hardware window-overflow check, but must preserve its register move.
        if instruction & 0x00ff_00ff == 0x0000_0010 {
            self.registers[destination] = self.registers[source];
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // MULA.AA.LL accumulates the signed product of the low halfwords
        // into Xtensa's 40-bit ACC register. GCC selects this instruction for
        // CoreMark's signed 16-bit matrix dot product.
        if instruction & 0xf == 4 && op2_op1 == 0x78 {
            const ACC_MASK: u64 = (1_u64 << 40) - 1;
            let accumulator = (u64::from(self.special_registers[17] & 0xff) << 32)
                | u64::from(self.special_registers[16]);
            let product =
                i64::from(self.registers[source] as i16) * i64::from(self.registers[right] as i16);
            let result = accumulator.wrapping_add(product as u64) & ACC_MASK;
            self.special_registers[16] = result as u32;
            self.special_registers[17] = (result >> 32) as u32;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        let rrr_result = match op2_op1 {
            0x10 => Some(self.registers[source] & self.registers[right]),
            0x20 => Some(self.registers[source] | self.registers[right]),
            0x30 => Some(self.registers[source] ^ self.registers[right]),
            // NEG ar, at is encoded in the RRR group with the unused source
            // field set to a0.
            0x60 => Some(0_u32.wrapping_sub(self.registers[right])),
            0x80 => Some(self.registers[source].wrapping_add(self.registers[right])),
            0x90 => Some(
                self.registers[source]
                    .wrapping_shl(1)
                    .wrapping_add(self.registers[right]),
            ),
            0xa0 => Some(
                self.registers[source]
                    .wrapping_shl(2)
                    .wrapping_add(self.registers[right]),
            ),
            0xb0 => Some(
                self.registers[source]
                    .wrapping_shl(3)
                    .wrapping_add(self.registers[right]),
            ),
            0xc0 => Some(self.registers[source].wrapping_sub(self.registers[right])),
            0xd0 => Some(
                self.registers[source]
                    .wrapping_shl(1)
                    .wrapping_sub(self.registers[right]),
            ),
            0xe0 => Some(
                self.registers[source]
                    .wrapping_shl(2)
                    .wrapping_sub(self.registers[right]),
            ),
            0xf0 => Some(
                self.registers[source]
                    .wrapping_shl(3)
                    .wrapping_sub(self.registers[right]),
            ),
            // SALTU
            0x62 => Some(u32::from(self.registers[source] < self.registers[right])),
            0x72 => Some(u32::from(
                (self.registers[source] as i32) < (self.registers[right] as i32),
            )),
            0x43 => Some((self.registers[source] as i32).min(self.registers[right] as i32) as u32),
            0x53 => Some((self.registers[source] as i32).max(self.registers[right] as i32) as u32),
            0x63 => Some(self.registers[source].min(self.registers[right])),
            0x73 => Some(self.registers[source].max(self.registers[right])),
            0x82 => Some(self.registers[source].wrapping_mul(self.registers[right])),
            0xc1 => {
                Some((self.registers[source] & 0xffff).wrapping_mul(self.registers[right] & 0xffff))
            }
            0xd1 => Some(
                i32::from(self.registers[source] as i16)
                    .wrapping_mul(i32::from(self.registers[right] as i16)) as u32,
            ),
            0xa2 => Some(
                ((u64::from(self.registers[source]) * u64::from(self.registers[right])) >> 32)
                    as u32,
            ),
            0xb2 => Some(
                (((i64::from(self.registers[source] as i32)
                    * i64::from(self.registers[right] as i32))
                    >> 32) as i32) as u32,
            ),
            0xc2 if self.registers[right] != 0 => {
                Some(self.registers[source] / self.registers[right])
            }
            0xd2 if self.registers[right] != 0 => {
                let numerator = self.registers[source] as i32;
                let denominator = self.registers[right] as i32;
                Some(numerator.checked_div(denominator).unwrap_or(i32::MIN) as u32)
            }
            0xe2 if self.registers[right] != 0 => {
                Some(self.registers[source] % self.registers[right])
            }
            0xf2 if self.registers[right] != 0 => {
                let numerator = self.registers[source] as i32;
                let denominator = self.registers[right] as i32;
                Some(numerator.checked_rem(denominator).unwrap_or_default() as u32)
            }
            _ => None,
        };
        if instruction & 0xf == 0
            && let Some(result) = rrr_result
        {
            self.registers[destination] = result;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // SEXT ar, as, imm sign-extends bit 7..22. GCC uses this for explicit
        // byte and halfword promotion.
        if instruction & 0x00ff_000f == 0x0023_0000 {
            let bits = u32::try_from(right).unwrap() + 8;
            self.registers[destination] = sign_extend(self.registers[source], bits) as u32;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // NSA/NSAU return the normalization shift amount. The unsigned form is
        // the one emitted by the current corpus.
        if instruction & 0x00ff_f00f == 0x0040_f000 {
            self.registers[right] = self.registers[source].leading_zeros();
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // Conditional moves from the core RRR group.
        if instruction & 0xf == 0 && matches!(op2_op1, 0x83 | 0x93 | 0xa3 | 0xb3) {
            let condition = match op2_op1 {
                0x83 => self.registers[right] == 0,
                0x93 => self.registers[right] != 0,
                0xa3 => (self.registers[right] as i32) < 0,
                0xb3 => (self.registers[right] as i32) >= 0,
                _ => unreachable!(),
            };
            if condition {
                self.registers[destination] = self.registers[source];
            }
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // MOVF/MOVT condition a move on one of the sixteen boolean registers.
        if instruction & 0xf == 0 && matches!(op2_op1, 0xc3 | 0xd3) {
            let boolean = self.boolean_registers & (1 << right) != 0;
            if boolean == (op2_op1 == 0xd3) {
                self.registers[destination] = self.registers[source];
            }
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // EXTUI ar, at, shift, width.
        if instruction & 0x000e_000f == 0x0004_0000 {
            let width = ((instruction >> 20) & 0xf) + 1;
            let shift = (instruction >> 8) & 0xf | (((instruction >> 16) & 1) << 4);
            let mask = if width == 32 {
                u32::MAX
            } else {
                (1_u32 << width) - 1
            };
            self.registers[destination] = (self.registers[right] >> shift) & mask;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // Immediate shift forms.
        if instruction & 0x00ef_000f == 0x0001_0000 {
            let encoded = (instruction >> 4) & 0xf | (((instruction >> 20) & 1) << 4);
            let shift = (32 - encoded) & 0x1f;
            self.registers[destination] = self.registers[source].wrapping_shl(shift);
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0x00ff_000f == 0x0041_0000 {
            let shift = (instruction >> 8) & 0xf;
            self.registers[destination] = self.registers[right].wrapping_shr(shift);
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0x00ef_000f == 0x0021_0000 {
            let shift = ((instruction >> 8) & 0xf) | (((instruction >> 20) & 1) << 4);
            self.registers[destination] = ((self.registers[right] as i32) >> shift) as u32;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // SSAI stores a 5-bit amount for SRC and the variable shifts.
        if instruction & 0x00ff_f00f == 0x0040_4000 {
            self.sar = ((instruction >> 8) & 0xf) | (((instruction >> 4) & 1) << 4);
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // SSA8B prepares SLL/SRC to place a selected byte at the
        // least-significant end of a word.
        if instruction & 0x00ff_f0ff == 0x0040_3000 {
            let shift = (self.registers[source] & 3) * 8;
            self.sar = (32 - shift) & 0x1f;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // Register-controlled shift setup and execution.
        if instruction & 0x00ff_f0ff == 0x0040_1000 {
            self.sar = (32 - (self.registers[source] & 0x1f)) & 0x1f;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0x00ff_f0ff == 0x0040_0000 {
            self.sar = self.registers[source] & 0x1f;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0x00ff_0f0f == 0x0091_0000 {
            self.registers[destination] = self.registers[right].wrapping_shr(self.sar & 0x1f);
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0x00ff_0f0f == 0x00b1_0000 {
            self.registers[destination] =
                ((self.registers[right] as i32) >> (self.sar & 0x1f)) as u32;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // SLL consumes the shift amount previously established by SSL.
        if instruction & 0x00ff_000f == 0x00a1_0000 {
            let shift = (32 - self.sar) & 0x1f;
            self.registers[destination] = self.registers[source].wrapping_shl(shift);
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0x00ff_000f == 0x0081_0000 {
            let shift = self.sar & 0x1f;
            self.registers[destination] = if shift == 0 {
                self.registers[source]
            } else {
                (self.registers[right] >> shift) | self.registers[source].wrapping_shl(32 - shift)
            };
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        // RRI8 memory and MOVI forms.
        if instruction & 0xf == 2 {
            let operation = (instruction >> 12) & 0xf;
            let register = usize::from(((instruction >> 4) & 0xf) as u8);
            let base = usize::from(((instruction >> 8) & 0xf) as u8);
            let immediate = (instruction >> 16) & 0xff;
            let (width, scale, load) = match operation {
                0 => (AccessWidth::Byte, 1, true),
                1 => (AccessWidth::HalfWord, 2, true),
                2 => (AccessWidth::Word, 4, true),
                4 => (AccessWidth::Byte, 1, false),
                5 => (AccessWidth::HalfWord, 2, false),
                6 => (AccessWidth::Word, 4, false),
                9 => {
                    let address = self.registers[base].wrapping_add(immediate * 2);
                    let value =
                        self.read(bus, address, AccessWidth::HalfWord, AccessKind::Read, now)?;
                    self.registers[register] = sign_extend(value, 16) as u32;
                    self.pc = next;
                    return Ok(StepReason::Advanced);
                }
                0xa => {
                    let encoded = immediate | ((base as u32) << 8);
                    self.registers[register] = sign_extend(encoded, 12) as u32;
                    self.pc = next;
                    return Ok(StepReason::Advanced);
                }
                0xd => {
                    self.registers[register] = self.registers[base]
                        .wrapping_add_signed(sign_extend(immediate, 8).wrapping_mul(256));
                    self.pc = next;
                    return Ok(StepReason::Advanced);
                }
                0xe => {
                    let address = self.registers[base].wrapping_add(immediate * 4);
                    let current =
                        self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                    if current == self.special_registers[12] {
                        self.write(
                            bus,
                            address,
                            AccessWidth::Word,
                            self.registers[register],
                            now,
                        )?;
                    }
                    self.registers[register] = current;
                    self.pc = next;
                    return Ok(StepReason::Advanced);
                }
                _ => (AccessWidth::Byte, 0, false),
            };
            if scale != 0 {
                let address = self.registers[base].wrapping_add(immediate * scale);
                if load {
                    self.registers[register] =
                        self.read(bus, address, width, AccessKind::Read, now)?;
                } else {
                    self.write(bus, address, width, self.registers[register], now)?;
                }
                self.pc = next;
                return Ok(StepReason::Advanced);
            }
        }
        // BRI8 register and bit-test branches.
        if instruction & 0xf == 7 {
            let condition_code = (instruction >> 12) & 0xf;
            let s = usize::from(((instruction >> 8) & 0xf) as u8);
            let t = usize::from(((instruction >> 4) & 0xf) as u8);
            let condition = match condition_code {
                0 => self.registers[s] & self.registers[t] == 0,
                1 => self.registers[s] == self.registers[t],
                2 => (self.registers[s] as i32) < (self.registers[t] as i32),
                3 => self.registers[s] < self.registers[t],
                4 => self.registers[s] & self.registers[t] == self.registers[t],
                5 => self.registers[s] & (1 << (self.registers[t] & 31)) == 0,
                6 | 7 => {
                    let bit = u32::try_from(t).unwrap() | ((condition_code & 1) << 4);
                    self.registers[s] & (1 << bit) == 0
                }
                8 => self.registers[s] & self.registers[t] != 0,
                9 => self.registers[s] != self.registers[t],
                0xa => (self.registers[s] as i32) >= (self.registers[t] as i32),
                0xb => self.registers[s] >= self.registers[t],
                0xc => self.registers[s] & self.registers[t] != self.registers[t],
                0xd => self.registers[s] & (1 << (self.registers[t] & 31)) != 0,
                0xe | 0xf => {
                    let bit = u32::try_from(t).unwrap() | ((condition_code & 1) << 4);
                    self.registers[s] & (1 << bit) != 0
                }
                _ => unreachable!(),
            };
            self.pc = if condition {
                self.pc
                    .wrapping_add(4)
                    .wrapping_add_signed(sign_extend((instruction >> 16) & 0xff, 8))
            } else {
                next
            };
            return Ok(StepReason::Advanced);
        }
        // BRI8 immediate comparisons.
        if instruction & 0xf == 6
            && matches!((instruction >> 4) & 0xf, 2 | 6 | 0xa | 0xb | 0xe | 0xf)
        {
            const VALUES: [i32; 16] = [-1, 1, 2, 3, 4, 5, 6, 7, 8, 10, 12, 16, 32, 64, 128, 256];
            const UNSIGNED_VALUES: [u32; 16] = [
                32768, 65536, 2, 3, 4, 5, 6, 7, 8, 10, 12, 16, 32, 64, 128, 256,
            ];
            let operation = (instruction >> 4) & 0xf;
            let register = usize::from(((instruction >> 8) & 0xf) as u8);
            let constant = usize::from(((instruction >> 12) & 0xf) as u8);
            let condition = match operation {
                2 => self.registers[register] as i32 == VALUES[constant],
                6 => self.registers[register] as i32 != VALUES[constant],
                0xa => (self.registers[register] as i32) < VALUES[constant],
                0xb => self.registers[register] < UNSIGNED_VALUES[constant],
                0xe => (self.registers[register] as i32) >= VALUES[constant],
                0xf => self.registers[register] >= UNSIGNED_VALUES[constant],
                _ => unreachable!(),
            };
            self.pc = if condition {
                self.pc
                    .wrapping_add(4)
                    .wrapping_add_signed(sign_extend((instruction >> 16) & 0xff, 8))
            } else {
                next
            };
            return Ok(StepReason::Advanced);
        }
        // Zero-comparison BRI12 branches.
        if instruction & 0xf == 6 && matches!((instruction >> 4) & 0xf, 1 | 5 | 9 | 0xd) {
            let operation = (instruction >> 4) & 0xf;
            let register = usize::from(((instruction >> 8) & 0xf) as u8);
            let value = self.registers[register] as i32;
            let condition = match operation {
                1 => value == 0,
                5 => value != 0,
                9 => value < 0,
                0xd => value >= 0,
                _ => unreachable!(),
            };
            self.pc = if condition {
                self.pc
                    .wrapping_add(4)
                    .wrapping_add_signed(sign_extend((instruction >> 12) & 0xfff, 12))
            } else {
                next
            };
            return Ok(StepReason::Advanced);
        }
        // Boolean-register branches share their primary opcode with the
        // zero-overhead loop family. Bits 15:12 distinguish BF/BT (0/1)
        // from LOOP/LOOPNEZ/LOOPGTZ (8/9/10).
        if instruction & 0xff == 0x76 && matches!((instruction >> 12) & 0xf, 0 | 1) {
            let boolean_register = usize::from(((instruction >> 8) & 0xf) as u8);
            let branch_on_true = instruction & 0x0000_1000 != 0;
            let boolean = self.boolean_registers & (1 << boolean_register) != 0;
            self.pc = if boolean == branch_on_true {
                self.pc
                    .wrapping_add(4)
                    .wrapping_add_signed(sign_extend((instruction >> 16) & 0xff, 8))
            } else {
                next
            };
            return Ok(StepReason::Advanced);
        }
        // Zero-overhead loop setup. The post-step hook redirects sequential
        // completion at loop_end; conditional variants skip an empty loop.
        if instruction & 0xff == 0x76 && matches!((instruction >> 12) & 0xf, 8 | 9 | 0xa) {
            let operation = (instruction >> 12) & 0xf;
            let register = usize::from(((instruction >> 8) & 0xf) as u8);
            let count = self.registers[register];
            self.loop_begin = next;
            self.loop_end = self
                .pc
                .wrapping_add(4)
                .wrapping_add_signed(sign_extend((instruction >> 16) & 0xff, 8));
            let skip = operation == 9 && count == 0 || operation == 0xa && (count as i32) <= 0;
            if skip {
                self.loop_count = 0;
                self.pc = self.loop_end;
            } else {
                self.loop_count = count;
                self.pc = next;
            }
            return Ok(StepReason::Advanced);
        }
        // JX as.
        if instruction & 0x00ff_f0ff == 0x0000_00a0 {
            self.pc = self.registers[source];
            return Ok(StepReason::Advanced);
        }
        // CALLX0/4/8/12 use the same source field as JX.
        if instruction & 0x00ff_f00f == 0 && matches!(instruction & 0xff, 0xc0 | 0xd0 | 0xe0 | 0xf0)
        {
            let call_increment =
                usize::try_from(((instruction & 0xff) - 0xc0) >> 4).unwrap_or_default() * 4;
            let target = self.registers[source];
            if call_increment == 0 {
                self.registers[0] = next;
                self.pc = target;
            } else {
                self.window_call(call_increment, target, next);
            }
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xf == 0x1 {
            let destination =
                usize::try_from((instruction >> 4) & 0xf).expect("four-bit register fits usize");
            // L32R's imm16 denotes a word in the 256 KiB literal window
            // preceding the aligned next PC. It is a backwards-biased field,
            // not an ordinary signed i16.
            let displacement =
                i32::try_from((instruction >> 8) & 0xffff).unwrap_or_default() - 0x1_0000;
            let literal_base = self.pc.wrapping_add(3) & !3;
            let address = literal_base.wrapping_add_signed(displacement.wrapping_mul(4));
            self.registers[destination] =
                self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
            self.pc = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xf == 0x6 {
            if instruction & 0x3f == 0x06 {
                let displacement = sign_extend((instruction >> 6) & 0x3ffff, 18);
                self.pc = self.pc.wrapping_add(4).wrapping_add_signed(displacement);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0x0000_0fff == 0x0136 {
                let source =
                    usize::try_from((instruction >> 8) & 0xf).expect("register fits usize");
                let frame = ((instruction >> 12) & 0xfff) << 3;
                let source_value = self.registers[source];
                // A newly created FreeRTOS windowed-ABI task is restored as
                // if its entry point had been CALL4'd: PS.CALLINC is one and
                // the task argument occupies a6. No CALL instruction exists
                // in that path, so ENTRY performs the initial logical window
                // rotation here.
                let initial_call_increment =
                    usize::try_from((self.ps >> 16) & 3).expect("CALLINC fits usize") * 4;
                if self.window_stack.is_empty() && matches!(initial_call_increment, 4 | 8 | 12) {
                    let caller = self.registers;
                    let mut callee = [0_u32; 16];
                    callee[..16 - initial_call_increment]
                        .copy_from_slice(&caller[initial_call_increment..]);
                    self.registers = callee;
                    self.ps &= !(3 << 16);
                }
                // A real LX7 eventually spills older physical register
                // windows into the ABI-reserved 16-byte area at the top of a
                // callee frame. Materialize the caller's first four value
                // registers there eagerly. This is architecturally
                // equivalent for functional execution and, critically, lets
                // conservative stack scanners such as MicroPython's GC see
                // roots that would otherwise exist only in Renvo's logical
                // host-side window stack.
                let spill_values = self.window_stack.last().map(|caller| {
                    [
                        caller.registers[2],
                        caller.registers[3],
                        caller.registers[4],
                        caller.registers[5],
                    ]
                });
                if let Some(spill_values) = spill_values {
                    let spill_base = source_value.wrapping_sub(16);
                    for (index, value) in spill_values.into_iter().enumerate() {
                        self.write(
                            bus,
                            spill_base.wrapping_add((index as u32) * 4),
                            AccessWidth::Word,
                            value,
                            now,
                        )?;
                    }
                }
                self.registers[1] = source_value.wrapping_sub(frame);
                self.pc = next;
                return Ok(StepReason::Advanced);
            }
        }
        if instruction & 0x00ff_ff0f == 0x0000_4000 {
            self.pc = next;
            return Ok(StepReason::Breakpoint);
        }
        Err(self.fault(
            CpuFaultKind::IllegalInstruction,
            format!("Xtensa instruction {instruction:#08x} is not implemented"),
        ))
    }
}
