use super::{
    Bus, CpuFault, CpuFaultKind, Mcs51Cpu, PSW_AC, PSW_C, PSW_OV, SimDuration, SimTime, StepOutcome,
};

impl Mcs51Cpu {
    fn relative_branch(&mut self, offset: u8) {
        let signed = i8::from_ne_bytes([offset]);
        self.pc = self.pc.wrapping_add_signed(i16::from(signed));
    }

    fn set_flag(&mut self, mask: u8, value: bool) {
        self.psw = (self.psw & !mask) | if value { mask } else { 0 };
    }

    fn add(&mut self, right: u8, with_carry: bool) {
        let left = self.a;
        let carry = u16::from(with_carry && self.carry());
        let wide = u16::from(left) + u16::from(right) + carry;
        let value = wide.to_le_bytes()[0];
        self.a = value;
        self.set_flag(PSW_C, wide > 0xff);
        self.set_flag(
            PSW_AC,
            u16::from(left & 0x0f) + u16::from(right & 0x0f) + carry > 0x0f,
        );
        self.set_flag(PSW_OV, (!(left ^ right) & (left ^ value) & 0x80) != 0);
    }

    fn subb(&mut self, right: u8) {
        let left = self.a;
        let carry_byte = u8::from(self.carry());
        let carry = u16::from(carry_byte);
        let subtrahend = u16::from(right) + carry;
        let value = left.wrapping_sub(right).wrapping_sub(carry_byte);
        self.a = value;
        self.set_flag(PSW_C, u16::from(left) < subtrahend);
        self.set_flag(
            PSW_AC,
            u16::from(left & 0x0f) < u16::from(right & 0x0f) + carry,
        );
        self.set_flag(PSW_OV, ((left ^ right) & (left ^ value) & 0x80) != 0);
    }

    fn operand(&mut self, opcode: u8, bus: &mut dyn Bus, now: SimTime) -> Result<u8, CpuFault> {
        match opcode & 0x0f {
            0x04 => self.fetch8(),
            0x05 => {
                let direct = self.fetch8()?;
                self.direct_read(bus, direct, now)
            }
            0x06 | 0x07 => Ok(self.indirect(opcode & 1)),
            0x08..=0x0f => Ok(self.reg(opcode & 7)),
            _ => Err(self.fault(
                CpuFaultKind::Architecture,
                format!("invalid operand form for opcode {opcode:#04x}"),
            )),
        }
    }

    fn inc_dec_operand(
        &mut self,
        opcode: u8,
        increment: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        let adjust = |value: u8| {
            if increment {
                value.wrapping_add(1)
            } else {
                value.wrapping_sub(1)
            }
        };
        match opcode & 0x0f {
            0x04 => self.a = adjust(self.a),
            0x05 => {
                let direct = self.fetch8()?;
                let value = adjust(self.direct_read(bus, direct, now)?);
                self.direct_write(bus, direct, value, now)?;
            }
            0x06 | 0x07 => {
                let index = opcode & 1;
                self.set_indirect(index, adjust(self.indirect(index)));
            }
            0x08..=0x0f => {
                let index = opcode & 7;
                self.set_reg(index, adjust(self.reg(index)));
            }
            _ => unreachable!("opcode group controls operand form"),
        }
        Ok(())
    }

    fn logic_accumulator(
        &mut self,
        opcode: u8,
        bus: &mut dyn Bus,
        now: SimTime,
        operation: fn(u8, u8) -> u8,
    ) -> Result<(), CpuFault> {
        let right = self.operand(opcode, bus, now)?;
        self.a = operation(self.a, right);
        Ok(())
    }

    fn compare_jump(&mut self, left: u8, right: u8, relative: u8) {
        self.set_carry(left < right);
        if left != right {
            self.relative_branch(relative);
        }
    }

