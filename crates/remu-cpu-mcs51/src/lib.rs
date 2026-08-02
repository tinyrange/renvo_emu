//! Reusable interpreted MCS-51 core with explicit Harvard memory spaces.

use remu_core::{
    AccessKind, AccessWidth, Architecture, Bus, Cpu, CpuFault, CpuFaultKind, CpuSnapshot,
    RegisterValue, ResetKind, SimDuration, SimTime, StepOutcome,
};
use serde::{Deserialize, Serialize};

mod execution;

const CODE_BYTES: usize = 32 * 1024;
const SFR_BUS_BASE: u64 = 0x1_0000;
const PSW_C: u8 = 0x80;
const PSW_AC: u8 = 0x40;
const PSW_OV: u8 = 0x04;
const PSW_P: u8 = 0x01;

/// Distinct MCS-51 memory/access spaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mcs51AddressSpace {
    /// Instruction/code bytes.
    Code,
    /// Direct and indirect internal data RAM.
    InternalData,
    /// External MOVX data space.
    ExternalData,
    /// Special-function-register pages.
    Sfr,
    /// Bit-addressable internal/SFR view.
    Bit,
}

/// Named MCS-51 debugger register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mcs51Register {
    /// Accumulator.
    A,
    /// B arithmetic register.
    B,
    /// Data pointer.
    Dptr,
    /// Program status word.
    Psw,
    /// Stack pointer.
    Sp,
    /// Active-bank register r0.
    R0,
    /// Active-bank register r1.
    R1,
    /// Active-bank register r2.
    R2,
    /// Active-bank register r3.
    R3,
    /// Active-bank register r4.
    R4,
    /// Active-bank register r5.
    R5,
    /// Active-bank register r6.
    R6,
    /// Active-bank register r7.
    R7,
    /// Program counter.
    Pc,
}

impl Mcs51Register {
    /// Stable debugger order.
    pub const ALL: [Self; 14] = [
        Self::A,
        Self::B,
        Self::Dptr,
        Self::Psw,
        Self::Sp,
        Self::R0,
        Self::R1,
        Self::R2,
        Self::R3,
        Self::R4,
        Self::R5,
        Self::R6,
        Self::R7,
        Self::Pc,
    ];

    /// Stable debugger name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
            Self::Dptr => "dptr",
            Self::Psw => "psw",
            Self::Sp => "sp",
            Self::R0 => "r0",
            Self::R1 => "r1",
            Self::R2 => "r2",
            Self::R3 => "r3",
            Self::R4 => "r4",
            Self::R5 => "r5",
            Self::R6 => "r6",
            Self::R7 => "r7",
            Self::Pc => "pc",
        }
    }

    /// GDB register number derived from the named variant.
    pub fn gdb_number(self) -> usize {
        Self::ALL
            .iter()
            .position(|register| *register == self)
            .expect("register appears in ALL")
    }

    /// Meaningful register width.
    pub const fn bits(self) -> u8 {
        match self {
            Self::Dptr | Self::Pc => 16,
            _ => 8,
        }
    }
}

/// Deterministic base MCS-51 interpreter.
pub struct Mcs51Cpu {
    code: Box<[u8]>,
    idata: [u8; 256],
    a: u8,
    b: u8,
    dptr: u16,
    psw: u8,
    sp: u8,
    pc: u16,
    sfr_page: u8,
    interrupts: [bool; 20],
    last_interrupt_line: Option<u8>,
    active_priority: Option<bool>,
    priority_stack: Vec<Option<bool>>,
    sfr_page_stack: Vec<u8>,
    waiting: bool,
    halted: bool,
}

impl Default for Mcs51Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Mcs51Cpu {
    /// Constructs an erased 32 KiB MCS-51 code store in reset state.
    pub fn new() -> Self {
        Self {
            code: vec![0xff; CODE_BYTES].into_boxed_slice(),
            idata: [0; 256],
            a: 0,
            b: 0,
            dptr: 0,
            psw: 0,
            sp: 7,
            pc: 0,
            sfr_page: 0,
            interrupts: [false; 20],
            last_interrupt_line: None,
            active_priority: None,
            priority_stack: Vec::new(),
            sfr_page_stack: Vec::new(),
            waiting: false,
            halted: false,
        }
    }

