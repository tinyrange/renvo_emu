//! AVR8 architectural identities and interpreted-core foundation.

use remu_core::{
    AccessKind, AccessWidth, Architecture, Bus, Cpu, CpuFault, CpuFaultKind, CpuSnapshot,
    RegisterValue, ResetKind, SimDuration, SimTime, StepOutcome, StepReason,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

mod execution;

const SREG_C: u8 = 1 << 0;
const SREG_Z: u8 = 1 << 1;
const SREG_N: u8 = 1 << 2;
const SREG_V: u8 = 1 << 3;
const SREG_S: u8 = 1 << 4;
const SREG_H: u8 = 1 << 5;
const SREG_T: u8 = 1 << 6;
const SREG_I: u8 = 1 << 7;

/// AVR Harvard address space selected for an architectural access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AvrAddressSpace {
    /// Word-addressed instruction storage.
    Program,
    /// Register file, I/O window and SRAM data space.
    Data,
    /// Direct I/O instruction window.
    Io,
    /// Non-volatile EEPROM storage.
    Eeprom,
}

/// Named ATmega AVR debugger register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AvrRegister {
    /// General register r0.
    R0,
    R1,
    R2,
    R3,
    R4,
    R5,
    R6,
    R7,
    /// General register r8.
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
    /// General register r16.
    R16,
    R17,
    R18,
    R19,
    R20,
    R21,
    R22,
    R23,
    /// General register r24.
    R24,
    R25,
    R26,
    R27,
    R28,
    R29,
    R30,
    R31,
    /// Status register.
    Sreg,
    /// Stack pointer.
    Sp,
    /// Word-addressed program counter.
    Pc,
}

impl AvrRegister {
    /// Deterministic debugger order, matching the AVR GDB register layout.
    pub const ALL: [Self; 35] = [
        Self::R0,
        Self::R1,
        Self::R2,
        Self::R3,
        Self::R4,
        Self::R5,
        Self::R6,
        Self::R7,
        Self::R8,
        Self::R9,
        Self::R10,
        Self::R11,
        Self::R12,
        Self::R13,
        Self::R14,
        Self::R15,
        Self::R16,
        Self::R17,
        Self::R18,
        Self::R19,
        Self::R20,
        Self::R21,
        Self::R22,
        Self::R23,
        Self::R24,
        Self::R25,
        Self::R26,
        Self::R27,
        Self::R28,
        Self::R29,
        Self::R30,
        Self::R31,
        Self::Sreg,
        Self::Sp,
        Self::Pc,
    ];

    /// Stable debugger name.
    pub const fn name(self) -> &'static str {
        const NAMES: [&str; 35] = [
            "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12", "r13",
            "r14", "r15", "r16", "r17", "r18", "r19", "r20", "r21", "r22", "r23", "r24", "r25",
            "r26", "r27", "r28", "r29", "r30", "r31", "sreg", "sp", "pc",
        ];
        NAMES[self.gdb_number()]
    }

    /// Architecture-specific GDB register number derived from the named variant.
    pub const fn gdb_number(self) -> usize {
        match self {
            Self::R0 => 0,
            Self::R1 => 1,
            Self::R2 => 2,
            Self::R3 => 3,
            Self::R4 => 4,
            Self::R5 => 5,
            Self::R6 => 6,
            Self::R7 => 7,
            Self::R8 => 8,
            Self::R9 => 9,
            Self::R10 => 10,
            Self::R11 => 11,
            Self::R12 => 12,
            Self::R13 => 13,
            Self::R14 => 14,
            Self::R15 => 15,
            Self::R16 => 16,
            Self::R17 => 17,
            Self::R18 => 18,
            Self::R19 => 19,
            Self::R20 => 20,
            Self::R21 => 21,
            Self::R22 => 22,
            Self::R23 => 23,
            Self::R24 => 24,
            Self::R25 => 25,
            Self::R26 => 26,
            Self::R27 => 27,
            Self::R28 => 28,
            Self::R29 => 29,
            Self::R30 => 30,
            Self::R31 => 31,
            Self::Sreg => 32,
            Self::Sp => 33,
            Self::Pc => 34,
        }
    }

    /// Register width in bits.
    pub const fn bits(self) -> u8 {
        match self {
            Self::Sp | Self::Pc => 16,
            _ => 8,
        }
    }
}

