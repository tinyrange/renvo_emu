//! Interpreted MSP430 CPUXv2 core with named architectural registers.

use renvo_core::{
    AccessKind, AccessWidth, Architecture, Bus, Cpu, CpuFault, CpuFaultKind, CpuSnapshot,
    RegisterValue, ResetKind, SimDuration, SimTime, StepOutcome, StepReason,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

mod execution;

const ADDRESS_MASK: u32 = 0x000f_ffff;
const SR_C: u16 = 1 << 0;
const SR_Z: u16 = 1 << 1;
const SR_N: u16 = 1 << 2;
const SR_GIE: u16 = 1 << 3;
const SR_CPUOFF: u16 = 1 << 4;
const SR_V: u16 = 1 << 8;

/// MSP430X access space used by the machine adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Msp430AddressSpace {
    /// Unified 20-bit memory and peripheral space.
    Unified,
    /// Persistent FRAM backing used for reset qualification.
    Fram,
}

/// Named CPUXv2 architectural register.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Msp430Register {
    /// Program counter r0.
    Pc,
    /// Stack pointer r1.
    Sp,
    /// Status/constant-generator register r2.
    Sr,
    /// Constant generator r3.
    Cg,
    /// General register r4.
    R4,
    /// General register r5.
    R5,
    /// General register r6.
    R6,
    /// General register r7.
    R7,
    /// General register r8.
    R8,
    /// General register r9.
    R9,
    /// General register r10.
    R10,
    /// General register r11.
    R11,
    /// General register r12.
    R12,
    /// General register r13.
    R13,
    /// General register r14.
    R14,
    /// General register r15.
    R15,
}

impl Msp430Register {
    /// Stable debugger order.
    pub const ALL: [Self; 16] = [
        Self::Pc,
        Self::Sp,
        Self::Sr,
        Self::Cg,
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
    ];

    /// Stable register name.
    pub const fn name(self) -> &'static str {
        const NAMES: [&str; 16] = [
            "pc", "sp", "sr", "cg", "r4", "r5", "r6", "r7", "r8", "r9", "r10", "r11", "r12", "r13",
            "r14", "r15",
        ];
        NAMES[self.gdb_number()]
    }

    /// GDB register number derived from the named register.
    pub const fn gdb_number(self) -> usize {
        self as usize
    }

    /// CPUXv2 register width exposed to the debugger.
    pub const fn bits(self) -> u8 {
        20
    }
}

#[derive(Clone, Copy)]
enum OperandTarget {
    Register(usize),
    Memory(u32),
}

/// Deterministic interpreted MSP430 CPUXv2 core.
pub struct Msp430Cpu {
    registers: [u32; 16],
    pending_vectors: BTreeSet<u32>,
    waiting: bool,
    halted: bool,
}

impl Default for Msp430Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Msp430Cpu {
    /// Constructs a reset CPUXv2 core.
    pub fn new() -> Self {
        Self {
            registers: [0; 16],
            pending_vectors: BTreeSet::new(),
            waiting: false,
            halted: false,
        }
    }

    /// Reads a named 20-bit architectural register.
    pub fn register(&self, register: Msp430Register) -> u32 {
        self.registers[register.gdb_number()]
    }

    fn fault(&self, kind: CpuFaultKind, message: impl Into<String>) -> CpuFault {
        CpuFault::new(kind, u64::from(self.registers[0]), message)
    }