    /// Loads code bytes at the requested code-space address.
    pub fn load_code(&mut self, address: u16, bytes: &[u8]) -> Result<(), CpuFault> {
        let start = usize::from(address);
        let end = start
            .checked_add(bytes.len())
            .ok_or_else(|| self.fault(CpuFaultKind::Architecture, "code range overflow"))?;
        if end > CODE_BYTES {
            return Err(self.fault(
                CpuFaultKind::Architecture,
                format!("code image ends at {end:#x}, beyond 32 KiB flash"),
            ));
        }
        self.code[start..end].copy_from_slice(bytes);
        Ok(())
    }

    /// Reads one code byte for diagnostics and execution recording.
    pub fn code_byte(&self, address: u16) -> Option<u8> {
        self.code.get(usize::from(address)).copied()
    }

    /// Reads a named architectural register.
    pub fn register(&self, register: Mcs51Register) -> u64 {
        match register {
            Mcs51Register::A => u64::from(self.a),
            Mcs51Register::B => u64::from(self.b),
            Mcs51Register::Dptr => u64::from(self.dptr),
            Mcs51Register::Psw => u64::from(self.psw),
            Mcs51Register::Sp => u64::from(self.sp),
            Mcs51Register::R0 => u64::from(self.reg(0)),
            Mcs51Register::R1 => u64::from(self.reg(1)),
            Mcs51Register::R2 => u64::from(self.reg(2)),
            Mcs51Register::R3 => u64::from(self.reg(3)),
            Mcs51Register::R4 => u64::from(self.reg(4)),
            Mcs51Register::R5 => u64::from(self.reg(5)),
            Mcs51Register::R6 => u64::from(self.reg(6)),
            Mcs51Register::R7 => u64::from(self.reg(7)),
            Mcs51Register::Pc => u64::from(self.pc),
        }
    }

    /// Sets a named register for debugger and test use.
    pub fn set_register(&mut self, register: Mcs51Register, value: u64) {
        let bytes = value.to_le_bytes();
        let byte = bytes[0];
        let word = u16::from_le_bytes([bytes[0], bytes[1]]);
        match register {
            Mcs51Register::A => self.a = byte,
            Mcs51Register::B => self.b = byte,
            Mcs51Register::Dptr => self.dptr = word,
            Mcs51Register::Psw => self.psw = byte,
            Mcs51Register::Sp => self.sp = byte,
            Mcs51Register::R0 => self.set_reg(0, byte),
            Mcs51Register::R1 => self.set_reg(1, byte),
            Mcs51Register::R2 => self.set_reg(2, byte),
            Mcs51Register::R3 => self.set_reg(3, byte),
            Mcs51Register::R4 => self.set_reg(4, byte),
            Mcs51Register::R5 => self.set_reg(5, byte),
            Mcs51Register::R6 => self.set_reg(6, byte),
            Mcs51Register::R7 => self.set_reg(7, byte),
            Mcs51Register::Pc => self.pc = word,
        }
        self.update_parity();
    }

    /// Returns the interrupt line consumed by the most recent CPU step.
    ///
    /// A machine model can use this acknowledgement point to apply
    /// architecture-specific side effects that occur when the core vectors
    /// to an interrupt handler, such as clearing an edge/overflow flag.
    pub fn last_interrupt_line(&self) -> Option<u8> {
        self.last_interrupt_line
    }

    fn fault(&self, kind: CpuFaultKind, message: impl Into<String>) -> CpuFault {
        CpuFault::new(kind, u64::from(self.pc), message)
    }

    fn fetch8(&mut self) -> Result<u8, CpuFault> {
        let value = *self
            .code
            .get(usize::from(self.pc))
            .ok_or_else(|| self.fault(CpuFaultKind::Bus, "program counter outside EFM8 flash"))?;
        self.pc = self.pc.wrapping_add(1);
        Ok(value)
    }

    fn fetch16(&mut self) -> Result<u16, CpuFault> {
        let high = self.fetch8()?;
        let low = self.fetch8()?;
        Ok((u16::from(high) << 8) | u16::from(low))
    }

    fn code_read(&self, address: u16) -> Result<u8, CpuFault> {
        self.code
            .get(usize::from(address))
            .copied()
            .ok_or_else(|| self.fault(CpuFaultKind::Bus, "MOVC outside EFM8 flash"))
    }

    fn bank_base(&self) -> usize {
        usize::from((self.psw >> 3) & 3) * 8
    }

    fn reg(&self, index: u8) -> u8 {
        self.idata[self.bank_base() + usize::from(index & 7)]
    }

    fn set_reg(&mut self, index: u8, value: u8) {
        let address = self.bank_base() + usize::from(index & 7);
        self.idata[address] = value;
    }

    fn indirect(&self, index: u8) -> u8 {
        self.idata[usize::from(self.reg(index))]
    }

