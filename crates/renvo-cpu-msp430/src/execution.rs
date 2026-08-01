use super::*;

impl Msp430Cpu {
    fn classic_index(base: u32, offset: u16) -> u32 {
        if base & 0x000f_0000 == 0 {
            u32::from((base as u16).wrapping_add(offset))
        } else {
            base.wrapping_add_signed(i32::from(offset as i16)) & ADDRESS_MASK
        }
    }

    fn execute_address_extension(
        &mut self,
        _extension: u16,
        source_extension: u16,
        destination_extension: u16,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        if instruction >> 12 >= 4 {
            return self.execute_extended_double(
                instruction,
                source_extension,
                destination_extension,
                true,
                bus,
                now,
            );
        }
        if instruction & 0xfc00 == 0x1000 {
            return self.execute_extended_single(
                instruction,
                source_extension,
                destination_extension,
                true,
                bus,
                now,
            );
        }
        Err(self.fault(
            CpuFaultKind::IllegalInstruction,
            format!("address extension before unsupported instruction {instruction:#06x}"),
        ))
    }

    fn extended_source(
        &mut self,
        register: usize,
        mode: u16,
        extension: u16,
        address_size: bool,
        byte: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<u32, CpuFault> {
        let data_mask = if address_size {
            ADDRESS_MASK
        } else if byte {
            0xff
        } else {
            0xffff
        };
        let constant = match (register, mode) {
            (3, 0) => Some(0),
            (3, 1) => Some(1),
            (3, 2) => Some(2),
            (3, 3) => Some(data_mask),
            (2, 2) => Some(4),
            (2, 3) => Some(8),
            _ => None,
        };
        if let Some(value) = constant {
            return Ok(value);
        }
        let read_memory =
            |cpu: &Self, bus: &mut dyn Bus, address: u32, now: SimTime| -> Result<u32, CpuFault> {
                if address_size {
                    cpu.read_address(bus, address, AccessKind::Read, now)
                } else {
                    cpu.read(bus, address, byte, AccessKind::Read, now)
                        .map(u32::from)
                }
            };
        match mode {
            0 => Ok(self.registers[register] & data_mask),
            1 => {
                let low = self.fetch(bus, now)?;
                let displacement = (u32::from(extension) << 16) | u32::from(low);
                let address = match register {
                    0 => self.registers[0].wrapping_add(displacement),
                    2 => displacement,
                    _ => self.registers[register].wrapping_add(displacement),
                } & ADDRESS_MASK;
                read_memory(self, bus, address, now)
            }
            2 => read_memory(self, bus, self.registers[register], now),
            3 if register == 0 => {
                let low = self.fetch(bus, now)?;
                Ok(((u32::from(extension) << 16) | u32::from(low)) & data_mask)
            }
            3 => {
                let address = self.registers[register];
                let value = read_memory(self, bus, address, now)?;
                let increment = if address_size {
                    4
                } else if byte && register != 1 {
                    1
                } else {
                    2
                };
                self.registers[register] = address.wrapping_add(increment) & ADDRESS_MASK;
                Ok(value)
            }
            _ => unreachable!(),
        }
    }

    fn extended_destination(
        &mut self,
        register: usize,
        indexed: bool,
        extension: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<OperandTarget, CpuFault> {
        if !indexed {
            return Ok(OperandTarget::Register(register));
        }
        let low = self.fetch(bus, now)?;
        let displacement = (u32::from(extension) << 16) | u32::from(low);
        let address = match register {
            0 => self.registers[0].wrapping_add(displacement),
            2 => displacement,
            _ => self.registers[register].wrapping_add(displacement),
        } & ADDRESS_MASK;
        Ok(OperandTarget::Memory(address))
    }

    fn extended_single_destination(
        &mut self,
        register: usize,
        mode: u16,
        extension: u16,
        address_size: bool,
        byte: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<OperandTarget, CpuFault> {
        match mode {
            0 => Ok(OperandTarget::Register(register)),
            1 => self.extended_destination(register, true, extension, bus, now),
            2 => Ok(OperandTarget::Memory(self.registers[register])),
            3 if register != 0 => {
                let address = self.registers[register];
                let increment = if address_size {
                    4
                } else if byte && register != 1 {
                    1
                } else {
                    2
                };
                self.registers[register] = address.wrapping_add(increment) & ADDRESS_MASK;
                Ok(OperandTarget::Memory(address))
            }
            _ => Err(self.fault(
                CpuFaultKind::IllegalInstruction,
                "immediate operand is not writable",
            )),
        }
    }

    fn extended_target_read(
        &self,
        target: OperandTarget,
        address_size: bool,
        byte: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<u32, CpuFault> {
        match target {
            OperandTarget::Register(register) => Ok(self.registers[register]
                & if address_size {
                    ADDRESS_MASK
                } else if byte {
                    0xff
                } else {
                    0xffff
                }),
            OperandTarget::Memory(address) if address_size => {
                self.read_address(bus, address, AccessKind::Read, now)
            }
            OperandTarget::Memory(address) => self
                .read(bus, address, byte, AccessKind::Read, now)
                .map(u32::from),
        }
    }

    fn extended_target_write(
        &mut self,
        target: OperandTarget,
        address_size: bool,
        byte: bool,
        value: u32,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        match target {
            OperandTarget::Register(register) => {
                self.set_register_value(register, value, !address_size && byte);
                Ok(())
            }
            OperandTarget::Memory(address) if address_size => {
                self.write_address(bus, address, value, now)
            }
            OperandTarget::Memory(address) => self.write(bus, address, byte, value as u16, now),
        }
    }

    fn set_extended_nz(&mut self, value: u32, bits: u32) {
        let mask = (1_u32 << bits) - 1;
        let sign = 1_u32 << (bits - 1);
        let mut status = self.status() & !(SR_N | SR_Z);
        if value & mask == 0 {
            status |= SR_Z;
        }
        if value & sign != 0 {
            status |= SR_N;
        }
        self.set_status(status);
    }

    fn set_extended_add_flags(&mut self, left: u32, right: u32, result: u64, bits: u32) {
        let modulus = 1_u64 << bits;
        let mask = modulus - 1;
        let sign = 1_u64 << (bits - 1);
        let value = result & mask;
        let mut status = self.status() & !(SR_C | SR_Z | SR_N | SR_V);
        if result >= modulus {
            status |= SR_C;
        }
        if value == 0 {
            status |= SR_Z;
        }
        if value & sign != 0 {
            status |= SR_N;
        }
        if (!(u64::from(left) ^ u64::from(right)) & (u64::from(left) ^ value) & sign) != 0 {
            status |= SR_V;
        }
        self.set_status(status);
    }

    fn set_extended_sub_flags(&mut self, left: u32, right: u32, result: u32, bits: u32) {
        let mask = (1_u32 << bits) - 1;
        let sign = 1_u32 << (bits - 1);
        let value = result & mask;
        let mut status = self.status() & !(SR_C | SR_Z | SR_N | SR_V);
        if left & mask >= right & mask {
            status |= SR_C;
        }
        if value == 0 {
            status |= SR_Z;
        }
        if value & sign != 0 {
            status |= SR_N;
        }
        if ((left ^ right) & (left ^ value) & sign) != 0 {
            status |= SR_V;
        }
        self.set_status(status);
    }

    fn execute_extended_double(
        &mut self,
        instruction: u16,
        source_extension: u16,
        destination_extension: u16,
        address_size: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let opcode = instruction >> 12;
        let source_register = usize::from((instruction >> 8) & 0xf);
        let destination_register = usize::from(instruction & 0xf);
        let source_mode = (instruction >> 4) & 3;
        let destination_indexed = instruction & 0x0080 != 0;
        let byte = !address_size && instruction & 0x0040 != 0;
        let bits = if address_size {
            20
        } else if byte {
            8
        } else {
            16
        };
        let mask = (1_u32 << bits) - 1;
        let source = self.extended_source(
            source_register,
            source_mode,
            source_extension,
            address_size,
            byte,
            bus,
            now,
        )?;
        let target = self.extended_destination(
            destination_register,
            destination_indexed,
            destination_extension,
            bus,
            now,
        )?;
        let destination = if opcode == 4 {
            0
        } else {
            self.extended_target_read(target, address_size, byte, bus, now)?
        };
        let mut write = true;
        let value = match opcode {
            4 => source,
            5 | 6 => {
                let carry = u32::from(opcode == 6 && self.status() & SR_C != 0);
                let right = (source + carry) & mask;
                let result = u64::from(destination) + u64::from(source) + u64::from(carry);
                self.set_extended_add_flags(destination, right, result, bits);
                result as u32 & mask
            }
            7 | 8 | 9 => {
                let borrow = u32::from(opcode == 7 && self.status() & SR_C == 0);
                let right = source.wrapping_add(borrow) & mask;
                let result = destination.wrapping_sub(right) & mask;
                self.set_extended_sub_flags(destination, right, result, bits);
                write = opcode != 9;
                result
            }
            10 => {
                let mut carry = u32::from(self.status() & SR_C != 0);
                let mut result = 0_u32;
                for digit in 0..bits / 4 {
                    let shift = digit * 4;
                    let sum = ((destination >> shift) & 0xf) + ((source >> shift) & 0xf) + carry;
                    let adjusted = if sum > 9 { sum + 6 } else { sum };
                    result |= (adjusted & 0xf) << shift;
                    carry = u32::from(adjusted > 0xf);
                }
                self.set_extended_nz(result, bits);
                let mut status = self.status() & !(SR_C | SR_V);
                if carry != 0 {
                    status |= SR_C;
                }
                self.set_status(status);
                result & mask
            }
            11 => {
                let result = destination & source;
                self.set_extended_nz(result, bits);
                let mut status = self.status() & !(SR_C | SR_V);
                if result != 0 {
                    status |= SR_C;
                }
                self.set_status(status);
                write = false;
                result
            }
            12 => destination & !source,
            13 => destination | source,
            14 => {
                let result = destination ^ source;
                self.set_extended_nz(result, bits);
                let sign = 1_u32 << (bits - 1);
                let mut status = self.status() & !(SR_C | SR_V);
                if result != 0 {
                    status |= SR_C;
                }
                if destination & source & sign != 0 {
                    status |= SR_V;
                }
                self.set_status(status);
                result
            }
            15 => {
                let result = destination & source;
                self.set_extended_nz(result, bits);
                let mut status = self.status() & !(SR_C | SR_V);
                if result != 0 {
                    status |= SR_C;
                }
                self.set_status(status);
                result
            }
            _ => {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    format!("unsupported extended double instruction {instruction:#06x}"),
                ));
            }
        };
        if write {
            self.extended_target_write(target, address_size, byte, value, bus, now)?;
        }
        Ok(StepReason::Advanced)
    }

    fn execute_extended_single(
        &mut self,
        instruction: u16,
        source_extension: u16,
        _destination_extension: u16,
        address_size: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let operation = (instruction >> 7) & 7;
        let byte = !address_size && instruction & 0x0040 != 0;
        let mode = (instruction >> 4) & 3;
        let register = usize::from(instruction & 0xf);
        if operation == 4 {
            let value = self.extended_source(
                register,
                mode,
                source_extension,
                address_size,
                byte,
                bus,
                now,
            )?;
            if address_size {
                self.push_address(bus, value, now)?;
            } else {
                self.push(bus, value as u16, now)?;
            }
            return Ok(StepReason::Advanced);
        }
        if operation == 5 {
            let value =
                self.extended_source(register, mode, source_extension, true, false, bus, now)?;
            self.push_address(bus, self.registers[0], now)?;
            self.registers[0] = value & ADDRESS_MASK;
            return Ok(StepReason::Advanced);
        }
        if operation > 3 {
            return Err(self.fault(
                CpuFaultKind::IllegalInstruction,
                format!("unsupported extended single instruction {instruction:#06x}"),
            ));
        }
        let target = self.extended_single_destination(
            register,
            mode,
            source_extension,
            address_size,
            byte,
            bus,
            now,
        )?;
        let bits = if address_size {
            20
        } else if byte {
            8
        } else {
            16
        };
        let mask = (1_u32 << bits) - 1;
        let old = self.extended_target_read(target, address_size, byte, bus, now)?;
        let value = match operation {
            0 => {
                let carry = old & 1 != 0;
                let result = (old >> 1) | (u32::from(self.status() & SR_C != 0) << (bits - 1));
                self.set_extended_nz(result, bits);
                let mut status = self.status() & !(SR_C | SR_V);
                if carry {
                    status |= SR_C;
                }
                if (status & SR_N != 0) ^ (status & SR_C != 0) {
                    status |= SR_V;
                }
                self.set_status(status);
                result
            }
            1 => ((old << 8) | ((old >> 8) & 0xff)) & mask,
            2 => {
                let carry = old & 1 != 0;
                let sign = old & (1_u32 << (bits - 1));
                let result = (old >> 1) | sign;
                self.set_extended_nz(result, bits);
                let mut status = self.status() & !(SR_C | SR_V);
                if carry {
                    status |= SR_C;
                }
                self.set_status(status);
                result
            }
            _ => {
                let result = (old as u8 as i8 as i32 as u32) & mask;
                self.set_extended_nz(result, bits);
                let mut status = self.status() & !(SR_C | SR_V);
                if result != 0 {
                    status |= SR_C;
                }
                self.set_status(status);
                result
            }
        };
        self.extended_target_write(target, address_size, byte, value, bus, now)?;
        Ok(StepReason::Advanced)
    }

    fn execute_mova(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let source_field = u32::from((instruction >> 8) & 0x0f);
        let mode = (instruction >> 4) & 0x0f;
        let destination = usize::from(instruction & 0x0f);
        let register_source = usize::try_from(source_field).expect("four-bit register fits usize");
        let mut write = true;
        let value = match mode {
            0 => self.read_address(bus, self.registers[register_source], AccessKind::Read, now)?,
            1 => {
                let address = self.registers[register_source];
                let value = self.read_address(bus, address, AccessKind::Read, now)?;
                self.registers[register_source] = address.wrapping_add(4) & ADDRESS_MASK;
                value
            }
            2 => {
                let address = (source_field << 16) | u32::from(self.fetch(bus, now)?);
                self.read_address(bus, address, AccessKind::Read, now)?
            }
            3 => {
                let displacement = self.fetch(bus, now)? as i16;
                let address = self.registers[register_source]
                    .wrapping_add_signed(i32::from(displacement))
                    & ADDRESS_MASK;
                self.read_address(bus, address, AccessKind::Read, now)?
            }
            6 => {
                let address = (u32::try_from(destination).expect("register fits u32") << 16)
                    | u32::from(self.fetch(bus, now)?);
                self.write_address(bus, address, self.registers[register_source], now)?;
                write = false;
                0
            }
            7 => {
                let displacement = self.fetch(bus, now)? as i16;
                let address = self.registers[destination]
                    .wrapping_add_signed(i32::from(displacement))
                    & ADDRESS_MASK;
                self.write_address(bus, address, self.registers[register_source], now)?;
                write = false;
                0
            }
            8..=11 => {
                let immediate = (source_field << 16) | u32::from(self.fetch(bus, now)?);
                match mode {
                    8 => immediate,
                    9 => {
                        let result =
                            self.registers[destination].wrapping_sub(immediate) & ADDRESS_MASK;
                        self.set_extended_sub_flags(
                            self.registers[destination],
                            immediate,
                            result,
                            20,
                        );
                        write = false;
                        0
                    }
                    10 => {
                        let result = u64::from(self.registers[destination]) + u64::from(immediate);
                        self.set_extended_add_flags(
                            self.registers[destination],
                            immediate,
                            result,
                            20,
                        );
                        result as u32 & ADDRESS_MASK
                    }
                    _ => {
                        let result =
                            self.registers[destination].wrapping_sub(immediate) & ADDRESS_MASK;
                        self.set_extended_sub_flags(
                            self.registers[destination],
                            immediate,
                            result,
                            20,
                        );
                        result
                    }
                }
            }
            12 => self.registers[register_source],
            13 => {
                let source = self.registers[register_source];
                let result = self.registers[destination].wrapping_sub(source) & ADDRESS_MASK;
                self.set_extended_sub_flags(self.registers[destination], source, result, 20);
                write = false;
                0
            }
            14 => {
                let source = self.registers[register_source];
                let result = u64::from(self.registers[destination]) + u64::from(source);
                self.set_extended_add_flags(self.registers[destination], source, result, 20);
                result as u32 & ADDRESS_MASK
            }
            15 => {
                let source = self.registers[register_source];
                let result = self.registers[destination].wrapping_sub(source) & ADDRESS_MASK;
                self.set_extended_sub_flags(self.registers[destination], source, result, 20);
                result
            }
            _ => {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    format!("unsupported MOVA-family instruction {instruction:#06x}"),
                ));
            }
        };
        if write {
            self.set_register_value(destination, value, false);
        }
        Ok(StepReason::Advanced)
    }

