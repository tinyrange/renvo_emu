use super::{Pic16Cpu, STATUS_C, STATUS_PD, STATUS_TO};
use remu_core::{Bus, CpuFault, CpuFaultKind, SimDuration, SimTime, StepOutcome};

impl Pic16Cpu {
    pub(super) fn execute(
        &mut self,
        instruction: u16,
        instruction_pc: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepOutcome, CpuFault> {
        let file = instruction as u8 & 0x7f;
        let destination_file = instruction & 0x0080 != 0;
        let ordinary = SimDuration::from_ticks(1);
        let control = SimDuration::from_ticks(2);

        let elapsed = match instruction {
            0x0000 => ordinary, // NOP
            0x0001 => {
                self.reset_state();
                self.reset_requested = Some(remu_core::ResetKind::Software);
                control
            }
            0x0008 => {
                self.pc = self.pop();
                control
            }
            0x0009 => {
                self.pc = self.pop();
                self.restore_shadow();
                let intcon = self.read_bus(bus, 0x0b, now)? | 0x80;
                self.write_bus(bus, 0x0b, intcon, now)?;
                control
            }
            0x000a => {
                self.push(self.pc);
                self.pc = ((u16::from(self.pclath) << 8) | u16::from(self.wreg)) & 0x3fff;
                control
            }
            0x000b => {
                self.pc = self.pc.wrapping_add(u16::from(self.wreg)) & 0x3fff;
                control
            }
            0x0063 => {
                self.status = (self.status | STATUS_TO) & !STATUS_PD;
                self.waiting = true;
                ordinary
            }
            0x0064 => {
                self.status |= STATUS_TO | STATUS_PD;
                self.watchdog_clear_requested = true;
                ordinary
            }
            0x0103 => {
                self.wreg = 0;
                self.set_zero(0);
                ordinary
            }
            _ if instruction & 0x3ff8 == 0x0010 => {
                let index = usize::from((instruction >> 2) & 1);
                let mode = instruction & 3;
                if mode == 0 {
                    self.fsr[index] = self.fsr[index].wrapping_add(1);
                } else if mode == 1 {
                    self.fsr[index] = self.fsr[index].wrapping_sub(1);
                }
                self.wreg = self.read_indirect(index, bus, now)?;
                self.set_zero(self.wreg);
                if mode == 2 {
                    self.fsr[index] = self.fsr[index].wrapping_add(1);
                } else if mode == 3 {
                    self.fsr[index] = self.fsr[index].wrapping_sub(1);
                }
                ordinary
            }
            _ if instruction & 0x3ff8 == 0x0018 => {
                let index = usize::from((instruction >> 2) & 1);
                let mode = instruction & 3;
                if mode == 0 {
                    self.fsr[index] = self.fsr[index].wrapping_add(1);
                } else if mode == 1 {
                    self.fsr[index] = self.fsr[index].wrapping_sub(1);
                }
                self.write_indirect(index, self.wreg, bus, now)?;
                if mode == 2 {
                    self.fsr[index] = self.fsr[index].wrapping_add(1);
                } else if mode == 3 {
                    self.fsr[index] = self.fsr[index].wrapping_sub(1);
                }
                ordinary
            }
            _ if instruction & 0x3f80 == 0x3100 => {
                let index = usize::from((instruction >> 6) & 1);
                let offset = sign_extend(instruction & 0x3f, 6);
                self.fsr[index] = self.fsr[index].wrapping_add_signed(offset);
                ordinary
            }
            _ if instruction & 0x3f80 == 0x3180 => {
                self.pclath = instruction as u8 & 0x7f;
                ordinary
            }
            _ if instruction & 0x3e00 == 0x3200 => {
                let offset = sign_extend(instruction & 0x01ff, 9);
                self.pc = self.pc.wrapping_add_signed(offset) & 0x3fff;
                control
            }
            _ if instruction & 0x3f80 == 0x3f00 => {
                let index = usize::from((instruction >> 6) & 1);
                let offset = sign_extend(instruction & 0x3f, 6);
                let saved = self.fsr[index];
                self.fsr[index] = self.fsr[index].wrapping_add_signed(offset);
                self.wreg = self.read_indirect(index, bus, now)?;
                self.set_zero(self.wreg);
                self.fsr[index] = saved;
                ordinary
            }
            _ if instruction & 0x3f80 == 0x3f80 => {
                let index = usize::from((instruction >> 6) & 1);
                let offset = sign_extend(instruction & 0x3f, 6);
                let saved = self.fsr[index];
                self.fsr[index] = self.fsr[index].wrapping_add_signed(offset);
                self.write_indirect(index, self.wreg, bus, now)?;
                self.fsr[index] = saved;
                ordinary
            }
            _ if instruction & 0x3fc0 == 0x0140 => {
                self.bsr = instruction as u8 & 0x3f;
                ordinary
            }
            _ if instruction & 0x3c00 == 0x1000 => {
                let bit = ((instruction >> 7) & 7) as u8;
                let value = self.read_core(file, bus, now)? & !(1 << bit);
                self.write_core(file, value, bus, now)?;
                ordinary
            }
            _ if instruction & 0x3c00 == 0x1400 => {
                let bit = ((instruction >> 7) & 7) as u8;
                let value = self.read_core(file, bus, now)? | (1 << bit);
                self.write_core(file, value, bus, now)?;
                ordinary
            }
            _ if instruction & 0x3c00 == 0x1800 || instruction & 0x3c00 == 0x1c00 => {
                let bit = ((instruction >> 7) & 7) as u8;
                let set = self.read_core(file, bus, now)? & (1 << bit) != 0;
                let skip_if_set = instruction & 0x0400 != 0;
                if set == skip_if_set {
                    self.pc = self.pc.wrapping_add(1) & 0x3fff;
                    control
                } else {
                    ordinary
                }
            }
            _ if instruction & 0x3800 == 0x2000 => {
                self.push(self.pc);
                self.pc = ((u16::from(self.pclath & 0x78) << 8) | (instruction & 0x07ff)) & 0x3fff;
                control
            }
            _ if instruction & 0x3800 == 0x2800 => {
                self.pc = ((u16::from(self.pclath & 0x78) << 8) | (instruction & 0x07ff)) & 0x3fff;
                control
            }
            _ if instruction & 0x3f00 == 0x3000 => {
                self.wreg = instruction as u8;
                ordinary
            }
            _ if instruction & 0x3f00 == 0x3400 => {
                self.wreg = instruction as u8;
                self.pc = self.pop();
                control
            }
            _ if instruction & 0x3f00 == 0x3800 => {
                self.wreg |= instruction as u8;
                self.set_zero(self.wreg);
                ordinary
            }
            _ if instruction & 0x3f00 == 0x3900 => {
                self.wreg &= instruction as u8;
                self.set_zero(self.wreg);
                ordinary
            }
            _ if instruction & 0x3f00 == 0x3a00 => {
                self.wreg ^= instruction as u8;
                self.set_zero(self.wreg);
                ordinary
            }
            _ if instruction & 0x3f00 == 0x3c00 => {
                let literal = instruction as u8;
                let result = literal.wrapping_sub(self.wreg);
                self.set_sub_flags(literal, self.wreg, 0, result);
                self.wreg = result;
                ordinary
            }
            _ if instruction & 0x3f00 == 0x3e00 => {
                let literal = instruction as u8;
                let result = self.wreg.wrapping_add(literal);
                self.set_add_flags(self.wreg, literal, 0, result);
                self.wreg = result;
                ordinary
            }
            _ if instruction & 0x3f80 == 0x0180 => {
                self.write_core(file, 0, bus, now)?;
                self.set_zero(0);
                ordinary
            }
            _ if instruction & 0x3f80 == 0x0080 => {
                self.write_core(file, self.wreg, bus, now)?;
                ordinary
            }
            _ if is_file_operation(instruction) => {
                self.execute_file(instruction, file, destination_file, bus, now)?
            }
            _ => {
                return Err(CpuFault::new(
                    CpuFaultKind::IllegalInstruction,
                    u64::from(instruction_pc),
                    format!("unsupported or reserved 14-bit opcode {instruction:#06x}"),
                ));
            }
        };
        Ok(StepOutcome::advanced(elapsed))
    }

    fn execute_file(
        &mut self,
        instruction: u16,
        file: u8,
        destination_file: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<SimDuration, CpuFault> {
        let opcode = instruction & 0x3f00;
        let source = self.read_core(file, bus, now)?;
        let carry = u8::from(self.status & STATUS_C != 0);
        let mut skip = false;
        let (result, update_zero, write_result) = match opcode {
            0x0200 => {
                let result = source.wrapping_sub(self.wreg);
                self.set_sub_flags(source, self.wreg, 0, result);
                (result, false, true)
            }
            0x0300 => (source.wrapping_sub(1), true, true),
            0x0400 => (source | self.wreg, true, true),
            0x0500 => (source & self.wreg, true, true),
            0x0600 => (source ^ self.wreg, true, true),
            0x0700 => {
                let result = source.wrapping_add(self.wreg);
                self.set_add_flags(source, self.wreg, 0, result);
                (result, false, true)
            }
            0x0800 => (source, true, true),
            0x0900 => (!source, true, true),
            0x0a00 => (source.wrapping_add(1), true, true),
            0x0b00 => {
                let result = source.wrapping_sub(1);
                skip = result == 0;
                (result, false, true)
            }
            0x0c00 => {
                let result = (carry << 7) | (source >> 1);
                self.status = (self.status & !STATUS_C) | (source & 1);
                (result, false, true)
            }
            0x0d00 => {
                let result = (source << 1) | carry;
                self.status = (self.status & !STATUS_C) | (source >> 7);
                (result, false, true)
            }
            0x0e00 => (source.rotate_left(4), false, true),
            0x0f00 => {
                let result = source.wrapping_add(1);
                skip = result == 0;
                (result, false, true)
            }
            0x3500 => {
                let result = source << 1;
                self.status = (self.status & !STATUS_C) | (source >> 7);
                (result, true, true)
            }
            0x3600 => {
                let result = source >> 1;
                self.status = (self.status & !STATUS_C) | (source & 1);
                (result, true, true)
            }
            0x3700 => {
                let result = (source >> 1) | (source & 0x80);
                self.status = (self.status & !STATUS_C) | (source & 1);
                (result, true, true)
            }
            0x3b00 => {
                let borrow = u8::from(carry == 0);
                let result = source.wrapping_sub(self.wreg).wrapping_sub(borrow);
                self.set_sub_flags(source, self.wreg, borrow, result);
                (result, false, true)
            }
            0x3d00 => {
                let result = source.wrapping_add(self.wreg).wrapping_add(carry);
                self.set_add_flags(source, self.wreg, carry, result);
                (result, false, true)
            }
            _ => {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    format!("reserved file-register opcode {instruction:#06x}"),
                ));
            }
        };
        if update_zero {
            self.set_zero(result);
        }
        if write_result {
            if destination_file {
                self.write_core(file, result, bus, now)?;
            } else {
                self.wreg = result;
            }
        }
        if skip {
            self.pc = self.pc.wrapping_add(1) & 0x3fff;
            Ok(SimDuration::from_ticks(2))
        } else {
            Ok(SimDuration::from_ticks(1))
        }
    }
}

fn sign_extend(value: u16, bits: u32) -> i16 {
    let shift = 16 - bits;
    ((value << shift) as i16) >> shift
}

fn is_file_operation(instruction: u16) -> bool {
    matches!(
        instruction & 0x3f00,
        0x0200
            | 0x0300
            | 0x0400
            | 0x0500
            | 0x0600
            | 0x0700
            | 0x0800
            | 0x0900
            | 0x0a00
            | 0x0b00
            | 0x0c00
            | 0x0d00
            | 0x0e00
            | 0x0f00
            | 0x3500
            | 0x3600
            | 0x3700
            | 0x3b00
            | 0x3d00
    )
}