    fn duration(opcode: u8) -> SimDuration {
        let ticks = match opcode {
            0x02
            | 0x12
            | 0x22
            | 0x32
            | 0x73
            | 0x83
            | 0x93
            | 0xa3
            | 0xb4..=0xbf
            | 0xd5
            | 0xd8..=0xdf
            | 0xe0
            | 0xe2
            | 0xe3
            | 0xf0
            | 0xf2
            | 0xf3 => 2,
            0x84 | 0xa4 => 4,
            _ if opcode & 0x1f == 0x01 || opcode & 0x1f == 0x11 => 2,
            _ => 1,
        };
        SimDuration::from_ticks(ticks)
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn execute(
        &mut self,
        opcode: u8,
        instruction_pc: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepOutcome, CpuFault> {
        if opcode & 0x1f == 0x01 {
            let low = self.fetch8()?;
            self.pc = (self.pc & 0xf800) | (u16::from(opcode & 0xe0) << 3) | u16::from(low);
            return Ok(StepOutcome::advanced(Self::duration(opcode)));
        }
        if opcode & 0x1f == 0x11 {
            let low = self.fetch8()?;
            let target = (self.pc & 0xf800) | (u16::from(opcode & 0xe0) << 3) | u16::from(low);
            self.push_pc();
            self.pc = target;
            return Ok(StepOutcome::advanced(Self::duration(opcode)));
        }

        match opcode {
            0x00 => {}
            0x02 => self.pc = self.fetch16()?,
            0x03 => self.a = self.a.rotate_right(1),
            0x04..=0x0f => self.inc_dec_operand(opcode, true, bus, now)?,
            0x10 => {
                let bit = self.fetch8()?;
                let relative = self.fetch8()?;
                if self.bit_read(bus, bit, now)? {
                    self.bit_write(bus, bit, false, now)?;
                    self.relative_branch(relative);
                }
            }
            0x12 => {
                let target = self.fetch16()?;
                self.push_pc();
                self.pc = target;
            }
            0x13 => {
                let carry = self.carry();
                self.set_carry(self.a & 1 != 0);
                self.a = (self.a >> 1) | (u8::from(carry) << 7);
            }
            0x14..=0x1f => self.inc_dec_operand(opcode, false, bus, now)?,
            0x20 | 0x30 => {
                let bit = self.fetch8()?;
                let relative = self.fetch8()?;
                if self.bit_read(bus, bit, now)? == (opcode == 0x20) {
                    self.relative_branch(relative);
                }
            }
            0x22 => self.pop_pc(),
            0x23 => self.a = self.a.rotate_left(1),
            0x24..=0x2f => {
                let right = self.operand(opcode, bus, now)?;
                self.add(right, false);
            }
            0x32 => {
                self.pop_pc();
                self.active_priority = self.priority_stack.pop().unwrap_or(None);
                self.sfr_page = self.sfr_page_stack.pop().unwrap_or(0);
            }
            0x33 => {
                let carry = self.carry();
                self.set_carry(self.a & 0x80 != 0);
                self.a = (self.a << 1) | u8::from(carry);
            }
            0x34..=0x3f => {
                let right = self.operand(opcode, bus, now)?;
                self.add(right, true);
            }
            0x40 | 0x50 | 0x60 | 0x70 => {
                let relative = self.fetch8()?;
                let take = match opcode {
                    0x40 => self.carry(),
                    0x50 => !self.carry(),
                    0x60 => self.a == 0,
                    0x70 => self.a != 0,
                    _ => unreachable!(),
                };
                if take {
                    self.relative_branch(relative);
                }
            }
            0x42 | 0x43 | 0x52 | 0x53 | 0x62 | 0x63 => {
                let direct = self.fetch8()?;
                let immediate = if opcode & 1 == 1 {
                    self.fetch8()?
                } else {
                    self.a
                };
                let old = self.direct_read(bus, direct, now)?;
                let value = match opcode & 0xf0 {
                    0x40 => old | immediate,
                    0x50 => old & immediate,
                    0x60 => old ^ immediate,
                    _ => unreachable!(),
                };
                self.direct_write(bus, direct, value, now)?;
            }
            0x44..=0x4f => self.logic_accumulator(opcode, bus, now, |a, b| a | b)?,
            0x54..=0x5f => self.logic_accumulator(opcode, bus, now, |a, b| a & b)?,
            0x64..=0x6f => self.logic_accumulator(opcode, bus, now, |a, b| a ^ b)?,
            0x72 | 0x82 | 0xa0 | 0xb0 => {
                let bit = self.fetch8()?;
                let mut value = self.bit_read(bus, bit, now)?;
                if matches!(opcode, 0xa0 | 0xb0) {
                    value = !value;
                }
                self.set_carry(if matches!(opcode, 0x72 | 0xa0) {
                    self.carry() | value
                } else {
                    self.carry() & value
                });
            }
            0x73 => self.pc = self.dptr.wrapping_add(u16::from(self.a)),
            0x74 => self.a = self.fetch8()?,
            0x75 => {
                let direct = self.fetch8()?;
                let immediate = self.fetch8()?;
                self.direct_write(bus, direct, immediate, now)?;
            }
            0x76 | 0x77 => {
                let immediate = self.fetch8()?;
                self.set_indirect(opcode & 1, immediate);
            }
            0x78..=0x7f => {
                let immediate = self.fetch8()?;
                self.set_reg(opcode & 7, immediate);
            }
            0x80 => {
                let relative = self.fetch8()?;
                self.relative_branch(relative);
            }
            0x83 => {
                self.a = self.code_read(self.pc.wrapping_add(u16::from(self.a)))?;
            }
            0x84 => {
                self.set_carry(false);
                match self.a.checked_div(self.b) {
                    Some(quotient) => {
                        self.b = self.a % self.b;
                        self.a = quotient;
                        self.set_flag(PSW_OV, false);
                    }
                    None => self.set_flag(PSW_OV, true),
                }
            }
            0x85 => {
                let source = self.fetch8()?;
                let destination = self.fetch8()?;
                let value = self.direct_read(bus, source, now)?;
                self.direct_write(bus, destination, value, now)?;
            }
            0x86 | 0x87 => {
                let direct = self.fetch8()?;
                self.direct_write(bus, direct, self.indirect(opcode & 1), now)?;
            }
            0x88..=0x8f => {
                let direct = self.fetch8()?;
                self.direct_write(bus, direct, self.reg(opcode & 7), now)?;
            }
            0x90 => self.dptr = self.fetch16()?,
            0x92 => {
                let bit = self.fetch8()?;
                self.bit_write(bus, bit, self.carry(), now)?;
            }
            0x93 => {
                self.a = self.code_read(self.dptr.wrapping_add(u16::from(self.a)))?;
            }
            0x94..=0x9f => {
                let right = self.operand(opcode, bus, now)?;
                self.subb(right);
            }
            0xa2 => {
                let bit = self.fetch8()?;
                let value = self.bit_read(bus, bit, now)?;
                self.set_carry(value);
            }
            0xa3 => self.dptr = self.dptr.wrapping_add(1),
            0xa4 => {
                let product = u16::from(self.a) * u16::from(self.b);
                let [low, high] = product.to_le_bytes();
                self.a = low;
                self.b = high;
                self.set_carry(false);
                self.set_flag(PSW_OV, product > 0xff);
            }
            0xa5 => {
                return Err(CpuFault::new(
                    CpuFaultKind::IllegalInstruction,
                    u64::from(instruction_pc),
                    "reserved MCS-51 opcode 0xa5",
                ));
            }
            0xa6 | 0xa7 => {
                let direct = self.fetch8()?;
                let value = self.direct_read(bus, direct, now)?;
                self.set_indirect(opcode & 1, value);
            }
            0xa8..=0xaf => {
                let direct = self.fetch8()?;
                let value = self.direct_read(bus, direct, now)?;
                self.set_reg(opcode & 7, value);
            }
            0xb2 => {
                let bit = self.fetch8()?;
                let value = !self.bit_read(bus, bit, now)?;
                self.bit_write(bus, bit, value, now)?;
            }
            0xb3 => self.set_carry(!self.carry()),
            0xb4 => {
                let immediate = self.fetch8()?;
                let relative = self.fetch8()?;
                self.compare_jump(self.a, immediate, relative);
            }
            0xb5 => {
                let direct = self.fetch8()?;
                let relative = self.fetch8()?;
                let right = self.direct_read(bus, direct, now)?;
                self.compare_jump(self.a, right, relative);
            }
            0xb6 | 0xb7 => {
                let immediate = self.fetch8()?;
                let relative = self.fetch8()?;
                self.compare_jump(self.indirect(opcode & 1), immediate, relative);
            }
            0xb8..=0xbf => {
                let immediate = self.fetch8()?;
                let relative = self.fetch8()?;
                self.compare_jump(self.reg(opcode & 7), immediate, relative);
            }
            0xc0 => {
                let direct = self.fetch8()?;
                let value = self.direct_read(bus, direct, now)?;
                self.push_byte(value);
            }
            0xc2 | 0xd2 => {
                let bit = self.fetch8()?;
                self.bit_write(bus, bit, opcode == 0xd2, now)?;
            }
            0xc3 | 0xd3 => self.set_carry(opcode == 0xd3),
            0xc4 => self.a = self.a.rotate_left(4),
            0xc5 => {
                let direct = self.fetch8()?;
                let old = self.direct_read(bus, direct, now)?;
                self.direct_write(bus, direct, self.a, now)?;
                self.a = old;
            }
            0xc6 | 0xc7 => {
                let index = opcode & 1;
                let old = self.indirect(index);
                self.set_indirect(index, self.a);
                self.a = old;
            }
            0xc8..=0xcf => {
                let index = opcode & 7;
                let old = self.reg(index);
                self.set_reg(index, self.a);
                self.a = old;
            }
            0xd0 => {
                let direct = self.fetch8()?;
                let value = self.pop_byte();
                self.direct_write(bus, direct, value, now)?;
            }
            0xd4 => {
                let old_carry = self.carry();
                let old_ac = self.psw & PSW_AC != 0;
                let mut value = u16::from(self.a);
                if self.a & 0x0f > 9 || old_ac {
                    value += 0x06;
                }
                if value > 0x99 || old_carry {
                    value += 0x60;
                }
                self.a = value.to_le_bytes()[0];
                self.set_carry(old_carry || value > 0xff);
            }
            0xd5 => {
                let direct = self.fetch8()?;
                let relative = self.fetch8()?;
                let value = self.direct_read(bus, direct, now)?.wrapping_sub(1);
                self.direct_write(bus, direct, value, now)?;
                if value != 0 {
                    self.relative_branch(relative);
                }
            }
            0xd6 | 0xd7 => {
                let index = opcode & 1;
                let value = self.indirect(index);
                let exchanged = (value & 0xf0) | (self.a & 0x0f);
                self.a = (self.a & 0xf0) | (value & 0x0f);
                self.set_indirect(index, exchanged);
            }
            0xd8..=0xdf => {
                let relative = self.fetch8()?;
                let index = opcode & 7;
                let value = self.reg(index).wrapping_sub(1);
                self.set_reg(index, value);
                if value != 0 {
                    self.relative_branch(relative);
                }
            }
            0xe0 => self.a = self.xdata_read(bus, self.dptr, now)?,
            0xe2 | 0xe3 => {
                let high = u16::from(self.sfr_read(bus, 0xa0, now)?);
                let address = (high << 8) | u16::from(self.reg(opcode & 1));
                self.a = self.xdata_read(bus, address, now)?;
            }
            0xe4 => self.a = 0,
            0xe5 => {
                let direct = self.fetch8()?;
                self.a = self.direct_read(bus, direct, now)?;
            }
            0xe6 | 0xe7 => self.a = self.indirect(opcode & 1),
            0xe8..=0xef => self.a = self.reg(opcode & 7),
            0xf0 => self.xdata_write(bus, self.dptr, self.a, now)?,
            0xf2 | 0xf3 => {
                let high = u16::from(self.sfr_read(bus, 0xa0, now)?);
                let address = (high << 8) | u16::from(self.reg(opcode & 1));
                self.xdata_write(bus, address, self.a, now)?;
            }
            0xf4 => self.a = !self.a,
            0xf5 => {
                let direct = self.fetch8()?;
                self.direct_write(bus, direct, self.a, now)?;
            }
            0xf6 | 0xf7 => self.set_indirect(opcode & 1, self.a),
            0xf8..=0xff => self.set_reg(opcode & 7, self.a),
            _ => {
                return Err(CpuFault::new(
                    CpuFaultKind::IllegalInstruction,
                    u64::from(instruction_pc),
                    format!("unimplemented MCS-51 opcode {opcode:#04x}"),
                ));
            }
        }
        Ok(StepOutcome::advanced(Self::duration(opcode)))
    }
}