/// Interpreted enhanced AVR8 core with an internal word-addressed program store.
pub struct AvrCpu {
    registers: [u8; 32],
    sreg: u8,
    sp: u16,
    pc: u16,
    program: Vec<u16>,
    interrupts: BTreeSet<u16>,
    last_interrupt_line: Option<u16>,
    waiting: bool,
    sleep_enabled: bool,
    halted: bool,
}

impl Default for AvrCpu {
    fn default() -> Self {
        Self::new()
    }
}

impl AvrCpu {
    /// Constructs a reset enhanced AVR core for an ATmega-class device.
    pub fn new() -> Self {
        Self {
            registers: [0; 32],
            sreg: 0,
            sp: 0x08ff,
            pc: 0,
            program: Vec::new(),
            interrupts: BTreeSet::new(),
            last_interrupt_line: None,
            waiting: false,
            // Preserve the standalone core's historical SLEEP behavior. A
            // machine model overrides this from the device's SMCR register.
            sleep_enabled: true,
            halted: false,
        }
    }

    /// Controls whether the next SLEEP instruction enters the waiting state.
    pub fn set_sleep_enabled(&mut self, enabled: bool) {
        self.sleep_enabled = enabled;
    }

    /// Loads byte-oriented flash contents into the word-addressed program store.
    pub fn load_program(&mut self, bytes: &[u8]) -> Result<(), CpuFault> {
        if bytes.len() > 32 * 1024 {
            return Err(self.fault(CpuFaultKind::Architecture, "program exceeds 32 KiB flash"));
        }
        self.program = bytes
            .chunks(2)
            .map(|pair| u16::from(pair[0]) | (u16::from(*pair.get(1).unwrap_or(&0xff)) << 8))
            .collect();
        Ok(())
    }

    /// Current named architectural register value.
    pub fn register(&self, register: AvrRegister) -> u64 {
        match register {
            AvrRegister::Sreg => u64::from(self.sreg),
            AvrRegister::Sp => u64::from(self.sp),
            AvrRegister::Pc => u64::from(self.pc),
            _ => u64::from(self.registers[register.gdb_number()]),
        }
    }

    /// Returns the interrupt line consumed by the most recent CPU step.
    ///
    /// A machine model can use this acknowledgement point to apply
    /// architecture-specific peripheral side effects, such as clearing an
    /// interrupt flag that hardware clears while vectoring.
    pub fn last_interrupt_line(&self) -> Option<u16> {
        self.last_interrupt_line
    }

    fn fault(&self, kind: CpuFaultKind, message: impl Into<String>) -> CpuFault {
        CpuFault::new(kind, u64::from(self.pc) * 2, message)
    }

    fn fetch(&self, pc: u16) -> Result<u16, CpuFault> {
        self.program.get(usize::from(pc)).copied().ok_or_else(|| {
            self.fault(
                CpuFaultKind::Bus,
                format!("program fetch outside loaded flash at word {pc:#06x}"),
            )
        })
    }

    fn program_read_byte(&self, address: u16) -> Result<u8, CpuFault> {
        let word = self.fetch(address >> 1)?;
        Ok(if address & 1 == 0 {
            word as u8
        } else {
            (word >> 8) as u8
        })
    }

