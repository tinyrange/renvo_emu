use super::*;

impl AvrCpu {
    fn set_logic_flags(&mut self, value: u8) {
        self.sreg &= !(SREG_Z | SREG_N | SREG_V | SREG_S);
        if value == 0 {
            self.sreg |= SREG_Z;
        }
        if value & 0x80 != 0 {
            self.sreg |= SREG_N | SREG_S;
        }
    }

    fn set_add_flags(&mut self, left: u8, right: u8, value: u8, preserve_zero: bool) {
        let old_zero = self.sreg & SREG_Z != 0;
        self.sreg &= !(SREG_H | SREG_V | SREG_N | SREG_Z | SREG_C | SREG_S);
        if ((left & right) | (right & !value) | (!value & left)) & 0x08 != 0 {
            self.sreg |= SREG_H;
        }
        if (!(left ^ right) & (left ^ value)) & 0x80 != 0 {
            self.sreg |= SREG_V;
        }
        if value & 0x80 != 0 {
            self.sreg |= SREG_N;
        }
        if value == 0 && (!preserve_zero || old_zero) {
            self.sreg |= SREG_Z;
        }
        if ((left & right) | (right & !value) | (!value & left)) & 0x80 != 0 {
            self.sreg |= SREG_C;
        }
        if (self.sreg & SREG_N != 0) ^ (self.sreg & SREG_V != 0) {
            self.sreg |= SREG_S;
        }
    }

    fn set_sub_flags(&mut self, left: u8, right: u8, value: u8, preserve_zero: bool) {
        let old_zero = self.sreg & SREG_Z != 0;
        self.sreg &= !(SREG_H | SREG_V | SREG_N | SREG_Z | SREG_C | SREG_S);
        let borrow = (!left & right) | (right & value) | (value & !left);
        if borrow & 0x08 != 0 {
            self.sreg |= SREG_H;
        }
        if ((left & !right & !value) | (!left & right & value)) & 0x80 != 0 {
            self.sreg |= SREG_V;
        }
        if value & 0x80 != 0 {
            self.sreg |= SREG_N;
        }
        if value == 0 && (!preserve_zero || old_zero) {
            self.sreg |= SREG_Z;
        }
        if borrow & 0x80 != 0 {
            self.sreg |= SREG_C;
        }
        if (self.sreg & SREG_N != 0) ^ (self.sreg & SREG_V != 0) {
            self.sreg |= SREG_S;
        }
    }

    fn pair(&self, low: usize) -> u16 {
        u16::from(self.registers[low]) | (u16::from(self.registers[low + 1]) << 8)
    }

    fn set_pair(&mut self, low: usize, value: u16) {
        self.registers[low] = value as u8;
        self.registers[low + 1] = (value >> 8) as u8;
    }

    fn next_is_wide(&self) -> Result<bool, CpuFault> {
        let next = self.fetch(self.pc)?;
        Ok(next & 0xfe0c == 0x940c || next & 0xfe0f == 0x9000 || next & 0xfe0f == 0x9200)
    }

    fn skip_next(&mut self) -> Result<(), CpuFault> {
        self.pc = self
            .pc
            .wrapping_add(if self.next_is_wide()? { 2 } else { 1 });
        Ok(())
    }

    fn relative(offset: u16, bits: u8) -> i16 {
        let shift = 16 - bits;
        ((offset << shift) as i16) >> shift
    }