    fn read(
        &self,
        bus: &mut dyn Bus,
        address: u32,
        byte: bool,
        kind: AccessKind,
        now: SimTime,
    ) -> Result<u16, CpuFault> {
        bus.read(
            u64::from(address & ADDRESS_MASK),
            if byte {
                AccessWidth::Byte
            } else {
                AccessWidth::HalfWord
            },
            kind,
            now,
        )
        .map(|value| value as u16)
        .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))
    }

    fn write(
        &self,
        bus: &mut dyn Bus,
        address: u32,
        byte: bool,
        value: u16,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        bus.write(
            u64::from(address & ADDRESS_MASK),
            if byte {
                AccessWidth::Byte
            } else {
                AccessWidth::HalfWord
            },
            u64::from(value),
            now,
        )
        .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))
    }

    fn fetch(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<u16, CpuFault> {
        let pc = self.registers[0];
        let word = self.read(bus, pc, false, AccessKind::Execute, now)?;
        self.registers[0] = pc.wrapping_add(2) & ADDRESS_MASK;
        Ok(word)
    }

    fn push(&mut self, bus: &mut dyn Bus, value: u16, now: SimTime) -> Result<(), CpuFault> {
        self.registers[1] = self.registers[1].wrapping_sub(2) & ADDRESS_MASK;
        self.write(bus, self.registers[1], false, value, now)
    }

    fn pop(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<u16, CpuFault> {
        let value = self.read(bus, self.registers[1], false, AccessKind::Read, now)?;
        self.registers[1] = self.registers[1].wrapping_add(2) & ADDRESS_MASK;
        Ok(value)
    }

    fn set_register_value(&mut self, index: usize, value: u32, byte: bool) {
        if index == 3 {
            return;
        }
        if byte {
            // MSP430 register-mode byte writes clear bits 19:8.
            self.registers[index] = value & 0xff;
        } else {
            self.registers[index] = value & ADDRESS_MASK;
        }
        if index == 2 {
            self.registers[index] &= 0xffff;
            self.waiting = self.registers[index] as u16 & SR_CPUOFF != 0;
        }
    }

    fn status(&self) -> u16 {
        self.registers[2] as u16
    }

    fn set_status(&mut self, status: u16) {
        self.registers[2] = u32::from(status);
        self.waiting = status & SR_CPUOFF != 0;
    }

    fn push_address(
        &mut self,
        bus: &mut dyn Bus,
        value: u32,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        self.registers[1] = self.registers[1].wrapping_sub(4) & ADDRESS_MASK;
        self.write(bus, self.registers[1], false, value as u16, now)?;
        self.write(
            bus,
            self.registers[1].wrapping_add(2),
            false,
            ((value >> 16) & 0x0f) as u16,
            now,
        )
    }

    fn read_address(
        &self,
        bus: &mut dyn Bus,
        address: u32,
        kind: AccessKind,
        now: SimTime,
    ) -> Result<u32, CpuFault> {
        let low = u32::from(self.read(bus, address, false, kind, now)?);
        let high = u32::from(self.read(
            bus,
            address.wrapping_add(2) & ADDRESS_MASK,
            false,
            kind,
            now,
        )?);
        Ok(low | ((high & 0x0f) << 16))
    }

    fn write_address(
        &self,
        bus: &mut dyn Bus,
        address: u32,
        value: u32,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        self.write(bus, address, false, value as u16, now)?;
        self.write(
            bus,
            address.wrapping_add(2) & ADDRESS_MASK,
            false,
            ((value >> 16) & 0x0f) as u16,
            now,
        )
    }

    fn pop_address(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<u32, CpuFault> {
        let low = u32::from(self.read(bus, self.registers[1], false, AccessKind::Read, now)?);
        let high = u32::from(self.read(
            bus,
            self.registers[1].wrapping_add(2),
            false,
            AccessKind::Read,
            now,
        )?);
        self.registers[1] = self.registers[1].wrapping_add(4) & ADDRESS_MASK;
        Ok(low | ((high & 0x0f) << 16))
    }
}

impl Cpu for Msp430Cpu {
    fn architecture(&self) -> Architecture {
        Architecture::Msp430X
    }

    fn reset(&mut self, _kind: ResetKind, bus: &mut dyn Bus) -> Result<(), CpuFault> {
        self.registers = [0; 16];
        self.pending_vectors.clear();
        self.waiting = false;
        self.halted = false;
        self.registers[0] =
            u32::from(self.read(bus, 0xfffe, false, AccessKind::Read, SimTime::ZERO)?);
        Ok(())
    }

    fn step(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<StepOutcome, CpuFault> {
        if self.halted {
            return Ok(StepOutcome {
                elapsed: SimDuration::TICK,
                reason: StepReason::Halted,
            });
        }
        if self.status() & SR_GIE != 0 {
            if let Some(vector) = self.pending_vectors.pop_last() {
                let pc = self.registers[0];
                let status = self.status();
                self.push(bus, pc as u16, now)?;
                self.push(bus, status | (((pc >> 16) as u16 & 0x0f) << 12), now)?;
                self.set_status(status & !(SR_GIE | 0x00f0));
                self.registers[0] =
                    u32::from(self.read(bus, vector, false, AccessKind::Read, now)?);
                return Ok(StepOutcome::advanced(SimDuration::from_ticks(6)));
            }
        }
        if self.waiting {
            return Ok(StepOutcome {
                elapsed: SimDuration::TICK,
                reason: StepReason::WaitForInterrupt,
            });
        }
        let instruction = self.fetch(bus, now)?;
        let reason = self.execute(instruction, bus, now)?;
        Ok(StepOutcome {
            elapsed: SimDuration::TICK,
            reason,
        })
    }

    fn set_interrupt(&mut self, line: u16, asserted: bool) -> Result<(), CpuFault> {
        let vector = u32::from(line);
        if !(0xff80..=0xfffc).contains(&vector) || vector & 1 != 0 {
            return Err(self.fault(
                CpuFaultKind::Unsupported,
                format!("invalid MSP430 vector address {vector:#x}"),
            ));
        }
        if asserted {
            self.pending_vectors.insert(vector);
            self.waiting = false;
        } else {
            self.pending_vectors.remove(&vector);
        }
        Ok(())
    }

    fn snapshot(&self) -> CpuSnapshot {
        CpuSnapshot {
            architecture: Architecture::Msp430X,
            pc: u64::from(self.registers[0]),
            registers: Msp430Register::ALL
                .iter()
                .copied()
                .map(|register| RegisterValue {
                    name: register.name().to_owned(),
                    value: u64::from(self.register(register)),
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
    use renvo_bus::{AddressSpace, Endianness};

    fn test_bus() -> AddressSpace {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("cpux-test", 0, 1 << 20, true).unwrap();
        bus
    }

    fn load_words(bus: &mut AddressSpace, address: u32, words: &[u16]) {
        let bytes = words
            .iter()
            .copied()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        bus.load(u64::from(address), &bytes).unwrap();
    }

    fn set_register(cpu: &mut Msp430Cpu, register: Msp430Register, value: u32) {
        cpu.set_register_value(register.gdb_number(), value, false);
    }

    #[test]
    fn mapping_is_named_and_dense() {
        for (index, register) in Msp430Register::ALL.iter().copied().enumerate() {
            assert_eq!(register.gdb_number(), index);
            assert!(!register.name().is_empty());
        }
    }

    #[test]
    fn mova_and_extension_word_preserve_twenty_bit_values_above_64k() {
        let mut bus = test_bus();
        let mut cpu = Msp430Cpu::new();
        set_register(&mut cpu, Msp430Register::Pc, 0x1_0020);
        // MOVA #0x12345,r4; MOVA r4,&0x23456; MOVA &0x23456,r5;
        // ADDX.A r4,r5 (extension word + ordinary ADD encoding).
        load_words(
            &mut bus,
            0x1_0020,
            &[
                0x0184, 0x2345, 0x0462, 0x3456, 0x0225, 0x3456, 0x1800, 0x5405,
            ],
        );
        for _ in 0..4 {
            cpu.step(&mut bus, SimTime::ZERO).unwrap();
        }
        assert_eq!(cpu.register(Msp430Register::R4), 0x1_2345);
        assert_eq!(cpu.register(Msp430Register::R5), 0x2_468a);
        assert_eq!(
            cpu.read_address(&mut bus, 0x2_3456, AccessKind::Read, SimTime::ZERO)
                .unwrap(),
            0x1_2345
        );
    }

    #[test]
    fn calla_and_reta_round_trip_a_twenty_bit_program_counter() {
        let mut bus = test_bus();
        let mut cpu = Msp430Cpu::new();
        set_register(&mut cpu, Msp430Register::Pc, 0x1_0100);
        set_register(&mut cpu, Msp430Register::Sp, 0x4000);
        // CALLA #0x12340; at the target, RETA is MOVA @SP+,PC.
        load_words(&mut bus, 0x1_0100, &[0x13b1, 0x2340]);
        load_words(&mut bus, 0x1_2340, &[0x0110]);
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(Msp430Register::Pc), 0x1_2340);
        assert_eq!(cpu.register(Msp430Register::Sp), 0x3ffc);
        assert_eq!(
            cpu.read_address(&mut bus, 0x3ffc, AccessKind::Read, SimTime::ZERO)
                .unwrap(),
            0x1_0104
        );
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(Msp430Register::Pc), 0x1_0104);
        assert_eq!(cpu.register(Msp430Register::Sp), 0x4000);
    }

    #[test]
    fn pushm_a_and_popm_a_use_named_register_ranges() {
        let mut bus = test_bus();
        let mut cpu = Msp430Cpu::new();
        set_register(&mut cpu, Msp430Register::Pc, 0x1_0200);
        set_register(&mut cpu, Msp430Register::Sp, 0x4000);
        set_register(&mut cpu, Msp430Register::R9, 0x1_2345);
        set_register(&mut cpu, Msp430Register::R10, 0xa_bcde);
        load_words(&mut bus, 0x1_0200, &[0x141a, 0x1619]);
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(Msp430Register::Sp), 0x3ff8);
        set_register(&mut cpu, Msp430Register::R9, 0);
        set_register(&mut cpu, Msp430Register::R10, 0);
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(Msp430Register::R9), 0x1_2345);
        assert_eq!(cpu.register(Msp430Register::R10), 0xa_bcde);
        assert_eq!(cpu.register(Msp430Register::Sp), 0x4000);
    }

    #[test]
    fn bit_updates_flags_without_writing_its_destination() {
        let mut bus = test_bus();
        let mut cpu = Msp430Cpu::new();
        set_register(&mut cpu, Msp430Register::Pc, 0x1000);
        // BIT #2,&0x0200. The absolute destination must remain unchanged.
        load_words(&mut bus, 0x1000, &[0xb3a2, 0x0200]);
        load_words(&mut bus, 0x0200, &[0x0002]);
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(
            bus.read(
                0x0200,
                AccessWidth::HalfWord,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            2
        );
        assert_ne!(cpu.register(Msp430Register::Sr) & u32::from(SR_C), 0);
        assert_eq!(cpu.register(Msp430Register::Sr) & u32::from(SR_Z), 0);
    }

    #[test]
    fn dadd_uses_packed_decimal_digits() {
        let mut bus = test_bus();
        let mut cpu = Msp430Cpu::new();
        set_register(&mut cpu, Msp430Register::Pc, 0x1000);
        set_register(&mut cpu, Msp430Register::R4, 0x0099);
        // DADD #1,r4.
        load_words(&mut bus, 0x1000, &[0xa314]);
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(Msp430Register::R4), 0x0100);
    }

    #[test]
    fn interrupt_and_reti_restore_a_twenty_bit_program_counter() {
        let mut bus = test_bus();
        let mut cpu = Msp430Cpu::new();
        set_register(&mut cpu, Msp430Register::Pc, 0x5_4320);
        set_register(&mut cpu, Msp430Register::Sp, 0x4000);
        set_register(&mut cpu, Msp430Register::Sr, u32::from(SR_GIE | SR_C));
        load_words(&mut bus, 0x2000, &[0x1300]);
        load_words(&mut bus, 0xfffc, &[0x2000]);
        cpu.set_interrupt(0xfffc, true).unwrap();
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(Msp430Register::Pc), 0x2000);
        assert_eq!(cpu.register(Msp430Register::Sp), 0x3ffc);
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(Msp430Register::Pc), 0x5_4320);
        assert_eq!(cpu.register(Msp430Register::Sp), 0x4000);
        assert_eq!(cpu.register(Msp430Register::Sr), u32::from(SR_GIE | SR_C));
    }
}