    fn set_indirect(&mut self, index: u8, value: u8) {
        let address = usize::from(self.reg(index));
        self.idata[address] = value;
    }

    fn sfr_bus_address(&self, address: u8) -> u64 {
        SFR_BUS_BASE + (u64::from(self.sfr_page) << 8) + u64::from(address)
    }

    fn sfr_read(&mut self, bus: &mut dyn Bus, address: u8, now: SimTime) -> Result<u8, CpuFault> {
        match address {
            0x81 => Ok(self.sp),
            0x82 => Ok(self.dptr.to_le_bytes()[0]),
            0x83 => Ok((self.dptr >> 8) as u8),
            0xa7 => Ok(self.sfr_page),
            0xd0 => Ok(self.psw),
            0xe0 => Ok(self.a),
            0xf0 => Ok(self.b),
            _ => bus
                .read(
                    self.sfr_bus_address(address),
                    AccessWidth::Byte,
                    AccessKind::Read,
                    now,
                )
                .map(|value| value.to_le_bytes()[0])
                .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string())),
        }
    }

    fn sfr_write(
        &mut self,
        bus: &mut dyn Bus,
        address: u8,
        value: u8,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        match address {
            0x81 => self.sp = value,
            0x82 => self.dptr = (self.dptr & 0xff00) | u16::from(value),
            0x83 => self.dptr = (self.dptr & 0x00ff) | (u16::from(value) << 8),
            0xa7 => self.sfr_page = value,
            0x87 => {
                self.waiting = value & 1 != 0;
                bus.write(
                    self.sfr_bus_address(address),
                    AccessWidth::Byte,
                    u64::from(value),
                    now,
                )
                .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))?;
            }
            0xd0 => self.psw = value,
            0xe0 => self.a = value,
            0xf0 => self.b = value,
            _ => bus
                .write(
                    self.sfr_bus_address(address),
                    AccessWidth::Byte,
                    u64::from(value),
                    now,
                )
                .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))?,
        }
        Ok(())
    }

    fn direct_read(
        &mut self,
        bus: &mut dyn Bus,
        address: u8,
        now: SimTime,
    ) -> Result<u8, CpuFault> {
        if address < 0x80 {
            Ok(self.idata[usize::from(address)])
        } else {
            self.sfr_read(bus, address, now)
        }
    }

    fn direct_write(
        &mut self,
        bus: &mut dyn Bus,
        address: u8,
        value: u8,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        if address < 0x80 {
            self.idata[usize::from(address)] = value;
            Ok(())
        } else {
            self.sfr_write(bus, address, value, now)
        }
    }

    fn bit_read(&mut self, bus: &mut dyn Bus, bit: u8, now: SimTime) -> Result<bool, CpuFault> {
        let (address, mask) = if bit < 0x80 {
            (0x20 + (bit >> 3), 1 << (bit & 7))
        } else {
            (bit & 0xf8, 1 << (bit & 7))
        };
        Ok(self.direct_read(bus, address, now)? & mask != 0)
    }

    fn bit_write(
        &mut self,
        bus: &mut dyn Bus,
        bit: u8,
        value: bool,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        let (address, mask) = if bit < 0x80 {
            (0x20 + (bit >> 3), 1 << (bit & 7))
        } else {
            (bit & 0xf8, 1 << (bit & 7))
        };
        let old = self.direct_read(bus, address, now)?;
        self.direct_write(
            bus,
            address,
            if value { old | mask } else { old & !mask },
            now,
        )
    }

    fn xdata_read(&self, bus: &mut dyn Bus, address: u16, now: SimTime) -> Result<u8, CpuFault> {
        bus.read(u64::from(address), AccessWidth::Byte, AccessKind::Read, now)
            .map(|value| value.to_le_bytes()[0])
            .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))
    }

    fn xdata_write(
        &self,
        bus: &mut dyn Bus,
        address: u16,
        value: u8,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        bus.write(u64::from(address), AccessWidth::Byte, u64::from(value), now)
            .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))
    }

    fn push_byte(&mut self, value: u8) {
        self.sp = self.sp.wrapping_add(1);
        self.idata[usize::from(self.sp)] = value;
    }

    fn pop_byte(&mut self) -> u8 {
        let value = self.idata[usize::from(self.sp)];
        self.sp = self.sp.wrapping_sub(1);
        value
    }

    fn push_pc(&mut self) {
        let [low, high] = self.pc.to_le_bytes();
        self.push_byte(low);
        self.push_byte(high);
    }

    fn pop_pc(&mut self) {
        let high = self.pop_byte();
        let low = self.pop_byte();
        self.pc = (u16::from(high) << 8) | u16::from(low);
    }

    fn carry(&self) -> bool {
        self.psw & PSW_C != 0
    }

    fn set_carry(&mut self, value: bool) {
        self.psw = (self.psw & !PSW_C) | if value { PSW_C } else { 0 };
    }

    fn update_parity(&mut self) {
        self.psw = (self.psw & !PSW_P) | u8::from(self.a.count_ones() & 1 != 0);
    }

    fn reset_state(&mut self) {
        self.idata.fill(0);
        self.a = 0;
        self.b = 0;
        self.dptr = 0;
        self.psw = 0;
        self.sp = 7;
        self.pc = 0;
        self.sfr_page = 0;
        self.interrupts = [false; 20];
        self.last_interrupt_line = None;
        self.active_priority = None;
        self.priority_stack.clear();
        self.sfr_page_stack.clear();
        self.waiting = false;
        self.halted = false;
    }

    fn pending_interrupt(&self) -> Option<(usize, bool)> {
        const LOW_LINES: [usize; 10] = [0, 1, 2, 6, 8, 10, 12, 14, 16, 18];
        const HIGH_LINES: [usize; 10] = [3, 4, 5, 7, 9, 11, 13, 15, 17, 19];
        for high in [true, false] {
            if high && self.active_priority == Some(true) {
                continue;
            }
            if !high && self.active_priority.is_some() {
                continue;
            }
            let lines = if high { &HIGH_LINES } else { &LOW_LINES };
            if let Some(line) = lines.iter().copied().find(|line| self.interrupts[*line]) {
                return Some((line, high));
            }
        }
        None
    }

    fn enter_interrupt(&mut self, line: usize, high: bool) {
        let vector = match line {
            0 | 3 => 0x000b,
            1 | 4 => 0x0023,
            2 | 5 => 0x002b,
            6 | 7 => 0x0033,
            8 | 9 => 0x001b,
            10 | 11 => 0x003b,
            12 | 13 => 0x007b,
            14 | 15 => 0x0073,
            16 | 17 => 0x008b,
            18 | 19 => 0x0093,
            _ => unreachable!("MCS-51 interrupt line is validated by pending_interrupt"),
        };
        self.push_pc();
        self.priority_stack.push(self.active_priority);
        self.sfr_page_stack.push(self.sfr_page);
        self.active_priority = Some(high);
        if matches!(line, 12 | 13) {
            self.sfr_page = 0x20;
        }
        self.pc = vector;
        self.waiting = false;
    }
}

