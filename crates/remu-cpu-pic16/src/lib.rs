//! Interpreted enhanced mid-range PIC16 core.

use remu_core::{
    AccessKind, AccessWidth, Architecture, Bus, Cpu, CpuFault, CpuFaultKind, CpuSnapshot,
    RegisterValue, ResetKind, SimDuration, SimTime, StepOutcome, StepReason,
};
use serde::{Deserialize, Serialize};

mod execution;

const PROGRAM_WORDS: usize = 16 * 1024;
const PROGRAM_MASK: u16 = 0x3fff;
const STATUS_C: u8 = 1 << 0;
const STATUS_DC: u8 = 1 << 1;
const STATUS_Z: u8 = 1 << 2;
const STATUS_PD: u8 = 1 << 3;
const STATUS_TO: u8 = 1 << 4;
const INTCON_GIE: u8 = 1 << 7;

/// PIC16 access space; program values are 14-bit words, not bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pic16AddressSpace {
    /// 14-bit word-addressed instruction space.
    Program,
    /// Banked byte data space.
    Data,
    /// Configuration words outside normal program execution.
    Configuration,
}

/// Named enhanced mid-range PIC16 debugger register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pic16Register {
    /// Working accumulator.
    Wreg,
    /// Status register.
    Status,
    /// Bank-select register.
    Bsr,
    /// File-select register zero.
    Fsr0,
    /// File-select register one.
    Fsr1,
    /// Program counter latch.
    Pclath,
    /// Hardware stack pointer.
    Stkptr,
    /// Word-addressed program counter.
    Pc,
}

impl Pic16Register {
    /// Stable debugger order.
    pub const ALL: [Self; 8] = [
        Self::Wreg,
        Self::Status,
        Self::Bsr,
        Self::Fsr0,
        Self::Fsr1,
        Self::Pclath,
        Self::Stkptr,
        Self::Pc,
    ];
    /// Stable debugger name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Wreg => "wreg",
            Self::Status => "status",
            Self::Bsr => "bsr",
            Self::Fsr0 => "fsr0",
            Self::Fsr1 => "fsr1",
            Self::Pclath => "pclath",
            Self::Stkptr => "stkptr",
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
            Self::Fsr0 | Self::Fsr1 | Self::Pc => 16,
            _ => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct InterruptShadow {
    wreg: u8,
    status: u8,
    bsr: u8,
    pclath: u8,
    fsr0: u16,
    fsr1: u16,
}

/// Deterministic enhanced mid-range PIC16 interpreter.
pub struct Pic16Cpu {
    program: Box<[u16; PROGRAM_WORDS]>,
    pc: u16,
    wreg: u8,
    status: u8,
    bsr: u8,
    fsr: [u16; 2],
    pclath: u8,
    stack: [u16; 16],
    stack_pointer: u8,
    stack_depth: u8,
    shadow: InterruptShadow,
    interrupt_asserted: bool,
    reset_requested: Option<ResetKind>,
    watchdog_clear_requested: bool,
    waiting: bool,
    halted: bool,
}

impl Default for Pic16Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Pic16Cpu {
    /// Constructs a reset core with erased program memory.
    pub fn new() -> Self {
        Self {
            program: Box::new([0x3fff; PROGRAM_WORDS]),
            pc: 0,
            wreg: 0,
            status: STATUS_PD | STATUS_TO,
            bsr: 0,
            fsr: [0; 2],
            pclath: 0,
            stack: [0; 16],
            stack_pointer: 0,
            stack_depth: 0,
            shadow: InterruptShadow::default(),
            interrupt_asserted: false,
            reset_requested: None,
            watchdog_clear_requested: false,
            waiting: false,
            halted: false,
        }
    }

    /// Loads reconstructed 14-bit words at a word address.
    pub fn load_program_words(&mut self, word_address: u16, words: &[u16]) -> Result<(), CpuFault> {
        let start = usize::from(word_address);
        let end = start.checked_add(words.len()).ok_or_else(|| {
            self.fault(CpuFaultKind::Architecture, "program image range overflow")
        })?;
        if end > PROGRAM_WORDS {
            return Err(self.fault(
                CpuFaultKind::Architecture,
                format!("program image ends at word {end:#x}, beyond 16K words"),
            ));
        }
        for (destination, source) in self.program[start..end].iter_mut().zip(words) {
            if source & !PROGRAM_MASK != 0 {
                return Err(CpuFault::new(
                    CpuFaultKind::Architecture,
                    u64::from(word_address),
                    format!("program word {source:#06x} exceeds 14 bits"),
                ));
            }
            *destination = *source;
        }
        Ok(())
    }