    pub(super) fn execute(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let d = usize::from((instruction >> 4) & 0x1f);
        let r = usize::from((instruction & 0xf) | ((instruction >> 5) & 0x10));
        match instruction {
            0x0000 => return Ok(StepReason::Advanced),
            0x95c8 => {
                self.registers[0] = self.program_read_byte(self.pair(30))?;
                return Ok(StepReason::Advanced);
            }
            0x9508 => {
                self.pc = self.pop_pc(bus, now)?;
                return Ok(StepReason::Advanced);
            }
            0x9518 => {
                self.pc = self.pop_pc(bus, now)?;
                self.sreg |= SREG_I;
                return Ok(StepReason::Advanced);
            }
            0x9588 => {
                if self.sleep_enabled {
                    self.waiting = true;
                    return Ok(StepReason::WaitForInterrupt);
                }
                return Ok(StepReason::Advanced);
            }
            0x9598 => {
                self.halted = true;
                return Ok(StepReason::Halted);
            }
            0x95a8 => return Ok(StepReason::Advanced),
            0x9478 => {
                self.sreg |= SREG_I;
                return Ok(StepReason::Advanced);
            }
            0x94f8 => {
                self.sreg &= !SREG_I;
                return Ok(StepReason::Advanced);
            }
            _ => {}
        }

        if instruction & 0xff00 == 0x0100 {
            let destination = usize::from((instruction >> 4) & 0xf) * 2;
            let source = usize::from(instruction & 0xf) * 2;
            self.registers[destination] = self.registers[source];
            self.registers[destination + 1] = self.registers[source + 1];
        } else if instruction & 0xfc00 == 0x0c00 {
            let left = self.registers[d];
            let right = self.registers[r];
            let value = left.wrapping_add(right);
            self.registers[d] = value;
            self.set_add_flags(left, right, value, false);
        } else if instruction & 0xfc00 == 0x1c00 {
            let left = self.registers[d];
            let right = self.registers[r].wrapping_add(u8::from(self.sreg & SREG_C != 0));
            let value = left.wrapping_add(right);
            self.registers[d] = value;
            self.set_add_flags(left, right, value, true);
        } else if instruction & 0xfc00 == 0x1800 {
            let left = self.registers[d];
            let right = self.registers[r];
            let value = left.wrapping_sub(right);
            self.registers[d] = value;
            self.set_sub_flags(left, right, value, false);
        } else if instruction & 0xfc00 == 0x0800 {
            let left = self.registers[d];
            let right = self.registers[r].wrapping_add(u8::from(self.sreg & SREG_C != 0));
            let value = left.wrapping_sub(right);
            self.registers[d] = value;
            self.set_sub_flags(left, right, value, true);
        } else if instruction & 0xfc00 == 0x1400 {
            let left = self.registers[d];
            let right = self.registers[r];
            self.set_sub_flags(left, right, left.wrapping_sub(right), false);
        } else if instruction & 0xfc00 == 0x0400 {
            let left = self.registers[d];
            let right = self.registers[r].wrapping_add(u8::from(self.sreg & SREG_C != 0));
            self.set_sub_flags(left, right, left.wrapping_sub(right), true);
        } else if instruction & 0xfc00 == 0x1000 {
            if self.registers[d] == self.registers[r] {
                self.skip_next()?;
            }
        } else if instruction & 0xfc00 == 0x2000 {
            self.registers[d] &= self.registers[r];
            self.set_logic_flags(self.registers[d]);
        } else if instruction & 0xfc00 == 0x2400 {
            self.registers[d] ^= self.registers[r];
            self.set_logic_flags(self.registers[d]);
        } else if instruction & 0xfc00 == 0x2800 {
            self.registers[d] |= self.registers[r];
            self.set_logic_flags(self.registers[d]);
        } else if instruction & 0xfc00 == 0x2c00 {
            self.registers[d] = self.registers[r];
        } else if instruction & 0xf000 == 0xe000 {
            let destination = 16 + usize::from((instruction >> 4) & 0xf);
            self.registers[destination] =
                ((instruction >> 4) & 0xf0) as u8 | (instruction & 0xf) as u8;
        } else if instruction & 0xf000 == 0x3000 {
            let destination = 16 + usize::from((instruction >> 4) & 0xf);
            let immediate = ((instruction >> 4) & 0xf0) as u8 | (instruction & 0xf) as u8;
            let left = self.registers[destination];
            self.set_sub_flags(left, immediate, left.wrapping_sub(immediate), false);
        } else if instruction & 0xf000 == 0x4000 || instruction & 0xf000 == 0x5000 {
            let destination = 16 + usize::from((instruction >> 4) & 0xf);
            let mut immediate = ((instruction >> 4) & 0xf0) as u8 | (instruction & 0xf) as u8;
            let preserve = instruction & 0xf000 == 0x4000;
            if preserve {
                immediate = immediate.wrapping_add(u8::from(self.sreg & SREG_C != 0));
            }
            let left = self.registers[destination];
            let value = left.wrapping_sub(immediate);
            self.registers[destination] = value;
            self.set_sub_flags(left, immediate, value, preserve);
        } else if instruction & 0xf000 == 0x6000 || instruction & 0xf000 == 0x7000 {
            let destination = 16 + usize::from((instruction >> 4) & 0xf);
            let immediate = ((instruction >> 4) & 0xf0) as u8 | (instruction & 0xf) as u8;
            if instruction & 0xf000 == 0x6000 {
                self.registers[destination] |= immediate;
            } else {
                self.registers[destination] &= immediate;
            }
            self.set_logic_flags(self.registers[destination]);
        } else if instruction & 0xf000 == 0xc000 {
            let offset = Self::relative(instruction & 0x0fff, 12);
            self.pc = self.pc.wrapping_add_signed(offset);
        } else if instruction & 0xf000 == 0xd000 {
            let return_pc = self.pc;
            let offset = Self::relative(instruction & 0x0fff, 12);
            self.push_pc(bus, return_pc, now)?;
            self.pc = self.pc.wrapping_add_signed(offset);
        } else if instruction & 0xfc00 == 0xf000 || instruction & 0xfc00 == 0xf400 {
            let offset = Self::relative((instruction >> 3) & 0x7f, 7);
            let flag = self.sreg & (1 << (instruction & 7)) != 0;
            let branch_if_set = instruction & 0x0400 == 0;
            if flag == branch_if_set {
                self.pc = self.pc.wrapping_add_signed(offset);
            }
        } else if instruction & 0xf800 == 0xb000 {
            let address = 0x20 + ((instruction & 0xf) | ((instruction >> 5) & 0x30));
            self.registers[d] = self.data_read(bus, address, now)?;
        } else if instruction & 0xf800 == 0xb800 {
            let address = 0x20 + ((instruction & 0xf) | ((instruction >> 5) & 0x30));
            self.data_write(bus, address, self.registers[d], now)?;
        } else if instruction & 0xff00 == 0x9a00 || instruction & 0xff00 == 0x9800 {
            let address = 0x20 + ((instruction >> 3) & 0x1f);
            let bit = 1_u8 << (instruction & 7);
            let mut value = self.data_read(bus, address, now)?;
            if instruction & 0xff00 == 0x9a00 {
                value |= bit;
            } else {
                value &= !bit;
            }
            self.data_write(bus, address, value, now)?;
        } else if instruction & 0xff00 == 0x9900 || instruction & 0xff00 == 0x9b00 {
            let address = 0x20 + ((instruction >> 3) & 0x1f);
            let set = self.data_read(bus, address, now)? & (1 << (instruction & 7)) != 0;
            if set == (instruction & 0xff00 == 0x9b00) {
                self.skip_next()?;
            }
        } else if instruction & 0xfe0e == 0x9004 {
            let address = self.pair(30);
            self.registers[d] = self.program_read_byte(address)?;
            if instruction & 1 != 0 {
                self.set_pair(30, address.wrapping_add(1));
            }
        } else if instruction & 0xfe0f == 0x9000 {
            let address = self.fetch(self.pc)?;
            self.pc = self.pc.wrapping_add(1);
            self.registers[d] = self.data_read(bus, address, now)?;
        } else if instruction & 0xfe0f == 0x9200 {
            let address = self.fetch(self.pc)?;
            self.pc = self.pc.wrapping_add(1);
            self.data_write(bus, address, self.registers[d], now)?;
        } else if instruction & 0xfe0f == 0x900f {
            self.registers[d] = self.pop(bus, now)?;
        } else if instruction & 0xfe0f == 0x920f {
            self.push(bus, self.registers[d], now)?;
        } else if instruction & 0xfe0f == 0x900c
            || instruction & 0xfe0f == 0x900d
            || instruction & 0xfe0f == 0x900e
        {
            let mut address = self.pair(26);
            if instruction & 0xf == 0xe {
                address = address.wrapping_sub(1);
                self.set_pair(26, address);
            }
            self.registers[d] = self.data_read(bus, address, now)?;
            if instruction & 0xf == 0xd {
                self.set_pair(26, address.wrapping_add(1));
            }
        } else if instruction & 0xfe0f == 0x920c
            || instruction & 0xfe0f == 0x920d
            || instruction & 0xfe0f == 0x920e
        {
            let mut address = self.pair(26);
            if instruction & 0xf == 0xe {
                address = address.wrapping_sub(1);
                self.set_pair(26, address);
            }
            self.data_write(bus, address, self.registers[d], now)?;
            if instruction & 0xf == 0xd {
                self.set_pair(26, address.wrapping_add(1));
            }
        } else if instruction & 0xfe0f == 0x9001
            || instruction & 0xfe0f == 0x9002
            || instruction & 0xfe0f == 0x9009
            || instruction & 0xfe0f == 0x900a
        {
            let low = if instruction & 8 == 0 { 30 } else { 28 };
            let mut address = self.pair(low);
            if instruction & 3 == 2 {
                address = address.wrapping_sub(1);
                self.set_pair(low, address);
            }
            self.registers[d] = self.data_read(bus, address, now)?;
            if instruction & 3 == 1 {
                self.set_pair(low, address.wrapping_add(1));
            }
        } else if instruction & 0xfe0f == 0x9201
            || instruction & 0xfe0f == 0x9202
            || instruction & 0xfe0f == 0x9209
            || instruction & 0xfe0f == 0x920a
        {
            let low = if instruction & 8 == 0 { 30 } else { 28 };
            let mut address = self.pair(low);
            if instruction & 3 == 2 {
                address = address.wrapping_sub(1);
                self.set_pair(low, address);
            }
            self.data_write(bus, address, self.registers[d], now)?;
            if instruction & 3 == 1 {
                self.set_pair(low, address.wrapping_add(1));
            }
        } else if instruction & 0xd000 == 0x8000 {
            let low = if instruction & 8 == 0 { 30 } else { 28 };
            let displacement =
                ((instruction >> 8) & 0x20) | ((instruction >> 7) & 0x18) | (instruction & 7);
            let address = self.pair(low).wrapping_add(displacement);
            if instruction & 0x0200 == 0 {
                self.registers[d] = self.data_read(bus, address, now)?;
            } else {
                self.data_write(bus, address, self.registers[d], now)?;
            }
        } else if instruction & 0xff00 == 0x9600 || instruction & 0xff00 == 0x9700 {
            let low = 24 + usize::from((instruction >> 3) & 6);
            let immediate = ((instruction >> 2) & 0x30) | (instruction & 0xf);
            let left = self.pair(low);
            let add = instruction & 0xff00 == 0x9600;
            let value = if add {
                left.wrapping_add(immediate)
            } else {
                left.wrapping_sub(immediate)
            };
            self.set_pair(low, value);
            self.sreg &= !(SREG_Z | SREG_N | SREG_V | SREG_S | SREG_C);
            if value == 0 {
                self.sreg |= SREG_Z;
            }
            if value & 0x8000 != 0 {
                self.sreg |= SREG_N;
            }
            let old_negative = left & 0x8000 != 0;
            let new_negative = value & 0x8000 != 0;
            let overflow = if add {
                !old_negative && new_negative
            } else {
                old_negative && !new_negative
            };
            let carry = if add {
                old_negative && !new_negative
            } else {
                !old_negative && new_negative
            };
            if overflow {
                self.sreg |= SREG_V;
            }
            if carry {
                self.sreg |= SREG_C;
            }
            if (self.sreg & SREG_N != 0) ^ (self.sreg & SREG_V != 0) {
                self.sreg |= SREG_S;
            }
        } else if instruction & 0xfe0f == 0x9403 {
            self.registers[d] = self.registers[d].wrapping_add(1);
            self.set_logic_flags(self.registers[d]);
        } else if instruction & 0xfe0f == 0x940a {
            self.registers[d] = self.registers[d].wrapping_sub(1);
            self.set_logic_flags(self.registers[d]);
        } else if instruction & 0xfe0f == 0x9400 {
            self.registers[d] = !self.registers[d];
            self.set_logic_flags(self.registers[d]);
            self.sreg |= SREG_C;
        } else if instruction & 0xfe0f == 0x9406 {
            let old = self.registers[d];
            self.registers[d] >>= 1;
            self.set_logic_flags(self.registers[d]);
            if old & 1 != 0 {
                self.sreg |= SREG_C;
            } else {
                self.sreg &= !SREG_C;
            }
        } else if instruction & 0xfe0f == 0x9407 {
            let old = self.registers[d];
            let carry_in = u8::from(self.sreg & SREG_C != 0) << 7;
            self.registers[d] = (old >> 1) | carry_in;
            self.set_logic_flags(self.registers[d]);
            if old & 1 != 0 {
                self.sreg |= SREG_C;
            } else {
                self.sreg &= !SREG_C;
            }
        } else if instruction & 0xfe0f == 0x9405 {
            let old = self.registers[d];
            self.registers[d] = ((old as i8) >> 1) as u8;
            self.set_logic_flags(self.registers[d]);
            if old & 1 != 0 {
                self.sreg |= SREG_C;
            } else {
                self.sreg &= !SREG_C;
            }
        } else if instruction & 0xfe0f == 0x9402 {
            self.registers[d] = self.registers[d].rotate_left(4);
        } else if instruction & 0xfe0c == 0x940c {
            let second = self.fetch(self.pc)?;
            self.pc = self.pc.wrapping_add(1);
            let target = second;
            if instruction & 2 != 0 {
                self.push_pc(bus, self.pc, now)?;
            }
            self.pc = target;
        } else if instruction & 0xff8f == 0x9408 {
            let flag = 1 << ((instruction >> 4) & 7);
            if instruction & 0x0080 == 0 {
                self.sreg |= flag as u8;
            } else {
                self.sreg &= !(flag as u8);
            }
        } else if instruction & 0xfc00 == 0x9c00 {
            let result = u16::from(self.registers[d]) * u16::from(self.registers[r]);
            self.set_pair(0, result);
            self.sreg &= !(SREG_Z | SREG_C);
            if result == 0 {
                self.sreg |= SREG_Z;
            }
            if result & 0x8000 != 0 {
                self.sreg |= SREG_C;
            }
        } else if instruction & 0xfe08 == 0xfc00 || instruction & 0xfe08 == 0xfe00 {
            let set = self.registers[d] & (1 << (instruction & 7)) != 0;
            if set == (instruction & 0x0200 != 0) {
                self.skip_next()?;
            }
        } else if instruction & 0xfe08 == 0xfa00 {
            let bit = 1 << (instruction & 7);
            if self.sreg & SREG_T != 0 {
                self.registers[d] |= bit;
            } else {
                self.registers[d] &= !bit;
            }
        } else if instruction & 0xfe08 == 0xf800 {
            if self.registers[d] & (1 << (instruction & 7)) != 0 {
                self.sreg |= SREG_T;
            } else {
                self.sreg &= !SREG_T;
            }
        } else {
            return Err(self.fault(
                CpuFaultKind::IllegalInstruction,
                format!("unsupported AVR instruction {instruction:#06x}"),
            ));
        }
        Ok(StepReason::Advanced)
    }
}