impl Cpu for Mcs51Cpu {
    fn architecture(&self) -> Architecture {
        Architecture::Mcs51
    }

    fn reset(&mut self, _kind: ResetKind, _bus: &mut dyn Bus) -> Result<(), CpuFault> {
        self.reset_state();
        Ok(())
    }

    fn step(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<StepOutcome, CpuFault> {
        self.last_interrupt_line = None;
        if let Some((line, high)) = self.pending_interrupt() {
            self.enter_interrupt(line, high);
            self.last_interrupt_line = Some(line as u8);
            return Ok(StepOutcome::advanced(SimDuration::from_ticks(2)));
        }
        if self.waiting {
            return Ok(StepOutcome {
                elapsed: SimDuration::from_ticks(1),
                reason: remu_core::StepReason::WaitForInterrupt,
            });
        }
        let instruction_pc = self.pc;
        let opcode = self.fetch8()?;
        let outcome = self.execute(opcode, instruction_pc, bus, now)?;
        self.update_parity();
        Ok(outcome)
    }

    fn set_interrupt(&mut self, line: u16, asserted: bool) -> Result<(), CpuFault> {
        let slot = self.interrupts.get_mut(usize::from(line)).ok_or_else(|| {
            CpuFault::new(
                CpuFaultKind::Architecture,
                u64::from(self.pc),
                format!("MCS-51 interrupt line {line} is outside 0..19"),
            )
        })?;
        *slot = asserted;
        if asserted {
            self.waiting = false;
        }
        Ok(())
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot {
            architecture: Architecture::Mcs51,
            pc: u64::from(self.pc),
            registers: Mcs51Register::ALL
                .iter()
                .copied()
                .map(|register| RegisterValue {
                    name: register.name().to_owned(),
                    value: self.register(register),
                    bits: register.bits(),
                })
                .collect(),
            waiting: self.waiting,
            halted: self.halted,
        }
    }
}

#[cfg(test)]
mod tests;