    fn calla_source(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<u32, CpuFault> {
        let form = (instruction >> 4) & 0x0f;
        let field = usize::from(instruction & 0x0f);
        match form {
            4..=7 => {
                let mode = form & 3;
                match (field, mode) {
                    (3, 0) => Ok(0),
                    (3, 1) => Ok(1),
                    (3, 2) => Ok(2),
                    (3, 3) => Ok(ADDRESS_MASK),
                    (2, 2) => Ok(4),
                    (2, 3) => Ok(8),
                    (_, 0) => Ok(self.registers[field]),
                    (_, 1) => {
                        let displacement = self.fetch(bus, now)?;
                        let address = match field {
                            0 => Self::classic_index(self.registers[0], displacement),
                            2 => u32::from(displacement),
                            _ => Self::classic_index(self.registers[field], displacement),
                        };
                        self.read_address(bus, address, AccessKind::Read, now)
                    }
                    (_, 2) => self.read_address(bus, self.registers[field], AccessKind::Read, now),
                    (0, 3) => self.fetch(bus, now).map(u32::from),
                    (_, 3) => {
                        let address = self.registers[field];
                        let value = self.read_address(bus, address, AccessKind::Read, now)?;
                        self.registers[field] = address.wrapping_add(4) & ADDRESS_MASK;
                        Ok(value)
                    }
                    _ => unreachable!(),
                }
            }
            8 => {
                let address = (u32::try_from(field).expect("field fits u32") << 16)
                    | u32::from(self.fetch(bus, now)?);
                self.read_address(bus, address, AccessKind::Read, now)
            }
            9 => {
                let low = u32::from(self.fetch(bus, now)?);
                let raw = (u32::try_from(field).expect("field fits u32") << 16) | low;
                let displacement = if raw & 0x80000 != 0 {
                    raw | !ADDRESS_MASK
                } else {
                    raw
                };
                let address = self.registers[0].wrapping_add(displacement) & ADDRESS_MASK;
                self.read_address(bus, address, AccessKind::Read, now)
            }
            11 => Ok((u32::try_from(field).expect("field fits u32") << 16)
                | u32::from(self.fetch(bus, now)?)),
            _ => Err(self.fault(
                CpuFaultKind::IllegalInstruction,
                format!("unsupported CALLA instruction {instruction:#06x}"),
            )),
        }
    }

    fn source(
        &mut self,
        register: usize,
        mode: u16,
        byte: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<u16, CpuFault> {
        let mask = if byte { 0x00ff } else { 0xffff };
        let constant = match (register, mode) {
            (3, 0) => Some(0),
            (3, 1) => Some(1),
            (3, 2) => Some(2),
            (3, 3) => Some(mask),
            (2, 2) => Some(4),
            (2, 3) => Some(8),
            _ => None,
        };
        if let Some(value) = constant {
            return Ok(value);
        }
        match mode {
            0 => Ok(self.registers[register] as u16 & mask),
            1 => {
                let offset = self.fetch(bus, now)?;
                let address = match register {
                    0 => Self::classic_index(self.registers[0], offset),
                    2 => u32::from(offset),
                    _ => Self::classic_index(self.registers[register], offset),
                };
                self.read(bus, address, byte, AccessKind::Read, now)
            }
            2 => self.read(bus, self.registers[register], byte, AccessKind::Read, now),
            3 if register == 0 => self.fetch(bus, now).map(|value| value & mask),
            3 => {
                let address = self.registers[register];
                let value = self.read(bus, address, byte, AccessKind::Read, now)?;
                let increment = if byte && register != 1 { 1 } else { 2 };
                self.registers[register] = address.wrapping_add(increment) & ADDRESS_MASK;
                Ok(value)
            }
            _ => unreachable!(),
        }
    }

    fn destination(
        &mut self,
        register: usize,
        indexed: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<OperandTarget, CpuFault> {
        if !indexed {
            return Ok(OperandTarget::Register(register));
        }
        let offset = self.fetch(bus, now)?;
        let address = match register {
            0 => Self::classic_index(self.registers[0], offset),
            2 => u32::from(offset),
            _ => Self::classic_index(self.registers[register], offset),
        };
        Ok(OperandTarget::Memory(address))
    }

    fn single_destination(
        &mut self,
        register: usize,
        mode: u16,
        byte: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<OperandTarget, CpuFault> {
        match mode {
            0 => Ok(OperandTarget::Register(register)),
            1 => self.destination(register, true, bus, now),
            2 => Ok(OperandTarget::Memory(self.registers[register])),
            3 => {
                let address = self.registers[register];
                let increment = if byte && register != 1 { 1 } else { 2 };
                self.registers[register] = address.wrapping_add(increment) & ADDRESS_MASK;
                Ok(OperandTarget::Memory(address))
            }
            _ => unreachable!(),
        }
    }

    fn target_read(
        &self,
        target: OperandTarget,
        byte: bool,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<u16, CpuFault> {
        match target {
            OperandTarget::Register(register) => Ok(if byte {
                self.registers[register] as u16 & 0xff
            } else {
                self.registers[register] as u16
            }),
            OperandTarget::Memory(address) => self.read(bus, address, byte, AccessKind::Read, now),
        }
    }

    fn target_write(
        &mut self,
        target: OperandTarget,
        byte: bool,
        value: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        match target {
            OperandTarget::Register(register) => {
                self.set_register_value(register, u32::from(value), byte);
                Ok(())
            }
            OperandTarget::Memory(address) => self.write(bus, address, byte, value, now),
        }
    }

    fn set_nz(&mut self, value: u16, byte: bool) {
        let sign = if byte { 0x80 } else { 0x8000 };
        let mask = if byte { 0xff } else { 0xffff };
        let mut status = self.status() & !(SR_N | SR_Z);
        if value & mask == 0 {
            status |= SR_Z;
        }
        if value & sign != 0 {
            status |= SR_N;
        }
        self.set_status(status);
    }

    fn set_add_flags(&mut self, left: u16, right: u16, result: u32, byte: bool) {
        let sign = if byte { 0x80_u32 } else { 0x8000 };
        let mask = if byte { 0xff_u32 } else { 0xffff };
        let value = result & mask;
        let mut status = self.status() & !(SR_C | SR_Z | SR_N | SR_V);
        if result > mask {
            status |= SR_C;
        }
        if value == 0 {
            status |= SR_Z;
        }
        if value & sign != 0 {
            status |= SR_N;
        }
        if (!(u32::from(left) ^ u32::from(right)) & (u32::from(left) ^ value) & sign) != 0 {
            status |= SR_V;
        }
        self.set_status(status);
    }

    fn set_sub_flags(&mut self, left: u16, right: u16, result: u32, byte: bool) {
        let sign = if byte { 0x80_u32 } else { 0x8000 };
        let mask = if byte { 0xff_u32 } else { 0xffff };
        let value = result & mask;
        let mut status = self.status() & !(SR_C | SR_Z | SR_N | SR_V);
        if u32::from(left) >= u32::from(right) {
            status |= SR_C;
        }
        if value == 0 {
            status |= SR_Z;
        }
        if value & sign != 0 {
            status |= SR_N;
        }
        if ((u32::from(left) ^ u32::from(right)) & (u32::from(left) ^ value) & sign) != 0 {
            status |= SR_V;
        }
        self.set_status(status);
    }

    fn condition(&self, condition: u16) -> bool {
        let status = self.status();
        match condition {
            0 => status & SR_Z == 0,
            1 => status & SR_Z != 0,
            2 => status & SR_C == 0,
            3 => status & SR_C != 0,
            4 => status & SR_N != 0,
            5 => (status & SR_N != 0) == (status & SR_V != 0),
            6 => (status & SR_N != 0) != (status & SR_V != 0),
            _ => true,
        }
    }

    pub(super) fn execute(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        if instruction == 0 {
            self.halted = true;
            return Ok(StepReason::Halted);
        }
        if instruction & 0xf800 == 0x1800 {
            let source_extension = ((instruction >> 8) & 7) * 2 + ((instruction >> 7) & 1);
            let address_length = instruction & 0x0040 == 0;
            let destination_extension = instruction & 0x000f;
            let extended_instruction = self.fetch(bus, now)?;
            let register_only = if extended_instruction >> 12 >= 4 {
                extended_instruction & 0x00b0 == 0
            } else {
                extended_instruction & 0xfc00 == 0x1000 && extended_instruction & 0x0030 == 0
            };
            if !register_only {
                if address_length {
                    return self.execute_address_extension(
                        instruction,
                        source_extension,
                        destination_extension,
                        extended_instruction,
                        bus,
                        now,
                    );
                }
                if extended_instruction >> 12 >= 4 {
                    return self.execute_extended_double(
                        extended_instruction,
                        source_extension,
                        destination_extension,
                        false,
                        bus,
                        now,
                    );
                }
                if extended_instruction & 0xfc00 == 0x1000 {
                    return self.execute_extended_single(
                        extended_instruction,
                        source_extension,
                        destination_extension,
                        false,
                        bus,
                        now,
                    );
                }
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    format!("extension before unsupported instruction {extended_instruction:#06x}"),
                ));
            }
            let repeats = if source_extension & 1 == 0 {
                u32::from(destination_extension)
            } else {
                self.registers[usize::from(destination_extension)] & 0x0f
            };
            let zero_carry = source_extension & 2 != 0;
            let mut reason = StepReason::Advanced;
            for _ in 0..=repeats {
                if zero_carry {
                    self.set_status(self.status() & !SR_C);
                }
                reason = if address_length {
                    if extended_instruction >> 12 >= 4 {
                        self.execute_extended_double(extended_instruction, 0, 0, true, bus, now)?
                    } else {
                        self.execute_extended_single(extended_instruction, 0, 0, true, bus, now)?
                    }
                } else {
                    self.execute(extended_instruction, bus, now)?
                };
                if reason != StepReason::Advanced {
                    break;
                }
            }
            return Ok(reason);
        }
        if instruction & 0xfe00 == 0x1400 {
            let word = instruction & 0x0100 != 0;
            let count = usize::from((instruction >> 4) & 0x0f) + 1;
            let top_register = usize::from(instruction & 0x0f);
            if count > top_register + 1 {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    format!("PUSHM register range underflows in {instruction:#06x}"),
                ));
            }
            for register in (top_register + 1 - count..=top_register).rev() {
                if word {
                    self.push(bus, self.registers[register] as u16, now)?;
                } else {
                    self.push_address(bus, self.registers[register], now)?;
                }
            }
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfe00 == 0x1600 {
            let word = instruction & 0x0100 != 0;
            let count = usize::from((instruction >> 4) & 0x0f) + 1;
            let bottom_register = usize::from(instruction & 0x0f);
            if bottom_register + count > self.registers.len() {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    format!("POPM register range overflows in {instruction:#06x}"),
                ));
            }
            for register in bottom_register..bottom_register + count {
                let value = if word {
                    u32::from(self.pop(bus, now)?)
                } else {
                    self.pop_address(bus, now)?
                };
                self.set_register_value(register, value, false);
            }
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xf0e0 == 0x0040 {
            let count = u32::from((instruction >> 10) & 3) + 1;
            let operation = (instruction >> 8) & 3;
            let word = instruction & 0x0010 != 0;
            let register = usize::from(instruction & 0x000f);
            let bits = if word { 16 } else { 20 };
            let mask = if word { 0xffff_u32 } else { ADDRESS_MASK };
            let sign = 1_u32 << (bits - 1);
            let mut value = self.registers[register] & mask;
            for _ in 0..count {
                let carry = match operation {
                    0 => {
                        let carry = value & 1 != 0;
                        value = (value >> 1) | (u32::from(self.status() & SR_C != 0) << (bits - 1));
                        carry
                    }
                    1 => {
                        let carry = value & 1 != 0;
                        value = (value >> 1) | (value & sign);
                        carry
                    }
                    2 => {
                        let carry = value & sign != 0;
                        value = (value << 1) & mask;
                        carry
                    }
                    _ => {
                        let carry = value & 1 != 0;
                        value >>= 1;
                        carry
                    }
                };
                let mut status = self.status() & !(SR_C | SR_Z | SR_N | SR_V);
                if carry {
                    status |= SR_C;
                }
                if value == 0 {
                    status |= SR_Z;
                }
                if value & sign != 0 {
                    status |= SR_N;
                }
                self.set_status(status);
            }
            self.set_register_value(register, value, false);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xf000 == 0 {
            let mode = (instruction >> 4) & 0x0f;
            if matches!(mode, 0..=3 | 6..=15) {
                return self.execute_mova(instruction, bus, now);
            }
        }
        if instruction & 0xff00 == 0x1300 && instruction != 0x1300 {
            let form = (instruction >> 4) & 0x0f;
            if matches!(form, 4..=9 | 11) {
                let target = self.calla_source(instruction, bus, now)?;
                self.push_address(bus, self.registers[0], now)?;
                self.registers[0] = target & ADDRESS_MASK;
                return Ok(StepReason::Advanced);
            }
        }
        if instruction & 0xe000 == 0x2000 {
            let condition = (instruction >> 10) & 7;
            if self.condition(condition) {
                let offset = (((instruction & 0x03ff) << 6) as i16 >> 5) as i32;
                self.registers[0] = self.registers[0].wrapping_add_signed(offset) & ADDRESS_MASK;
            }
            return Ok(StepReason::Advanced);
        }
        if instruction == 0x1300 {
            let stacked_status = self.pop(bus, now)?;
            let pc_low = self.pop(bus, now)?;
            self.set_status(stacked_status & 0x0fff);
            self.registers[0] = u32::from(pc_low) | (u32::from(stacked_status >> 12) << 16);
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfc00 == 0x1000 {
            let operation = (instruction >> 7) & 7;
            let byte = instruction & 0x0040 != 0;
            let mode = (instruction >> 4) & 3;
            let register = usize::from(instruction & 0xf);
            if operation == 4 {
                let value = self.source(register, mode, byte, bus, now)?;
                self.push(bus, value, now)?;
                return Ok(StepReason::Advanced);
            }
            if operation == 5 {
                let value = self.source(register, mode, false, bus, now)?;
                self.push(bus, self.registers[0] as u16, now)?;
                self.registers[0] = u32::from(value);
                return Ok(StepReason::Advanced);
            }
            let target = self.single_destination(register, mode, byte, bus, now)?;
            let old = self.target_read(target, byte, bus, now)?;
            let result = match operation {
                0 => {
                    let carry = old & 1;
                    let width = if byte { 8 } else { 16 };
                    let value = (old >> 1) | (u16::from(self.status() & SR_C != 0) << (width - 1));
                    let mut status = self.status() & !(SR_C | SR_Z | SR_N | SR_V);
                    if carry != 0 {
                        status |= SR_C;
                    }
                    self.set_status(status);
                    self.set_nz(value, byte);
                    value
                }
                1 => old.rotate_left(8),
                2 => {
                    let value = if byte {
                        ((old as u8 as i8) >> 1) as u8 as u16
                    } else {
                        ((old as i16) >> 1) as u16
                    };
                    let carry = old & 1 != 0;
                    self.set_nz(value, byte);
                    let mut status = self.status() & !SR_C;
                    if carry {
                        status |= SR_C;
                    }
                    self.set_status(status);
                    value
                }
                3 => {
                    let value = u16::from(old as u8 as i8 as i16 as u16);
                    self.set_nz(value, false);
                    let mut status = self.status() & !(SR_C | SR_V);
                    if value != 0 {
                        status |= SR_C;
                    }
                    self.set_status(status);
                    value
                }
                _ => {
                    return Err(self.fault(
                        CpuFaultKind::IllegalInstruction,
                        format!("unsupported MSP430 single-operand instruction {instruction:#06x}"),
                    ));
                }
            };
            self.target_write(target, byte, result, bus, now)?;
            return Ok(StepReason::Advanced);
        }
        let opcode = instruction >> 12;
        if opcode < 4 {
            return Err(self.fault(
                CpuFaultKind::IllegalInstruction,
                format!("unsupported MSP430X instruction {instruction:#06x}"),
            ));
        }
        let source_register = usize::from((instruction >> 8) & 0xf);
        let destination_register = usize::from(instruction & 0xf);
        let source_mode = (instruction >> 4) & 3;
        let destination_indexed = instruction & 0x0080 != 0;
        let byte = instruction & 0x0040 != 0;
        let source = self.source(source_register, source_mode, byte, bus, now)?;
        let target = self.destination(destination_register, destination_indexed, bus, now)?;
        let destination = if opcode == 4 {
            0
        } else {
            self.target_read(target, byte, bus, now)?
        };
        let mask = if byte { 0xff_u32 } else { 0xffff };
        let mut write = true;
        let value = match opcode {
            4 => source,
            5 | 6 => {
                let carry = u32::from(opcode == 6 && self.status() & SR_C != 0);
                let result = u32::from(destination) + u32::from(source) + carry;
                self.set_add_flags(destination, source.wrapping_add(carry as u16), result, byte);
                (result & mask) as u16
            }
            7 | 8 | 9 => {
                let borrow = u32::from(opcode == 7 && self.status() & SR_C == 0);
                let right = u32::from(source) + borrow;
                let result = u32::from(destination).wrapping_sub(right);
                self.set_sub_flags(destination, right as u16, result, byte);
                write = opcode != 9;
                (result & mask) as u16
            }
            10 => {
                let mut carry = u16::from(self.status() & SR_C != 0);
                let mut result = 0_u16;
                let digits = if byte { 2 } else { 4 };
                for digit in 0..digits {
                    let shift = digit * 4;
                    let sum = ((destination >> shift) & 0xf) + ((source >> shift) & 0xf) + carry;
                    let adjusted = if sum > 9 { sum + 6 } else { sum };
                    result |= (adjusted & 0xf) << shift;
                    carry = u16::from(adjusted > 0xf);
                }
                self.set_nz(result, byte);
                let mut status = self.status() & !(SR_C | SR_V);
                if carry != 0 {
                    status |= SR_C;
                }
                self.set_status(status);
                result
            }
            11 => {
                let result = destination & source;
                self.set_nz(result, byte);
                let mut status = self.status() & !(SR_C | SR_V);
                if result != 0 {
                    status |= SR_C;
                }
                self.set_status(status);
                write = false;
                result
            }
            12 => destination & !source,
            13 => destination | source,
            14 => {
                let result = destination ^ source;
                self.set_nz(result, byte);
                let sign = if byte { 0x80 } else { 0x8000 };
                let mut status = self.status() & !(SR_C | SR_V);
                if result != 0 {
                    status |= SR_C;
                }
                if destination & source & sign != 0 {
                    status |= SR_V;
                }
                self.set_status(status);
                result
            }
            15 => {
                let result = destination & source;
                self.set_nz(result, byte);
                let mut status = self.status() & !(SR_C | SR_V);
                if result != 0 {
                    status |= SR_C;
                }
                self.set_status(status);
                result
            }
            _ => unreachable!(),
        };
        if write {
            self.target_write(target, byte, value, bus, now)?;
        }
        Ok(StepReason::Advanced)
    }
}