    /// Reads one loaded program word for disassembly and execution logging.
    pub fn program_word(&self, word_address: u16) -> Option<u16> {
        self.program.get(usize::from(word_address)).copied()
    }

    /// Reads a named architectural register.
    pub fn register(&self, register: Pic16Register) -> u64 {
        match register {
            Pic16Register::Wreg => u64::from(self.wreg),
            Pic16Register::Status => u64::from(self.status),
            Pic16Register::Bsr => u64::from(self.bsr),
            Pic16Register::Fsr0 => u64::from(self.fsr[0]),
            Pic16Register::Fsr1 => u64::from(self.fsr[1]),
            Pic16Register::Pclath => u64::from(self.pclath),
            Pic16Register::Stkptr => u64::from(self.stack_pointer),
            Pic16Register::Pc => u64::from(self.pc),
        }
    }

    /// Sets a named architectural register for debugger/test use.
    pub fn set_register(&mut self, register: Pic16Register, value: u64) {
        match register {
            Pic16Register::Wreg => self.wreg = value as u8,
            Pic16Register::Status => self.status = value as u8,
            Pic16Register::Bsr => self.bsr = value as u8 & 0x3f,
            Pic16Register::Fsr0 => self.fsr[0] = value as u16,
            Pic16Register::Fsr1 => self.fsr[1] = value as u16,
            Pic16Register::Pclath => self.pclath = value as u8 & 0x7f,
            Pic16Register::Stkptr => self.stack_pointer = value as u8 & 0x0f,
            Pic16Register::Pc => self.pc = value as u16 & PROGRAM_MASK,
        }
    }

    /// Whether the software RESET instruction was executed is represented by PC=0 and reset state.
    pub fn waiting(&self) -> bool {
        self.waiting
    }

    /// Consumes a reset requested by the RESET instruction.
    pub fn take_reset_request(&mut self) -> Option<ResetKind> {
        self.reset_requested.take()
    }

    /// Consumes a watchdog-clear request made by CLRWDT.
    pub fn take_watchdog_clear(&mut self) -> bool {
        std::mem::take(&mut self.watchdog_clear_requested)
    }

    fn fault(&self, kind: CpuFaultKind, message: impl Into<String>) -> CpuFault {
        CpuFault::new(kind, u64::from(self.pc), message)
    }

    fn fetch(&mut self) -> Result<u16, CpuFault> {
        let address = usize::from(self.pc);
        let instruction = *self.program.get(address).ok_or_else(|| {
            self.fault(
                CpuFaultKind::Bus,
                "program counter is outside program memory",
            )
        })?;
        self.pc = self.pc.wrapping_add(1) & PROGRAM_MASK;
        Ok(instruction)
    }

    fn push(&mut self, value: u16) {
        self.stack[usize::from(self.stack_pointer)] = value & PROGRAM_MASK;
        self.stack_pointer = self.stack_pointer.wrapping_add(1) & 0x0f;
        self.stack_depth = self.stack_depth.saturating_add(1).min(16);
    }

    fn pop(&mut self) -> u16 {
        self.stack_pointer = self.stack_pointer.wrapping_sub(1) & 0x0f;
        if self.stack_depth != 0 {
            self.stack_depth -= 1;
        }
        self.stack[usize::from(self.stack_pointer)]
    }

    fn direct_address(&self, file: u8) -> u16 {
        match file {
            0x00..=0x0b | 0x70..=0x7f => u16::from(file),
            _ => u16::from(self.bsr & 0x3f) * 0x80 + u16::from(file),
        }
    }

    fn linear_address(address: u16) -> Option<u16> {
        if (0x2000..0x29f0).contains(&address) {
            let offset = address - 0x2000;
            let bank = offset / 80;
            let within = offset % 80;
            Some(bank * 0x80 + 0x20 + within)
        } else {
            None
        }
    }