    fn data_read(&mut self, bus: &mut dyn Bus, address: u16, now: SimTime) -> Result<u8, CpuFault> {
        match address {
            0x00..=0x1f => Ok(self.registers[usize::from(address)]),
            0x5d => Ok(self.sp as u8),
            0x5e => Ok((self.sp >> 8) as u8),
            0x5f => Ok(self.sreg),
            _ => bus
                .read(u64::from(address), AccessWidth::Byte, AccessKind::Read, now)
                .map(|value| value as u8)
                .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string())),
        }
    }

    fn data_write(
        &mut self,
        bus: &mut dyn Bus,
        address: u16,
        value: u8,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        match address {
            0x00..=0x1f => self.registers[usize::from(address)] = value,
            0x5d => self.sp = (self.sp & 0xff00) | u16::from(value),
            0x5e => self.sp = (self.sp & 0x00ff) | (u16::from(value) << 8),
            0x5f => self.sreg = value,
            _ => bus
                .write(u64::from(address), AccessWidth::Byte, u64::from(value), now)
                .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))?,
        }
        Ok(())
    }

    fn push(&mut self, bus: &mut dyn Bus, value: u8, now: SimTime) -> Result<(), CpuFault> {
        self.data_write(bus, self.sp, value, now)?;
        self.sp = self.sp.wrapping_sub(1);
        Ok(())
    }

    fn pop(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<u8, CpuFault> {
        self.sp = self.sp.wrapping_add(1);
        self.data_read(bus, self.sp, now)
    }

    fn push_pc(&mut self, bus: &mut dyn Bus, pc: u16, now: SimTime) -> Result<(), CpuFault> {
        self.push(bus, pc as u8, now)?;
        self.push(bus, (pc >> 8) as u8, now)
    }

    fn pop_pc(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<u16, CpuFault> {
        let high = self.pop(bus, now)?;
        let low = self.pop(bus, now)?;
        Ok(u16::from(low) | (u16::from(high) << 8))
    }
}

impl Cpu for AvrCpu {
    fn architecture(&self) -> Architecture {
        Architecture::Avr8
    }

    fn reset(&mut self, _kind: ResetKind, _bus: &mut dyn Bus) -> Result<(), CpuFault> {
        self.registers = [0; 32];
        self.sreg = 0;
        self.sp = 0x08ff;
        self.pc = 0;
        self.interrupts.clear();
        self.last_interrupt_line = None;
        self.waiting = false;
        self.sleep_enabled = true;
        self.halted = false;
        Ok(())
    }

    fn step(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<StepOutcome, CpuFault> {
        self.last_interrupt_line = None;
        if self.halted {
            return Ok(StepOutcome {
                elapsed: SimDuration::TICK,
                reason: StepReason::Halted,
            });
        }
        if self.sreg & SREG_I != 0 {
            if let Some(line) = self.interrupts.pop_first() {
                self.push_pc(bus, self.pc, now)?;
                self.sreg &= !SREG_I;
                self.pc = line.saturating_add(1).saturating_mul(2);
                self.last_interrupt_line = Some(line);
                self.waiting = false;
                return Ok(StepOutcome::advanced(SimDuration::from_ticks(4)));
            }
        }
        if self.waiting {
            return Ok(StepOutcome {
                elapsed: SimDuration::TICK,
                reason: StepReason::WaitForInterrupt,
            });
        }
        let instruction = self.fetch(self.pc)?;
        self.pc = self.pc.wrapping_add(1);
        let reason = self.execute(instruction, bus, now)?;
        Ok(StepOutcome {
            elapsed: SimDuration::TICK,
            reason,
        })
    }

    fn set_interrupt(&mut self, line: u16, asserted: bool) -> Result<(), CpuFault> {
        if line >= 128 {
            return Err(self.fault(
                CpuFaultKind::Unsupported,
                "AVR interrupt vector exceeds the supported table",
            ));
        }
        if asserted {
            self.interrupts.insert(line);
            self.waiting = false;
        } else {
            self.interrupts.remove(&line);
        }
        Ok(())
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot {
            architecture: Architecture::Avr8,
            pc: u64::from(self.pc) * 2,
            registers: AvrRegister::ALL
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
mod tests {
    use super::*;
    #[test]
    fn named_registers_define_the_complete_gdb_order() {
        for (number, register) in AvrRegister::ALL.iter().copied().enumerate() {
            assert_eq!(register.gdb_number(), number);
            assert!(!register.name().is_empty());
        }
    }

    #[test]
    fn extended_atmega_vector_table_accepts_timer3_and_timer4_lines() {
        let mut cpu = AvrCpu::new();
        assert!(cpu.set_interrupt(32, true).is_ok());
        assert!(cpu.set_interrupt(43, true).is_ok());
        assert!(cpu.set_interrupt(128, true).is_err());
    }
}