    fn read_bus(&self, bus: &mut dyn Bus, address: u16, now: SimTime) -> Result<u8, CpuFault> {
        bus.read(u64::from(address), AccessWidth::Byte, AccessKind::Read, now)
            .map(|value| value as u8)
            .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))
    }

    fn write_bus(
        &self,
        bus: &mut dyn Bus,
        address: u16,
        value: u8,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        bus.write(u64::from(address), AccessWidth::Byte, u64::from(value), now)
            .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))
    }

    fn read_core(&mut self, file: u8, bus: &mut dyn Bus, now: SimTime) -> Result<u8, CpuFault> {
        match file {
            0x00 => self.read_indirect(0, bus, now),
            0x01 => self.read_indirect(1, bus, now),
            0x02 => Ok(self.pc as u8),
            0x03 => Ok(self.status),
            0x04 => Ok(self.fsr[0] as u8),
            0x05 => Ok((self.fsr[0] >> 8) as u8),
            0x06 => Ok(self.fsr[1] as u8),
            0x07 => Ok((self.fsr[1] >> 8) as u8),
            0x08 => Ok(self.bsr),
            0x09 => Ok(self.wreg),
            0x0a => Ok(self.pclath),
            _ => self.read_bus(bus, self.direct_address(file), now),
        }
    }

    fn write_core(
        &mut self,
        file: u8,
        value: u8,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        match file {
            0x00 => self.write_indirect(0, value, bus, now),
            0x01 => self.write_indirect(1, value, bus, now),
            0x02 => {
                self.pc = ((u16::from(self.pclath) << 8) | u16::from(value)) & PROGRAM_MASK;
                Ok(())
            }
            0x03 => {
                self.status = value & 0x1f;
                Ok(())
            }
            0x04 => {
                self.fsr[0] = (self.fsr[0] & 0xff00) | u16::from(value);
                Ok(())
            }
            0x05 => {
                self.fsr[0] = (self.fsr[0] & 0x00ff) | (u16::from(value) << 8);
                Ok(())
            }
            0x06 => {
                self.fsr[1] = (self.fsr[1] & 0xff00) | u16::from(value);
                Ok(())
            }
            0x07 => {
                self.fsr[1] = (self.fsr[1] & 0x00ff) | (u16::from(value) << 8);
                Ok(())
            }
            0x08 => {
                self.bsr = value & 0x3f;
                Ok(())
            }
            0x09 => {
                self.wreg = value;
                Ok(())
            }
            0x0a => {
                self.pclath = value & 0x7f;
                Ok(())
            }
            _ => self.write_bus(bus, self.direct_address(file), value, now),
        }
    }

    fn resolve_indirect(&self, index: usize) -> Option<u16> {
        let address = self.fsr[index];
        if address == 0 || address == 1 {
            None
        } else {
            Self::linear_address(address).or(Some(address & 0x1fff))
        }
    }

    fn read_indirect(
        &mut self,
        index: usize,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<u8, CpuFault> {
        if self.fsr[index] & 0x8000 != 0 {
            let word_address = self.fsr[index] & PROGRAM_MASK;
            return Ok(self.program[usize::from(word_address)] as u8);
        }
        let Some(address) = self.resolve_indirect(index) else {
            return Ok(0);
        };
        if address <= 0x0a {
            self.read_core(address as u8, bus, now)
        } else {
            self.read_bus(bus, address, now)
        }
    }

    fn write_indirect(
        &mut self,
        index: usize,
        value: u8,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        if self.fsr[index] & 0x8000 != 0 {
            // Indirect writes cannot program flash through INDF; NVM registers
            // own that operation on the physical device.
            return Ok(());
        }
        let Some(address) = self.resolve_indirect(index) else {
            return Ok(());
        };
        if address <= 0x0a {
            self.write_core(address as u8, value, bus, now)
        } else {
            self.write_bus(bus, address, value, now)
        }
    }

    fn set_zero(&mut self, value: u8) {
        self.status = (self.status & !STATUS_Z) | if value == 0 { STATUS_Z } else { 0 };
    }

    fn set_add_flags(&mut self, lhs: u8, rhs: u8, carry: u8, result: u8) {
        let sum = u16::from(lhs) + u16::from(rhs) + u16::from(carry);
        let digit = (lhs & 0x0f) + (rhs & 0x0f) + carry;
        self.status &= !(STATUS_C | STATUS_DC | STATUS_Z);
        self.status |= if sum > 0xff { STATUS_C } else { 0 };
        self.status |= if digit > 0x0f { STATUS_DC } else { 0 };
        self.status |= if result == 0 { STATUS_Z } else { 0 };
    }

    fn set_sub_flags(&mut self, lhs: u8, rhs: u8, borrow: u8, result: u8) {
        self.status &= !(STATUS_C | STATUS_DC | STATUS_Z);
        self.status |= if u16::from(lhs) >= u16::from(rhs) + u16::from(borrow) {
            STATUS_C
        } else {
            0
        };
        self.status |= if u16::from(lhs & 0x0f) >= u16::from(rhs & 0x0f) + u16::from(borrow) {
            STATUS_DC
        } else {
            0
        };
        self.status |= if result == 0 { STATUS_Z } else { 0 };
    }

    fn interrupt_enabled(&self, bus: &mut dyn Bus, now: SimTime) -> Result<bool, CpuFault> {
        Ok(self.read_bus(bus, 0x0b, now)? & INTCON_GIE != 0)
    }

    fn enter_interrupt(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<(), CpuFault> {
        self.push(self.pc);
        self.shadow = InterruptShadow {
            wreg: self.wreg,
            status: self.status,
            bsr: self.bsr,
            pclath: self.pclath,
            fsr0: self.fsr[0],
            fsr1: self.fsr[1],
        };
        let intcon = self.read_bus(bus, 0x0b, now)? & !INTCON_GIE;
        self.write_bus(bus, 0x0b, intcon, now)?;
        self.pc = 4;
        self.waiting = false;
        Ok(())
    }

    fn restore_shadow(&mut self) {
        self.wreg = self.shadow.wreg;
        self.status = self.shadow.status;
        self.bsr = self.shadow.bsr;
        self.pclath = self.shadow.pclath;
        self.fsr = [self.shadow.fsr0, self.shadow.fsr1];
    }

    fn reset_state(&mut self) {
        self.pc = 0;
        self.wreg = 0;
        self.status = STATUS_PD | STATUS_TO;
        self.bsr = 0;
        self.fsr = [0; 2];
        self.pclath = 0;
        self.stack_pointer = 0;
        self.stack_depth = 0;
        self.shadow = InterruptShadow::default();
        self.interrupt_asserted = false;
        self.reset_requested = None;
        self.watchdog_clear_requested = false;
        self.waiting = false;
        self.halted = false;
    }
}

impl Cpu for Pic16Cpu {
    fn architecture(&self) -> Architecture {
        Architecture::Pic16Enhanced
    }

    fn reset(&mut self, _kind: ResetKind, _bus: &mut dyn Bus) -> Result<(), CpuFault> {
        self.reset_state();
        Ok(())
    }

    fn step(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<StepOutcome, CpuFault> {
        if self.halted {
            return Ok(StepOutcome {
                elapsed: SimDuration::from_ticks(0),
                reason: StepReason::Halted,
            });
        }
        if self.interrupt_asserted && self.interrupt_enabled(bus, now)? {
            self.enter_interrupt(bus, now)?;
            return Ok(StepOutcome::advanced(SimDuration::from_ticks(2)));
        }
        if self.waiting {
            return Ok(StepOutcome {
                elapsed: SimDuration::from_ticks(1),
                reason: StepReason::WaitForInterrupt,
            });
        }
        let instruction_pc = self.pc;
        let instruction = self.fetch()?;
        self.execute(instruction, instruction_pc, bus, now)
    }

    fn set_interrupt(&mut self, line: u16, asserted: bool) -> Result<(), CpuFault> {
        if line != 0 {
            return Err(self.fault(
                CpuFaultKind::Architecture,
                format!("PIC16 exposes one combined interrupt input, not line {line}"),
            ));
        }
        self.interrupt_asserted = asserted;
        if asserted {
            self.waiting = false;
        }
        Ok(())
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot {
            architecture: Architecture::Pic16Enhanced,
            pc: u64::from(self.pc),
            registers: Pic16Register::ALL
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
