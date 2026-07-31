//! Profile-driven interpreted 32-bit RISC-V CPU.
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::manual_checked_ops,
    clippy::struct_excessive_bools,
    clippy::too_many_lines
)]

use renvo_core::{
    AccessKind, AccessWidth, Architecture, Bus, Cpu, CpuFault, CpuFaultKind, CpuSnapshot,
    RegisterValue, ResetKind, SimDuration, SimTime, StepOutcome, StepReason,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const CSR_MSTATUS: u16 = 0x300;
const CSR_MISA: u16 = 0x301;
const CSR_MEDELEG: u16 = 0x302;
const CSR_MIDELEG: u16 = 0x303;
const CSR_MIE: u16 = 0x304;
const CSR_MTVEC: u16 = 0x305;
const CSR_MCOUNTEREN: u16 = 0x306;
const CSR_MSCRATCH: u16 = 0x340;
const CSR_MEPC: u16 = 0x341;
const CSR_MCAUSE: u16 = 0x342;
const CSR_MTVAL: u16 = 0x343;
const CSR_MIP: u16 = 0x344;
const CSR_PMPCFG0: u16 = 0x3a0;
const CSR_PMPCFG3: u16 = 0x3a3;
const CSR_PMPADDR0: u16 = 0x3b0;
const CSR_PMPADDR15: u16 = 0x3bf;
const CSR_ESP_PCER_MACHINE: u16 = 0x7e0;
const CSR_ESP_PCMR_MACHINE: u16 = 0x7e1;
const CSR_ESP_PCCR_MACHINE: u16 = 0x7e2;
const CSR_MCYCLE: u16 = 0xb00;
const CSR_MINSTRET: u16 = 0xb02;
const CSR_MCYCLEH: u16 = 0xb80;
const CSR_MINSTRETH: u16 = 0xb82;
const CSR_PMACFG0: u16 = 0xbc0;
const CSR_PMACFG15: u16 = 0xbcf;
const CSR_PMAADDR0: u16 = 0xbd0;
const CSR_PMAADDR15: u16 = 0xbdf;
const CSR_MEIEA: u16 = 0xbe0;
const CSR_MEIPA: u16 = 0xbe1;
const CSR_MEIFA: u16 = 0xbe2;
const CSR_MEIPRA: u16 = 0xbe3;
const CSR_MEINEXT: u16 = 0xbe4;
const CSR_MEICONTEXT: u16 = 0xbe5;
const MSTATUS_MIE: u32 = 1 << 3;
const MSTATUS_MPIE: u32 = 1 << 7;
const HAZARD3_IRQ_WINDOWS: usize = 32;

/// Interrupt and trap behaviour selected by a chip profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterruptModel {
    /// Standard machine-mode CSRs and direct/vectored `mtvec`.
    Machine,
    /// WCH QingKe/PFIC-facing core profile.
    QingKe,
    /// Raspberry Pi Hazard3 machine-mode profile.
    Hazard3,
}

/// Named RISC-V integer register using the standard ABI names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum RiscVRegister {
    /// Hard-wired zero register x0.
    Zero = 0,
    /// Return-address register x1.
    Ra = 1,
    /// Stack pointer x2.
    Sp = 2,
    /// Global pointer x3.
    Gp = 3,
    /// Thread pointer x4.
    Tp = 4,
    /// Temporary register x5.
    T0 = 5,
    /// Temporary register x6.
    T1 = 6,
    /// Temporary register x7.
    T2 = 7,
    /// Saved/frame-pointer register x8.
    S0 = 8,
    /// Saved register x9.
    S1 = 9,
    /// Argument/return register x10.
    A0 = 10,
    /// Argument/return register x11.
    A1 = 11,
    /// Argument register x12.
    A2 = 12,
    /// Argument register x13.
    A3 = 13,
    /// Argument register x14.
    A4 = 14,
    /// Argument register x15.
    A5 = 15,
    /// Argument register x16.
    A6 = 16,
    /// Argument register x17.
    A7 = 17,
    /// Saved register x18.
    S2 = 18,
    /// Saved register x19.
    S3 = 19,
    /// Saved register x20.
    S4 = 20,
    /// Saved register x21.
    S5 = 21,
    /// Saved register x22.
    S6 = 22,
    /// Saved register x23.
    S7 = 23,
    /// Saved register x24.
    S8 = 24,
    /// Saved register x25.
    S9 = 25,
    /// Saved register x26.
    S10 = 26,
    /// Saved register x27.
    S11 = 27,
    /// Temporary register x28.
    T3 = 28,
    /// Temporary register x29.
    T4 = 29,
    /// Temporary register x30.
    T5 = 30,
    /// Temporary register x31.
    T6 = 31,
}

impl RiscVRegister {
    const fn index(self) -> u8 {
        self as u8
    }

    /// Returns one of the eight ABI argument registers.
    pub const fn argument(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::A0),
            1 => Some(Self::A1),
            2 => Some(Self::A2),
            3 => Some(Self::A3),
            4 => Some(Self::A4),
            5 => Some(Self::A5),
            6 => Some(Self::A6),
            7 => Some(Self::A7),
            _ => None,
        }
    }
}

/// Selectable ISA and reset configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiscVProfile {
    /// Stable profile name.
    pub name: String,
    /// Architectural integer register count, 16 for RV32E or 32 for RV32I.
    pub registers: u8,
    /// Integer multiply/divide extension.
    pub extension_m: bool,
    /// Atomic extension.
    pub extension_a: bool,
    /// Compressed extension.
    pub extension_c: bool,
    /// Compiler-facing bit manipulation subset.
    pub extension_b: bool,
    /// Zcmp compressed push/pop register-list instructions.
    pub extension_zcmp: bool,
    /// CSR instructions.
    pub extension_zicsr: bool,
    /// ESP32-C6 physical-memory-attribute and protection CSRs.
    pub esp32c6_memory_protection_csrs: bool,
    /// Initial reset vector.
    pub reset_vector: u32,
    /// Trap/interrupt integration profile.
    pub interrupt_model: InterruptModel,
    /// Treat EBREAK as a deterministic machine halt.
    pub ebreak_halts: bool,
}

impl RiscVProfile {
    /// WCH CH32V003 `QingKe` V2A compiler profile.
    pub fn ch32v003() -> Self {
        Self {
            name: "wch-ch32v003-qingke-v2a".to_owned(),
            registers: 16,
            extension_m: false,
            extension_a: false,
            extension_c: true,
            extension_b: false,
            extension_zcmp: false,
            extension_zicsr: true,
            esp32c6_memory_protection_csrs: false,
            reset_vector: 0,
            interrupt_model: InterruptModel::QingKe,
            ebreak_halts: true,
        }
    }

    /// WCH CH32V006 `QingKe` V2C compiler profile.
    pub fn ch32v006() -> Self {
        Self {
            name: "wch-ch32v006-qingke-v2c".to_owned(),
            ..Self::ch32v003()
        }
    }

    /// ESP32-C6 high-performance RV32IMAC core profile.
    pub fn esp32c6() -> Self {
        Self {
            name: "espressif-esp32c6-hp".to_owned(),
            registers: 32,
            extension_m: true,
            extension_a: true,
            extension_c: true,
            extension_b: false,
            extension_zcmp: false,
            extension_zicsr: true,
            esp32c6_memory_protection_csrs: true,
            reset_vector: 0x4000_0000,
            interrupt_model: InterruptModel::Machine,
            ebreak_halts: true,
        }
    }

    /// RP2350 Hazard3 compiler profile.
    pub fn rp2350_hazard3() -> Self {
        Self {
            name: "raspberrypi-rp2350-hazard3".to_owned(),
            registers: 32,
            extension_m: true,
            extension_a: true,
            extension_c: true,
            extension_b: true,
            extension_zcmp: true,
            extension_zicsr: true,
            esp32c6_memory_protection_csrs: false,
            reset_vector: 0,
            interrupt_model: InterruptModel::Hazard3,
            ebreak_halts: true,
        }
    }

    fn validate(&self) -> Result<(), CpuFault> {
        if self.registers != 16 && self.registers != 32 {
            return Err(CpuFault::new(
                CpuFaultKind::Architecture,
                self.reset_vector.into(),
                "RISC-V profile must expose 16 or 32 integer registers",
            ));
        }
        Ok(())
    }
}

/// Interpreted RV32 CPU state.
pub struct RiscVCpu {
    profile: RiscVProfile,
    registers: [u32; 32],
    pc: u32,
    csrs: [u32; 4096],
    cycle: u64,
    instret: u64,
    waiting: bool,
    halted: bool,
    asserted_interrupts: BTreeSet<u16>,
    hazard3_external_interrupts: BTreeSet<u16>,
    hazard3_external_enabled: [u16; HAZARD3_IRQ_WINDOWS],
    hazard3_external_forced: [u16; HAZARD3_IRQ_WINDOWS],
    hazard3_external_priorities: [u16; HAZARD3_IRQ_WINDOWS],
    hazard3_external_active: bool,
    reservation: Option<u32>,
}

impl RiscVCpu {
    /// Constructs a reset CPU.
    pub fn new(profile: RiscVProfile) -> Result<Self, CpuFault> {
        profile.validate()?;
        let mut cpu = Self {
            pc: profile.reset_vector,
            profile,
            registers: [0; 32],
            csrs: [0; 4096],
            cycle: 0,
            instret: 0,
            waiting: false,
            halted: false,
            asserted_interrupts: BTreeSet::new(),
            hazard3_external_interrupts: BTreeSet::new(),
            hazard3_external_enabled: [0; HAZARD3_IRQ_WINDOWS],
            hazard3_external_forced: [0; HAZARD3_IRQ_WINDOWS],
            hazard3_external_priorities: [0; HAZARD3_IRQ_WINDOWS],
            hazard3_external_active: false,
            reservation: None,
        };
        cpu.initialize_csrs();
        Ok(cpu)
    }

    /// Selected profile.
    pub const fn profile(&self) -> &RiscVProfile {
        &self.profile
    }

    /// Current 32-bit program counter.
    pub const fn pc(&self) -> u32 {
        self.pc
    }

    /// Sets the direct-load entry point.
    pub fn set_pc(&mut self, pc: u32) -> Result<(), CpuFault> {
        let alignment = if self.profile.extension_c { 2 } else { 4 };
        if pc % alignment != 0 {
            return Err(CpuFault::new(
                CpuFaultKind::Architecture,
                pc.into(),
                "misaligned RISC-V entry point",
            ));
        }
        self.pc = pc;
        Ok(())
    }

    /// Reads an integer register.
    pub fn register(&self, register: RiscVRegister) -> Result<u32, CpuFault> {
        let index = register.index();
        self.check_register(index)?;
        Ok(self.registers[usize::from(index)])
    }

    /// Writes an integer register. Writes to x0 are discarded.
    pub fn set_register(&mut self, register: RiscVRegister, value: u32) -> Result<(), CpuFault> {
        let index = register.index();
        self.check_register(index)?;
        self.write_register(index, value);
        Ok(())
    }

    /// Establishes the machine trap-vector value used by a boot or multicore
    /// handoff.
    pub fn set_trap_vector(&mut self, address: u32) -> Result<(), CpuFault> {
        self.write_csr(CSR_MTVEC, address)
    }

    /// Reports whether the hart has explicitly entered `WFI`.
    pub const fn waiting_for_interrupt(&self) -> bool {
        self.waiting
    }

    /// Enables or disables one machine interrupt cause in `mie`.
    pub fn set_machine_interrupt_enabled(
        &mut self,
        line: u16,
        enabled: bool,
    ) -> Result<(), CpuFault> {
        if line >= 32 {
            return Err(CpuFault::new(
                CpuFaultKind::Unsupported,
                self.pc.into(),
                format!("RISC-V interrupt line {line} is outside the modeled 0..31 range"),
            ));
        }
        if enabled {
            self.csrs[usize::from(CSR_MIE)] |= 1_u32 << line;
        } else {
            self.csrs[usize::from(CSR_MIE)] &= !(1_u32 << line);
        }
        Ok(())
    }

    /// Asserts or clears one Hazard3 external interrupt-controller input.
    pub fn set_hazard3_external_interrupt(
        &mut self,
        line: u16,
        asserted: bool,
    ) -> Result<(), CpuFault> {
        if self.profile.interrupt_model != InterruptModel::Hazard3 || line >= 512 {
            return Err(CpuFault::new(
                CpuFaultKind::Unsupported,
                self.pc.into(),
                format!("Hazard3 external interrupt line {line} is unavailable"),
            ));
        }
        let changed = if asserted {
            self.hazard3_external_interrupts.insert(line)
        } else {
            self.hazard3_external_interrupts.remove(&line)
        };
        if changed {
            self.refresh_hazard3_machine_external();
        }
        Ok(())
    }

    fn hazard3_irq_is_enabled(&self, line: u16) -> bool {
        let window = usize::from(line / 16);
        let bit = line % 16;
        self.hazard3_external_enabled
            .get(window)
            .is_some_and(|bits| bits & (1 << bit) != 0)
    }

    fn hazard3_irq_is_forced(&self, line: u16) -> bool {
        let window = usize::from(line / 16);
        let bit = line % 16;
        self.hazard3_external_forced
            .get(window)
            .is_some_and(|bits| bits & (1 << bit) != 0)
    }

    fn hazard3_irq_priority(&self, line: u16) -> u8 {
        let window = usize::from(line / 4);
        let shift = u32::from((line % 4) * 4);
        self.hazard3_external_priorities
            .get(window)
            .map_or(0, |priorities| ((priorities >> shift) & 0xf) as u8)
    }

    fn hazard3_irq_is_pending(&self, line: u16) -> bool {
        self.hazard3_external_interrupts.contains(&line) || self.hazard3_irq_is_forced(line)
    }

    fn hazard3_preemption_threshold(&self) -> u8 {
        ((self.csrs[usize::from(CSR_MEICONTEXT)] >> 16) & 0x1f) as u8
    }

    fn hazard3_enabled_pending(&self) -> impl Iterator<Item = u16> + '_ {
        (0..512_u16).filter(|line| {
            self.hazard3_irq_is_pending(*line)
                && self.hazard3_irq_is_enabled(*line)
                && self.hazard3_irq_priority(*line) >= self.hazard3_preemption_threshold()
        })
    }

    fn hazard3_next_external(&self) -> Option<u16> {
        self.hazard3_enabled_pending().max_by(|left, right| {
            self.hazard3_irq_priority(*left)
                .cmp(&self.hazard3_irq_priority(*right))
                .then_with(|| right.cmp(left))
        })
    }

    fn refresh_hazard3_machine_external(&mut self) {
        let machine_external = self.hazard3_next_external().is_some();
        if machine_external {
            self.asserted_interrupts.insert(11);
            self.csrs[usize::from(CSR_MIP)] |= 1 << 11;
        } else {
            self.asserted_interrupts.remove(&11);
            self.csrs[usize::from(CSR_MIP)] &= !(1 << 11);
        }
    }

    fn hazard3_pending_window(&self, index: usize) -> u16 {
        let base = (index * 16) as u16;
        (0..16_u16).fold(0_u16, |bits, offset| {
            if self.hazard3_irq_is_pending(base + offset) {
                bits | (1 << offset)
            } else {
                bits
            }
        })
    }

    fn hazard3_irqarray_read(&self, address: u16, index: usize) -> Result<u32, CpuFault> {
        let window = match address {
            CSR_MEIEA => self.hazard3_external_enabled[index],
            CSR_MEIPA => self.hazard3_pending_window(index),
            CSR_MEIFA => self.hazard3_external_forced[index],
            CSR_MEIPRA => self.hazard3_external_priorities[index],
            _ => {
                return Err(CpuFault::new(
                    CpuFaultKind::Unsupported,
                    self.pc.into(),
                    format!("CSR {address:#05x} is not a Hazard3 IRQ array"),
                ));
            }
        };
        Ok(u32::from(window) << 16)
    }

    fn hazard3_irqarray_access(
        &mut self,
        address: u16,
        source: u32,
        funct3: u32,
    ) -> Result<u32, CpuFault> {
        let index = (source & 0x1f) as usize;
        let old = self.hazard3_irqarray_read(address, index)?;
        let mask = (source >> 16) as u16;
        let target = match address {
            CSR_MEIEA => Some(&mut self.hazard3_external_enabled[index]),
            CSR_MEIPA => None,
            CSR_MEIFA => Some(&mut self.hazard3_external_forced[index]),
            CSR_MEIPRA => Some(&mut self.hazard3_external_priorities[index]),
            _ => unreachable!("validated by hazard3_irqarray_read"),
        };
        if let Some(target) = target {
            match funct3 {
                1 | 5 => *target = mask,
                2 | 6 => *target |= mask,
                3 | 7 => *target &= !mask,
                _ => return self.illegal(source),
            }
            self.refresh_hazard3_machine_external();
        }
        Ok(old)
    }

    /// Reads a modeled CSR.
    pub fn csr(&self, address: u16) -> Result<u32, CpuFault> {
        self.read_csr(address)
    }

    fn initialize_csrs(&mut self) {
        let mut misa = 1_u32 << 30; // MXL=1, RV32
        misa |= if self.profile.registers == 16 {
            1 << (b'E' - b'A')
        } else {
            1 << (b'I' - b'A')
        };
        if self.profile.extension_m {
            misa |= 1 << (b'M' - b'A');
        }
        if self.profile.extension_a {
            misa |= 1;
        }
        if self.profile.extension_c {
            misa |= 1 << (b'C' - b'A');
        }
        self.csrs[usize::from(CSR_MISA)] = misa;
        self.csrs[usize::from(CSR_MTVEC)] = self.profile.reset_vector;
        if self.profile.interrupt_model == InterruptModel::Hazard3 {
            self.csrs[usize::from(CSR_MEICONTEXT)] = 0x0000_8000;
        }
    }

    fn check_register(&self, index: u8) -> Result<(), CpuFault> {
        if index >= self.profile.registers {
            return Err(CpuFault::new(
                CpuFaultKind::IllegalInstruction,
                self.pc.into(),
                format!(
                    "register x{index} is not available in {}",
                    self.profile.name
                ),
            ));
        }
        Ok(())
    }

    fn write_register(&mut self, index: u8, value: u32) {
        if index != 0 {
            self.registers[usize::from(index)] = value;
        }
        self.registers[0] = 0;
    }

    fn read_register(&self, index: u8) -> Result<u32, CpuFault> {
        self.check_register(index)?;
        Ok(self.registers[usize::from(index)])
    }

    fn fetch16(&mut self, bus: &mut dyn Bus, address: u32, now: SimTime) -> Result<u16, CpuFault> {
        bus.read(
            u64::from(address),
            AccessWidth::HalfWord,
            AccessKind::Execute,
            now,
        )
        .map(|value| value as u16)
        .map_err(|fault| {
            CpuFault::new(
                CpuFaultKind::Bus,
                self.pc.into(),
                format!("instruction fetch failed: {fault}"),
            )
        })
    }

    fn load(
        &mut self,
        bus: &mut dyn Bus,
        address: u32,
        width: AccessWidth,
        now: SimTime,
    ) -> Result<u32, CpuFault> {
        bus.read(u64::from(address), width, AccessKind::Read, now)
            .map(|value| value as u32)
            .map_err(|fault| {
                CpuFault::new(
                    CpuFaultKind::Bus,
                    self.pc.into(),
                    format!("load failed: {fault}"),
                )
            })
    }

    fn store(
        &mut self,
        bus: &mut dyn Bus,
        address: u32,
        width: AccessWidth,
        value: u32,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        self.reservation = None;
        bus.write(u64::from(address), width, u64::from(value), now)
            .map_err(|fault| {
                CpuFault::new(
                    CpuFaultKind::Bus,
                    self.pc.into(),
                    format!("store failed: {fault}"),
                )
            })
    }

    fn read_csr(&self, address: u16) -> Result<u32, CpuFault> {
        let value = match address {
            0xc00 | CSR_MCYCLE => self.cycle as u32,
            0xc80 | CSR_MCYCLEH => (self.cycle >> 32) as u32,
            0xc02 | CSR_MINSTRET => self.instret as u32,
            0xc82 | CSR_MINSTRETH => (self.instret >> 32) as u32,
            0xf14 => 0, // mhartid: the functional machine currently executes hart 0.
            CSR_MEIEA | CSR_MEIPA | CSR_MEIFA | CSR_MEIPRA => {
                self.hazard3_irqarray_read(address, 0)?
            }
            CSR_MEINEXT => self
                .hazard3_next_external()
                .map_or(0x8000_0000, |line| u32::from(line) * 4),
            CSR_MEICONTEXT => self.csrs[usize::from(address)],
            CSR_PMPCFG0..=CSR_PMPCFG3 | CSR_PMPADDR0..=CSR_PMPADDR15
                if self.profile.esp32c6_memory_protection_csrs =>
            {
                self.csrs[usize::from(address)]
            }
            CSR_PMACFG0..=CSR_PMACFG15 | CSR_PMAADDR0..=CSR_PMAADDR15
                if self.profile.esp32c6_memory_protection_csrs =>
            {
                self.csrs[usize::from(address)]
            }
            CSR_ESP_PCER_MACHINE | CSR_ESP_PCMR_MACHINE
                if self.profile.esp32c6_memory_protection_csrs =>
            {
                self.csrs[usize::from(address)]
            }
            CSR_ESP_PCCR_MACHINE if self.profile.esp32c6_memory_protection_csrs => {
                self.cycle as u32
            }
            CSR_MSTATUS | CSR_MISA | CSR_MEDELEG | CSR_MIDELEG | CSR_MIE | CSR_MTVEC
            | CSR_MCOUNTEREN | CSR_MSCRATCH | CSR_MEPC | CSR_MCAUSE | CSR_MTVAL | CSR_MIP => {
                self.csrs[usize::from(address)]
            }
            _ => {
                return Err(CpuFault::new(
                    CpuFaultKind::Unsupported,
                    self.pc.into(),
                    format!("unmodeled CSR {address:#05x}"),
                ));
            }
        };
        Ok(value)
    }

    fn write_csr(&mut self, address: u16, value: u32) -> Result<(), CpuFault> {
        match address {
            CSR_MEIEA | CSR_MEIFA | CSR_MEIPRA => {
                let index = (value & 0x1f) as usize;
                let window = (value >> 16) as u16;
                match address {
                    CSR_MEIEA => self.hazard3_external_enabled[index] = window,
                    CSR_MEIFA => self.hazard3_external_forced[index] = window,
                    CSR_MEIPRA => self.hazard3_external_priorities[index] = window,
                    _ => unreachable!(),
                }
                self.refresh_hazard3_machine_external();
            }
            CSR_MEIPA | CSR_MEINEXT => {}
            CSR_MEICONTEXT => {
                self.csrs[usize::from(address)] = value;
                self.refresh_hazard3_machine_external();
            }
            CSR_PMPCFG0..=CSR_PMPCFG3 | CSR_PMPADDR0..=CSR_PMPADDR15
                if self.profile.esp32c6_memory_protection_csrs =>
            {
                self.csrs[usize::from(address)] = value;
            }
            CSR_PMACFG0..=CSR_PMACFG15 | CSR_PMAADDR0..=CSR_PMAADDR15
                if self.profile.esp32c6_memory_protection_csrs =>
            {
                self.csrs[usize::from(address)] = value;
            }
            CSR_ESP_PCER_MACHINE | CSR_ESP_PCMR_MACHINE
                if self.profile.esp32c6_memory_protection_csrs =>
            {
                self.csrs[usize::from(address)] = value;
            }
            CSR_ESP_PCCR_MACHINE if self.profile.esp32c6_memory_protection_csrs => {
                self.cycle = (self.cycle & 0xffff_ffff_0000_0000) | u64::from(value);
            }
            CSR_MSTATUS | CSR_MEDELEG | CSR_MIDELEG | CSR_MIE | CSR_MTVEC | CSR_MCOUNTEREN
            | CSR_MSCRATCH | CSR_MEPC | CSR_MCAUSE | CSR_MTVAL | CSR_MIP => {
                self.csrs[usize::from(address)] = value;
            }
            CSR_MCYCLE => self.cycle = (self.cycle & 0xffff_ffff_0000_0000) | u64::from(value),
            CSR_MCYCLEH => {
                self.cycle = (self.cycle & 0x0000_0000_ffff_ffff) | (u64::from(value) << 32);
            }
            CSR_MINSTRET => {
                self.instret = (self.instret & 0xffff_ffff_0000_0000) | u64::from(value);
            }
            CSR_MINSTRETH => {
                self.instret = (self.instret & 0x0000_0000_ffff_ffff) | (u64::from(value) << 32);
            }
            CSR_MISA | 0xc00 | 0xc80 | 0xc02 | 0xc82 => {
                return Err(CpuFault::new(
                    CpuFaultKind::IllegalInstruction,
                    self.pc.into(),
                    format!("read-only CSR {address:#05x}"),
                ));
            }
            _ => {
                return Err(CpuFault::new(
                    CpuFaultKind::Unsupported,
                    self.pc.into(),
                    format!("unmodeled CSR {address:#05x}"),
                ));
            }
        }
        Ok(())
    }

    fn pending_interrupt(&self) -> Option<u16> {
        if self.csrs[usize::from(CSR_MSTATUS)] & MSTATUS_MIE == 0 {
            return None;
        }
        self.asserted_interrupts
            .iter()
            .copied()
            .filter(|line| {
                !(*line == 11
                    && self.profile.interrupt_model == InterruptModel::Hazard3
                    && self.hazard3_external_active)
            })
            .find(|line| self.csrs[usize::from(CSR_MIE)] & (1_u32 << line) != 0)
    }

    fn take_interrupt(&mut self, line: u16) {
        let cause = u32::from(line);
        self.csrs[usize::from(CSR_MEPC)] = self.pc;
        self.csrs[usize::from(CSR_MCAUSE)] = 0x8000_0000 | cause;
        self.csrs[usize::from(CSR_MTVAL)] = 0;
        let status = self.csrs[usize::from(CSR_MSTATUS)];
        let previous_ie = (status & MSTATUS_MIE) << 4;
        self.csrs[usize::from(CSR_MSTATUS)] =
            (status & !(MSTATUS_MIE | MSTATUS_MPIE)) | previous_ie;
        let mtvec = self.csrs[usize::from(CSR_MTVEC)];
        if line == 11 && self.profile.interrupt_model == InterruptModel::Hazard3 {
            // Hazard3 saves the external-interrupt preemption context on entry.
            // Until MRET restores that context, an interrupt cannot preempt
            // itself merely because its level-sensitive request remains high.
            self.hazard3_external_active = true;
            self.csrs[0xbe5] |= 1; // meicontext.mreteirq
        }
        self.pc = if mtvec & 0x3 == 1 {
            (mtvec & !0x3).wrapping_add(cause.wrapping_mul(4))
        } else {
            mtvec & !0x3
        };
        self.waiting = false;
    }

    fn execute32(
        &mut self,
        instruction: u32,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let opcode = instruction & 0x7f;
        let rd = ((instruction >> 7) & 0x1f) as u8;
        let funct3 = (instruction >> 12) & 0x7;
        let rs1 = ((instruction >> 15) & 0x1f) as u8;
        let rs2 = ((instruction >> 20) & 0x1f) as u8;
        let funct7 = instruction >> 25;
        let current_pc = self.pc;
        let mut next_pc = self.pc.wrapping_add(4);
        let mut reason = StepReason::Advanced;

        match opcode {
            0x37 => self.write_register(rd, instruction & 0xffff_f000), // LUI
            0x17 => self.write_register(rd, current_pc.wrapping_add(instruction & 0xffff_f000)),
            0x6f => {
                self.check_register(rd)?;
                let immediate = decode_j_immediate(instruction);
                self.write_register(rd, next_pc);
                next_pc = current_pc.wrapping_add_signed(immediate);
            }
            0x67 if funct3 == 0 => {
                let base = self.read_register(rs1)?;
                self.check_register(rd)?;
                let target = base.wrapping_add_signed(sign_extend(instruction >> 20, 12)) & !1;
                self.write_register(rd, next_pc);
                next_pc = target;
            }
            0x63 => {
                let left = self.read_register(rs1)?;
                let right = self.read_register(rs2)?;
                let branch = match funct3 {
                    0 => left == right,
                    1 => left != right,
                    4 => (left as i32) < (right as i32),
                    5 => (left as i32) >= (right as i32),
                    6 => left < right,
                    7 => left >= right,
                    _ => return self.illegal(instruction),
                };
                if branch {
                    next_pc = current_pc.wrapping_add_signed(decode_b_immediate(instruction));
                }
            }
            0x03 => {
                let base = self.read_register(rs1)?;
                self.check_register(rd)?;
                let address = base.wrapping_add_signed(sign_extend(instruction >> 20, 12));
                let value = match funct3 {
                    0 => sign_extend(self.load(bus, address, AccessWidth::Byte, now)?, 8) as u32,
                    1 => {
                        sign_extend(self.load(bus, address, AccessWidth::HalfWord, now)?, 16) as u32
                    }
                    2 => self.load(bus, address, AccessWidth::Word, now)?,
                    4 => self.load(bus, address, AccessWidth::Byte, now)?,
                    5 => self.load(bus, address, AccessWidth::HalfWord, now)?,
                    _ => return self.illegal(instruction),
                };
                self.write_register(rd, value);
            }
            0x23 => {
                let base = self.read_register(rs1)?;
                let value = self.read_register(rs2)?;
                let immediate =
                    sign_extend(((instruction >> 25) << 5) | ((instruction >> 7) & 0x1f), 12);
                let address = base.wrapping_add_signed(immediate);
                let width = match funct3 {
                    0 => AccessWidth::Byte,
                    1 => AccessWidth::HalfWord,
                    2 => AccessWidth::Word,
                    _ => return self.illegal(instruction),
                };
                self.store(bus, address, width, value, now)?;
            }
            0x13 => {
                let left = self.read_register(rs1)?;
                self.check_register(rd)?;
                let immediate = sign_extend(instruction >> 20, 12);
                let bitmanip = self
                    .profile
                    .extension_b
                    .then(|| execute_b_immediate(instruction, funct3, left, rs2))
                    .flatten();
                let value = if let Some(value) = bitmanip {
                    value
                } else {
                    match funct3 {
                        0 => left.wrapping_add_signed(immediate),
                        2 => u32::from((left as i32) < immediate),
                        3 => u32::from(left < immediate as u32),
                        4 => left ^ immediate as u32,
                        6 => left | immediate as u32,
                        7 => left & immediate as u32,
                        1 if funct7 == 0 => left << (rs2 & 0x1f),
                        5 if funct7 == 0 => left >> (rs2 & 0x1f),
                        5 if funct7 == 0x20 => ((left as i32) >> (rs2 & 0x1f)) as u32,
                        _ => return self.illegal(instruction),
                    }
                };
                self.write_register(rd, value);
            }
            0x33 => {
                let left = self.read_register(rs1)?;
                let right = self.read_register(rs2)?;
                self.check_register(rd)?;
                let value = if self.profile.extension_b {
                    if let Some(value) = execute_b_register(funct7, funct3, left, right) {
                        value
                    } else if funct7 == 1 {
                        if !self.profile.extension_m {
                            return self.illegal(instruction);
                        }
                        execute_m(funct3, left, right).ok_or_else(|| {
                            CpuFault::new(
                                CpuFaultKind::IllegalInstruction,
                                self.pc.into(),
                                format!("illegal M-extension instruction {instruction:#010x}"),
                            )
                        })?
                    } else {
                        match (funct7, funct3) {
                            (0x00, 0) => left.wrapping_add(right),
                            (0x20, 0) => left.wrapping_sub(right),
                            (0x00, 1) => left << (right & 0x1f),
                            (0x00, 2) => u32::from((left as i32) < (right as i32)),
                            (0x00, 3) => u32::from(left < right),
                            (0x00, 4) => left ^ right,
                            (0x00, 5) => left >> (right & 0x1f),
                            (0x20, 5) => ((left as i32) >> (right & 0x1f)) as u32,
                            (0x00, 6) => left | right,
                            (0x00, 7) => left & right,
                            _ => return self.illegal(instruction),
                        }
                    }
                } else if funct7 == 1 {
                    if !self.profile.extension_m {
                        return self.illegal(instruction);
                    }
                    execute_m(funct3, left, right).ok_or_else(|| {
                        CpuFault::new(
                            CpuFaultKind::IllegalInstruction,
                            self.pc.into(),
                            format!("illegal M-extension instruction {instruction:#010x}"),
                        )
                    })?
                } else {
                    match (funct7, funct3) {
                        (0x00, 0) => left.wrapping_add(right),
                        (0x20, 0) => left.wrapping_sub(right),
                        (0x00, 1) => left << (right & 0x1f),
                        (0x00, 2) => u32::from((left as i32) < (right as i32)),
                        (0x00, 3) => u32::from(left < right),
                        (0x00, 4) => left ^ right,
                        (0x00, 5) => left >> (right & 0x1f),
                        (0x20, 5) => ((left as i32) >> (right & 0x1f)) as u32,
                        (0x00, 6) => left | right,
                        (0x00, 7) => left & right,
                        _ => return self.illegal(instruction),
                    }
                };
                self.write_register(rd, value);
            }
            0x0f => {} // FENCE/FENCE.I are ordering no-ops in the functional model.
            0x73 => {
                reason = self.execute_system(instruction, rd, rs1, funct3)?;
                if reason == StepReason::WaitForInterrupt {
                    self.waiting = true;
                } else if reason == StepReason::Halted {
                    self.halted = true;
                } else if instruction == 0x3020_0073 {
                    next_pc = self.csrs[usize::from(CSR_MEPC)];
                }
            }
            0x2f if funct3 == 2 && self.profile.extension_a => {
                self.execute_atomic(instruction, rd, rs1, rs2, bus, now)?;
            }
            _ => return self.illegal(instruction),
        }

        if next_pc & if self.profile.extension_c { 1 } else { 3 } != 0 {
            return Err(CpuFault::new(
                CpuFaultKind::Architecture,
                current_pc.into(),
                format!("misaligned instruction target {next_pc:#010x}"),
            ));
        }
        self.pc = next_pc;
        Ok(reason)
    }

    fn execute_system(
        &mut self,
        instruction: u32,
        rd: u8,
        rs1: u8,
        funct3: u32,
    ) -> Result<StepReason, CpuFault> {
        if funct3 == 0 {
            return match instruction {
                0x0000_0073 => Err(CpuFault::new(
                    CpuFaultKind::Architecture,
                    self.pc.into(),
                    "environment call from machine mode",
                )),
                0x0010_0073 if self.profile.ebreak_halts => Ok(StepReason::Halted),
                0x0010_0073 => Ok(StepReason::Breakpoint),
                0x1050_0073 => Ok(StepReason::WaitForInterrupt),
                0x3020_0073 => {
                    let status = self.csrs[usize::from(CSR_MSTATUS)];
                    let restored_ie = (status & MSTATUS_MPIE) >> 4;
                    self.csrs[usize::from(CSR_MSTATUS)] =
                        (status | MSTATUS_MPIE) & !MSTATUS_MIE | restored_ie;
                    if self.profile.interrupt_model == InterruptModel::Hazard3
                        && self.csrs[0xbe5] & 1 != 0
                    {
                        self.hazard3_external_active = false;
                        self.csrs[0xbe5] &= !1;
                    }
                    Ok(StepReason::Advanced)
                }
                _ => self.illegal(instruction),
            };
        }
        if !self.profile.extension_zicsr {
            return self.illegal(instruction);
        }
        let csr_address = (instruction >> 20) as u16;
        let source = match funct3 {
            1..=3 => self.read_register(rs1)?,
            5..=7 => u32::from(rs1),
            _ => return self.illegal(instruction),
        };
        if self.profile.interrupt_model == InterruptModel::Hazard3
            && matches!(csr_address, CSR_MEIEA | CSR_MEIPA | CSR_MEIFA | CSR_MEIPRA)
        {
            let old = self.hazard3_irqarray_access(csr_address, source, funct3)?;
            self.check_register(rd)?;
            self.write_register(rd, old);
            return Ok(StepReason::Advanced);
        }
        let old = self.read_csr(csr_address)?;
        let write = match funct3 {
            1 | 5 => Some(source),
            2 | 6 if source != 0 => Some(old | source),
            3 | 7 if source != 0 => Some(old & !source),
            2 | 3 | 6 | 7 => None,
            _ => return self.illegal(instruction),
        };
        if let Some(value) = write {
            self.write_csr(csr_address, value)?;
        }
        self.check_register(rd)?;
        self.write_register(rd, old);
        Ok(StepReason::Advanced)
    }

    fn execute_atomic(
        &mut self,
        instruction: u32,
        rd: u8,
        rs1: u8,
        rs2: u8,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        let operation = (instruction >> 27) & 0x1f;
        let address = self.read_register(rs1)?;
        self.check_register(rd)?;
        if address & 3 != 0 {
            return Err(CpuFault::new(
                CpuFaultKind::Architecture,
                self.pc.into(),
                format!("misaligned atomic word address {address:#010x}"),
            ));
        }
        if operation == 0x02 {
            if rs2 != 0 {
                return self.illegal(instruction);
            }
            let old = self.load(bus, address, AccessWidth::Word, now)?;
            self.reservation = Some(address);
            self.write_register(rd, old);
            return Ok(());
        }
        if operation == 0x03 {
            let success = self.reservation == Some(address);
            self.reservation = None;
            if success {
                let value = self.read_register(rs2)?;
                self.store(bus, address, AccessWidth::Word, value, now)?;
            }
            self.write_register(rd, u32::from(!success));
            return Ok(());
        }
        let old = self.load(bus, address, AccessWidth::Word, now)?;
        let operand = self.read_register(rs2)?;
        let value = match operation {
            0x00 => old.wrapping_add(operand),
            0x01 => operand,
            0x04 => old ^ operand,
            0x08 => old | operand,
            0x0c => old & operand,
            0x10 => (old as i32).min(operand as i32) as u32,
            0x14 => (old as i32).max(operand as i32) as u32,
            0x18 => old.min(operand),
            0x1c => old.max(operand),
            _ => return self.illegal(instruction),
        };
        self.store(bus, address, AccessWidth::Word, value, now)?;
        self.write_register(rd, old);
        Ok(())
    }

    fn execute16(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let quadrant = instruction & 0x3;
        let funct3 = instruction >> 13;
        let current_pc = self.pc;
        let mut next_pc = self.pc.wrapping_add(2);
        let mut reason = StepReason::Advanced;

        match (quadrant, funct3) {
            (0, 0) => {
                let immediate = decode_c_addi4spn(instruction);
                if immediate == 0 {
                    return self.illegal16(instruction);
                }
                let rd = compact_register((instruction >> 2) & 0x7);
                self.write_register(rd, self.registers[2].wrapping_add(immediate));
            }
            (0, 2) => {
                let rd = compact_register((instruction >> 2) & 0x7);
                let rs1 = compact_register((instruction >> 7) & 0x7);
                let address = self
                    .read_register(rs1)?
                    .wrapping_add(decode_c_lw_sw(instruction));
                let value = self.load(bus, address, AccessWidth::Word, now)?;
                self.write_register(rd, value);
            }
            (0, 4) if self.profile.extension_b => {
                let operation = (instruction >> 10) & 3;
                let register = compact_register((instruction >> 2) & 7);
                let base = self.read_register(compact_register((instruction >> 7) & 7))?;
                match operation {
                    0 => {
                        let immediate = u32::from((instruction >> 6) & 1)
                            | (u32::from((instruction >> 5) & 1) << 1);
                        let address = base.wrapping_add(immediate);
                        let value = self.load(bus, address, AccessWidth::Byte, now)?;
                        self.write_register(register, value);
                    }
                    1 => {
                        let address = base.wrapping_add(u32::from((instruction >> 5) & 1) * 2);
                        let value = self.load(bus, address, AccessWidth::HalfWord, now)?;
                        self.write_register(
                            register,
                            if instruction & (1 << 6) != 0 {
                                i32::from(value as u16 as i16) as u32
                            } else {
                                value
                            },
                        );
                    }
                    2 => {
                        let immediate = u32::from((instruction >> 6) & 1)
                            | (u32::from((instruction >> 5) & 1) << 1);
                        let address = base.wrapping_add(immediate);
                        self.store(
                            bus,
                            address,
                            AccessWidth::Byte,
                            self.read_register(register)?,
                            now,
                        )?;
                    }
                    3 => {
                        let address = base.wrapping_add(u32::from((instruction >> 5) & 1) * 2);
                        self.store(
                            bus,
                            address,
                            AccessWidth::HalfWord,
                            self.read_register(register)?,
                            now,
                        )?;
                    }
                    _ => unreachable!(),
                }
            }
            (0, 6) => {
                let rs2 = compact_register((instruction >> 2) & 0x7);
                let rs1 = compact_register((instruction >> 7) & 0x7);
                let address = self
                    .read_register(rs1)?
                    .wrapping_add(decode_c_lw_sw(instruction));
                self.store(
                    bus,
                    address,
                    AccessWidth::Word,
                    self.read_register(rs2)?,
                    now,
                )?;
            }
            (1, 0) => {
                let rd = ((instruction >> 7) & 0x1f) as u8;
                self.check_register(rd)?;
                let immediate = decode_c_imm6(instruction);
                self.write_register(rd, self.read_register(rd)?.wrapping_add_signed(immediate));
            }
            (1, 1) => {
                // RV32 C.JAL
                self.write_register(1, next_pc);
                next_pc = current_pc.wrapping_add_signed(decode_c_jump(instruction));
            }
            (1, 2) => {
                let rd = ((instruction >> 7) & 0x1f) as u8;
                self.check_register(rd)?;
                self.write_register(rd, decode_c_imm6(instruction) as u32);
            }
            (1, 3) => {
                let rd = ((instruction >> 7) & 0x1f) as u8;
                if rd == 2 {
                    let immediate = decode_c_addi16sp(instruction);
                    if immediate == 0 {
                        return self.illegal16(instruction);
                    }
                    self.registers[2] = self.registers[2].wrapping_add_signed(immediate);
                } else {
                    self.check_register(rd)?;
                    let immediate = decode_c_imm6(instruction);
                    if rd == 0 || immediate == 0 {
                        return self.illegal16(instruction);
                    }
                    self.write_register(rd, (immediate as u32) << 12);
                }
            }
            (1, 4) => self.execute_c_alu(instruction)?,
            (1, 5) => next_pc = current_pc.wrapping_add_signed(decode_c_jump(instruction)),
            (1, 6 | 7) => {
                let rs1 = compact_register((instruction >> 7) & 0x7);
                let zero = self.read_register(rs1)? == 0;
                if (funct3 == 6 && zero) || (funct3 == 7 && !zero) {
                    next_pc = current_pc.wrapping_add_signed(decode_c_branch(instruction));
                }
            }
            (2, 0) => {
                let rd = ((instruction >> 7) & 0x1f) as u8;
                self.check_register(rd)?;
                let shift = ((instruction >> 2) & 0x1f) as u32;
                if rd == 0 || instruction & (1 << 12) != 0 {
                    return self.illegal16(instruction);
                }
                self.write_register(rd, self.read_register(rd)? << shift);
            }
            (2, 2) => {
                let rd = ((instruction >> 7) & 0x1f) as u8;
                self.check_register(rd)?;
                if rd == 0 {
                    return self.illegal16(instruction);
                }
                let address = self.registers[2].wrapping_add(decode_c_lwsp(instruction));
                let value = self.load(bus, address, AccessWidth::Word, now)?;
                self.write_register(rd, value);
            }
            (2, 4) => {
                let rd_rs1 = ((instruction >> 7) & 0x1f) as u8;
                let rs2 = ((instruction >> 2) & 0x1f) as u8;
                self.check_register(rd_rs1)?;
                self.check_register(rs2)?;
                let bit12 = instruction & (1 << 12) != 0;
                match (bit12, rd_rs1, rs2) {
                    (false, 0, _) => return self.illegal16(instruction),
                    (false, _, 0) => next_pc = self.read_register(rd_rs1)? & !1,
                    (false, _, _) => self.write_register(rd_rs1, self.read_register(rs2)?),
                    (true, 0, 0) if self.profile.ebreak_halts => {
                        self.halted = true;
                        reason = StepReason::Halted;
                    }
                    (true, 0, 0) => reason = StepReason::Breakpoint,
                    (true, _, 0) => {
                        self.write_register(1, next_pc);
                        next_pc = self.read_register(rd_rs1)? & !1;
                    }
                    (true, _, _) => self.write_register(
                        rd_rs1,
                        self.read_register(rd_rs1)?
                            .wrapping_add(self.read_register(rs2)?),
                    ),
                }
            }
            (2, 5) if self.profile.extension_zcmp && instruction & 0xf800 == 0xb800 => {
                if let Some(return_address) = self.execute_zcmp(instruction, bus, now)? {
                    next_pc = return_address;
                }
            }
            (2, 6) => {
                let rs2 = ((instruction >> 2) & 0x1f) as u8;
                self.check_register(rs2)?;
                let address = self.registers[2].wrapping_add(decode_c_swsp(instruction));
                self.store(
                    bus,
                    address,
                    AccessWidth::Word,
                    self.read_register(rs2)?,
                    now,
                )?;
            }
            _ => return self.illegal16(instruction),
        }
        if next_pc & 1 != 0 {
            return Err(CpuFault::new(
                CpuFaultKind::Architecture,
                current_pc.into(),
                format!("misaligned compressed instruction target {next_pc:#010x}"),
            ));
        }
        self.pc = next_pc;
        Ok(reason)
    }

    fn execute_zcmp(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<Option<u32>, CpuFault> {
        let rlist = ((instruction >> 4) & 0xf) as u8;
        if rlist < 4 {
            return self.illegal16(instruction);
        }
        let registers: &[u8] = match rlist {
            4 => &[1],
            5 => &[1, 8],
            6 => &[1, 8, 9],
            7 => &[1, 8, 9, 18],
            8 => &[1, 8, 9, 18, 19],
            9 => &[1, 8, 9, 18, 19, 20],
            10 => &[1, 8, 9, 18, 19, 20, 21],
            11 => &[1, 8, 9, 18, 19, 20, 21, 22],
            12 => &[1, 8, 9, 18, 19, 20, 21, 22, 23],
            13 => &[1, 8, 9, 18, 19, 20, 21, 22, 23, 24],
            14 => &[1, 8, 9, 18, 19, 20, 21, 22, 23, 24, 25],
            15 => &[1, 8, 9, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27],
            _ => unreachable!(),
        };
        let stack_base = match rlist {
            4..=7 => 16,
            8..=11 => 32,
            12..=14 => 48,
            15 => 64,
            _ => unreachable!(),
        };
        let stack_adjust = stack_base + u32::from((instruction >> 2) & 3) * 16;
        let operation = (instruction >> 8) & 7;

        if operation == 0 {
            let old_sp = self.registers[2];
            for (index, register) in registers.iter().rev().enumerate() {
                let address = old_sp.wrapping_sub(((index + 1) * 4) as u32);
                self.store(
                    bus,
                    address,
                    AccessWidth::Word,
                    self.read_register(*register)?,
                    now,
                )?;
            }
            self.registers[2] = old_sp.wrapping_sub(stack_adjust);
            return Ok(None);
        }

        if !matches!(operation, 2 | 4 | 6) {
            return self.illegal16(instruction);
        }
        let old_sp = self.registers[2];
        for (index, register) in registers.iter().rev().enumerate() {
            let address = old_sp
                .wrapping_add(stack_adjust)
                .wrapping_sub(((index + 1) * 4) as u32);
            let value = self.load(bus, address, AccessWidth::Word, now)?;
            self.write_register(*register, value);
        }
        self.registers[2] = old_sp.wrapping_add(stack_adjust);
        if operation == 4 {
            self.write_register(10, 0);
        }
        Ok((operation >= 4).then(|| self.registers[1] & !1))
    }

    fn execute_c_alu(&mut self, instruction: u16) -> Result<(), CpuFault> {
        let rd = compact_register((instruction >> 7) & 0x7);
        let operation = (instruction >> 10) & 0x3;
        let immediate = decode_c_imm6(instruction);
        let left = self.read_register(rd)?;
        let value = match operation {
            0 => {
                if instruction & (1 << 12) != 0 {
                    return self.illegal16(instruction);
                }
                left >> (u32::from(instruction >> 2) & 0x1f)
            }
            1 => {
                if instruction & (1 << 12) != 0 {
                    return self.illegal16(instruction);
                }
                ((left as i32) >> (u32::from(instruction >> 2) & 0x1f)) as u32
            }
            2 => left & immediate as u32,
            3 => {
                if instruction & (1 << 12) != 0 {
                    return self.illegal16(instruction);
                }
                let right = self.read_register(compact_register((instruction >> 2) & 0x7))?;
                match (instruction >> 5) & 0x3 {
                    0 => left.wrapping_sub(right),
                    1 => left ^ right,
                    2 => left | right,
                    3 => left & right,
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        };
        self.write_register(rd, value);
        Ok(())
    }

    fn illegal<T>(&self, instruction: u32) -> Result<T, CpuFault> {
        Err(CpuFault::new(
            CpuFaultKind::IllegalInstruction,
            self.pc.into(),
            format!(
                "instruction {instruction:#010x} is not valid for {}",
                self.profile.name
            ),
        ))
    }

    fn illegal16<T>(&self, instruction: u16) -> Result<T, CpuFault> {
        Err(CpuFault::new(
            CpuFaultKind::IllegalInstruction,
            self.pc.into(),
            format!(
                "compressed instruction {instruction:#06x} is not valid for {}",
                self.profile.name
            ),
        ))
    }
}

impl Cpu for RiscVCpu {
    fn architecture(&self) -> Architecture {
        Architecture::RiscV32
    }

    fn reset(&mut self, _kind: ResetKind, _bus: &mut dyn Bus) -> Result<(), CpuFault> {
        self.registers = [0; 32];
        self.csrs = [0; 4096];
        self.pc = self.profile.reset_vector;
        self.cycle = 0;
        self.instret = 0;
        self.waiting = false;
        self.halted = false;
        self.asserted_interrupts.clear();
        self.hazard3_external_interrupts.clear();
        self.hazard3_external_enabled = [0; HAZARD3_IRQ_WINDOWS];
        self.hazard3_external_forced = [0; HAZARD3_IRQ_WINDOWS];
        self.hazard3_external_priorities = [0; HAZARD3_IRQ_WINDOWS];
        self.hazard3_external_active = false;
        self.reservation = None;
        self.initialize_csrs();
        Ok(())
    }

    fn step(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<StepOutcome, CpuFault> {
        if self.halted {
            return Ok(StepOutcome {
                elapsed: SimDuration::ZERO,
                reason: StepReason::Halted,
            });
        }
        if let Some(interrupt) = self.pending_interrupt() {
            self.take_interrupt(interrupt);
            self.cycle = self.cycle.wrapping_add(1);
            return Ok(StepOutcome::advanced(SimDuration::TICK));
        }
        if self.waiting {
            return Ok(StepOutcome {
                elapsed: SimDuration::TICK,
                reason: StepReason::WaitForInterrupt,
            });
        }

        let low = self.fetch16(bus, self.pc, now)?;
        let reason = if low & 0x3 == 0x3 {
            let high = self.fetch16(bus, self.pc.wrapping_add(2), now)?;
            self.execute32(u32::from(low) | (u32::from(high) << 16), bus, now)?
        } else if self.profile.extension_c {
            self.execute16(low, bus, now)?
        } else {
            return self.illegal16(low);
        };
        self.registers[0] = 0;
        self.cycle = self.cycle.wrapping_add(1);
        self.instret = self.instret.wrapping_add(1);
        Ok(StepOutcome {
            elapsed: SimDuration::TICK,
            reason,
        })
    }

    fn set_interrupt(&mut self, line: u16, asserted: bool) -> Result<(), CpuFault> {
        if line >= 32 {
            return Err(CpuFault::new(
                CpuFaultKind::Unsupported,
                self.pc.into(),
                format!("RISC-V interrupt line {line} is outside the modeled 0..31 range"),
            ));
        }
        if asserted {
            self.asserted_interrupts.insert(line);
            self.csrs[usize::from(CSR_MIP)] |= 1_u32 << line;
        } else {
            self.asserted_interrupts.remove(&line);
            self.csrs[usize::from(CSR_MIP)] &= !(1_u32 << line);
        }
        Ok(())
    }

    fn snapshot(&self) -> CpuSnapshot {
        let mut registers = (0..self.profile.registers)
            .map(|index| RegisterValue {
                name: format!("x{index}"),
                value: u64::from(self.registers[usize::from(index)]),
                bits: 32,
            })
            .collect::<Vec<_>>();
        registers.extend([
            RegisterValue {
                name: "mstatus".to_owned(),
                value: u64::from(self.csrs[usize::from(CSR_MSTATUS)]),
                bits: 32,
            },
            RegisterValue {
                name: "mtvec".to_owned(),
                value: u64::from(self.csrs[usize::from(CSR_MTVEC)]),
                bits: 32,
            },
            RegisterValue {
                name: "mscratch".to_owned(),
                value: u64::from(self.csrs[usize::from(CSR_MSCRATCH)]),
                bits: 32,
            },
            RegisterValue {
                name: "mie".to_owned(),
                value: u64::from(self.csrs[usize::from(CSR_MIE)]),
                bits: 32,
            },
            RegisterValue {
                name: "mip".to_owned(),
                value: u64::from(self.csrs[usize::from(CSR_MIP)]),
                bits: 32,
            },
            RegisterValue {
                name: "mepc".to_owned(),
                value: u64::from(self.csrs[usize::from(CSR_MEPC)]),
                bits: 32,
            },
            RegisterValue {
                name: "mcause".to_owned(),
                value: u64::from(self.csrs[usize::from(CSR_MCAUSE)]),
                bits: 32,
            },
        ]);
        CpuSnapshot {
            architecture: Architecture::RiscV32,
            pc: self.pc.into(),
            registers,
            waiting: self.waiting,
            halted: self.halted,
        }
    }
}

fn execute_m(funct3: u32, left: u32, right: u32) -> Option<u32> {
    Some(match funct3 {
        0 => left.wrapping_mul(right),
        1 => (((left as i32 as i64) * (right as i32 as i64)) >> 32) as u32,
        2 => (((left as i32 as i64) * i64::from(right)) >> 32) as u32,
        3 => ((u64::from(left) * u64::from(right)) >> 32) as u32,
        4 => {
            if right == 0 {
                u32::MAX
            } else if left == 0x8000_0000 && right == u32::MAX {
                left
            } else {
                ((left as i32) / (right as i32)) as u32
            }
        }
        5 => {
            if right == 0 {
                u32::MAX
            } else {
                left / right
            }
        }
        6 => {
            if right == 0 {
                left
            } else if left == 0x8000_0000 && right == u32::MAX {
                0
            } else {
                ((left as i32) % (right as i32)) as u32
            }
        }
        7 => {
            if right == 0 {
                left
            } else {
                left % right
            }
        }
        _ => return None,
    })
}

fn execute_b_register(funct7: u32, funct3: u32, left: u32, right: u32) -> Option<u32> {
    let shift = right & 0x1f;
    Some(match (funct7, funct3) {
        // Zba
        (0x10, 2) => right.wrapping_add(left << 1),
        (0x10, 4) => right.wrapping_add(left << 2),
        (0x10, 6) => right.wrapping_add(left << 3),
        // Zbb
        (0x20, 4) => left ^ !right,
        (0x20, 6) => left | !right,
        (0x20, 7) => left & !right,
        (0x30, 1) => left.rotate_left(shift),
        (0x30, 5) => left.rotate_right(shift),
        (0x05, 4) => (left as i32).min(right as i32) as u32,
        (0x05, 5) => left.min(right),
        (0x05, 6) => (left as i32).max(right as i32) as u32,
        (0x05, 7) => left.max(right),
        // Zbkb
        (0x04, 4) => (left & 0xffff) | (right << 16),
        (0x04, 7) => (left & 0xff) | ((right & 0xff) << 8),
        // Zbs
        (0x24, 1) => left & !(1 << shift),
        (0x24, 5) => (left >> shift) & 1,
        (0x34, 1) => left ^ (1 << shift),
        (0x14, 1) => left | (1 << shift),
        _ => return None,
    })
}

fn execute_b_immediate(
    instruction: u32,
    funct3: u32,
    left: u32,
    shift_register: u8,
) -> Option<u32> {
    let immediate = instruction >> 20;
    let funct7 = instruction >> 25;
    let shift = u32::from(shift_register & 0x1f);
    Some(match (funct3, immediate) {
        (1, 0x600) => left.leading_zeros(),
        (1, 0x601) => left.trailing_zeros(),
        (1, 0x602) => left.count_ones(),
        (1, 0x604) => i32::from(left as u8 as i8) as u32,
        (1, 0x605) => i32::from(left as u16 as i16) as u32,
        (5, 0x698) => left.swap_bytes(),
        (5, 0x287) => {
            let mut result = 0_u32;
            for byte in 0..4 {
                if left & (0xff << (byte * 8)) != 0 {
                    result |= 0xff << (byte * 8);
                }
            }
            result
        }
        _ => match (funct7, funct3) {
            (0x30, 5) => left.rotate_right(shift),
            (0x24, 1) => left & !(1 << shift),
            (0x24, 5) => (left >> shift) & 1,
            (0x34, 1) => left ^ (1 << shift),
            (0x14, 1) => left | (1 << shift),
            _ => return None,
        },
    })
}

const fn sign_extend(value: u32, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value << shift) as i32) >> shift
}

const fn decode_j_immediate(instruction: u32) -> i32 {
    let encoded = ((instruction >> 31) << 20)
        | (((instruction >> 12) & 0xff) << 12)
        | (((instruction >> 20) & 1) << 11)
        | (((instruction >> 21) & 0x3ff) << 1);
    sign_extend(encoded, 21)
}

const fn decode_b_immediate(instruction: u32) -> i32 {
    let encoded = ((instruction >> 31) << 12)
        | (((instruction >> 7) & 1) << 11)
        | (((instruction >> 25) & 0x3f) << 5)
        | (((instruction >> 8) & 0xf) << 1);
    sign_extend(encoded, 13)
}

const fn compact_register(encoded: u16) -> u8 {
    8 + encoded as u8
}

fn decode_c_imm6(instruction: u16) -> i32 {
    sign_extend(
        u32::from((instruction >> 2) & 0x1f) | (u32::from(instruction >> 12) << 5),
        6,
    )
}

fn decode_c_addi4spn(instruction: u16) -> u32 {
    (u32::from((instruction >> 6) & 1) << 2)
        | (u32::from((instruction >> 5) & 1) << 3)
        | (u32::from((instruction >> 11) & 0x3) << 4)
        | (u32::from((instruction >> 7) & 0xf) << 6)
}

fn decode_c_lw_sw(instruction: u16) -> u32 {
    (u32::from((instruction >> 6) & 1) << 2)
        | (u32::from((instruction >> 10) & 0x7) << 3)
        | (u32::from((instruction >> 5) & 1) << 6)
}

fn decode_c_lwsp(instruction: u16) -> u32 {
    (u32::from((instruction >> 4) & 0x7) << 2)
        | (u32::from((instruction >> 12) & 1) << 5)
        | (u32::from((instruction >> 2) & 0x3) << 6)
}

fn decode_c_swsp(instruction: u16) -> u32 {
    (u32::from((instruction >> 9) & 0xf) << 2) | (u32::from((instruction >> 7) & 0x3) << 6)
}

fn decode_c_addi16sp(instruction: u16) -> i32 {
    let encoded = (u32::from((instruction >> 6) & 1) << 4)
        | (u32::from((instruction >> 2) & 1) << 5)
        | (u32::from((instruction >> 5) & 1) << 6)
        | (u32::from((instruction >> 3) & 0x3) << 7)
        | (u32::from((instruction >> 12) & 1) << 9);
    sign_extend(encoded, 10)
}

fn decode_c_jump(instruction: u16) -> i32 {
    let encoded = (u32::from((instruction >> 3) & 0x7) << 1)
        | (u32::from((instruction >> 11) & 1) << 4)
        | (u32::from((instruction >> 2) & 1) << 5)
        | (u32::from((instruction >> 7) & 1) << 6)
        | (u32::from((instruction >> 6) & 1) << 7)
        | (u32::from((instruction >> 9) & 0x3) << 8)
        | (u32::from((instruction >> 8) & 1) << 10)
        | (u32::from((instruction >> 12) & 1) << 11);
    sign_extend(encoded, 12)
}

fn decode_c_branch(instruction: u16) -> i32 {
    let encoded = (u32::from((instruction >> 3) & 0x3) << 1)
        | (u32::from((instruction >> 10) & 0x3) << 3)
        | (u32::from((instruction >> 2) & 1) << 5)
        | (u32::from((instruction >> 5) & 0x3) << 6)
        | (u32::from((instruction >> 12) & 1) << 8);
    sign_extend(encoded, 9)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renvo_bus::AddressSpace;

    fn cpu_and_bus(words: &[u32], profile: RiscVProfile) -> (RiscVCpu, AddressSpace) {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 4096, true).unwrap();
        let bytes = words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>();
        bus.load(0, &bytes).unwrap();
        let mut cpu = RiscVCpu::new(profile).unwrap();
        cpu.set_pc(0).unwrap();
        (cpu, bus)
    }

    #[test]
    fn executes_integer_program_and_halts() {
        // addi x1,x0,7; addi x2,x0,5; add x3,x1,x2; ebreak
        let words = [0x0070_0093, 0x0050_0113, 0x0020_81b3, 0x0010_0073];
        let (mut cpu, mut bus) = cpu_and_bus(&words, RiscVProfile::esp32c6());
        cpu.set_pc(0).unwrap();
        for _ in 0..3 {
            assert_eq!(
                cpu.step(&mut bus, SimTime::ZERO).unwrap().reason,
                StepReason::Advanced
            );
        }
        assert_eq!(cpu.register(RiscVRegister::Gp).unwrap(), 12);
        assert_eq!(
            cpu.step(&mut bus, SimTime::ZERO).unwrap().reason,
            StepReason::Halted
        );
    }

    #[test]
    fn rv32e_rejects_high_registers() {
        // addi x16,x0,1
        let (mut cpu, mut bus) = cpu_and_bus(&[0x0010_0813], RiscVProfile::ch32v003());
        let fault = cpu.step(&mut bus, SimTime::ZERO).unwrap_err();
        assert_eq!(fault.kind, CpuFaultKind::IllegalInstruction);
    }

    #[test]
    fn loads_and_stores_little_endian_words() {
        // addi x1,x0,64; addi x2,x0,42; sw x2,0(x1); lw x3,0(x1)
        let words = [0x0400_0093, 0x02a0_0113, 0x0020_a023, 0x0000_a183];
        let (mut cpu, mut bus) = cpu_and_bus(&words, RiscVProfile::esp32c6());
        for _ in words {
            cpu.step(&mut bus, SimTime::ZERO).unwrap();
        }
        assert_eq!(cpu.register(RiscVRegister::Gp).unwrap(), 42);
    }

    #[test]
    fn m_extension_division_corner_cases_match_riscv() {
        assert_eq!(execute_m(4, 7, 0), Some(u32::MAX));
        assert_eq!(execute_m(6, 7, 0), Some(7));
        assert_eq!(execute_m(4, 0x8000_0000, u32::MAX), Some(0x8000_0000));
    }

    #[test]
    fn hazard3_zbkb_pack_combines_low_halfwords() {
        // pack x15,x19,x0
        let (mut cpu, mut bus) = cpu_and_bus(&[0x0809_c7b3], RiscVProfile::rp2350_hazard3());
        cpu.set_register(RiscVRegister::S3, 0x1234_abcd).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(RiscVRegister::A5).unwrap(), 0xabcd);
    }

    #[test]
    fn zbb_signed_and_unsigned_min_max_encodings_are_distinct() {
        assert_eq!(execute_b_register(0x05, 4, u32::MAX, 1), Some(u32::MAX));
        assert_eq!(execute_b_register(0x05, 5, u32::MAX, 1), Some(1));
        assert_eq!(execute_b_register(0x05, 6, u32::MAX, 1), Some(1));
        assert_eq!(execute_b_register(0x05, 7, u32::MAX, 1), Some(u32::MAX));
        // Exact operands from the official RP2350 RISC-V MicroPython CDC path.
        assert_eq!(execute_b_register(0x05, 5, 65_490, 64), Some(64));
    }

    #[test]
    fn zbb_sign_extend_byte_and_halfword_use_their_unary_encodings() {
        assert_eq!(
            execute_b_immediate(0x604a_1a13, 1, 0x0000_00a5, 4),
            Some(0xffff_ffa5)
        );
        assert_eq!(
            execute_b_immediate(0x605a_1a13, 1, 0x0000_a5a5, 5),
            Some(0xffff_a5a5)
        );
    }

    #[test]
    fn esp32c6_memory_protection_csrs_persist_and_reset() {
        let mut cpu = RiscVCpu::new(RiscVProfile::esp32c6()).unwrap();
        let mut bus = AddressSpace::default();

        for (address, value) in [
            (CSR_PMPCFG0, 0x9f18_0f00),
            (CSR_PMPADDR15, 0x3fff_ffff),
            (CSR_PMACFG0, 0x0000_001f),
            (CSR_PMAADDR15, 0x4000_0000),
            (CSR_ESP_PCER_MACHINE, 0x0000_00ff),
            (CSR_ESP_PCMR_MACHINE, 0x0000_0001),
        ] {
            cpu.write_csr(address, value).unwrap();
            assert_eq!(cpu.read_csr(address).unwrap(), value);
        }
        cpu.write_csr(CSR_ESP_PCCR_MACHINE, 1234).unwrap();
        assert_eq!(cpu.read_csr(CSR_ESP_PCCR_MACHINE).unwrap(), 1234);

        cpu.reset(ResetKind::PowerOn, &mut bus).unwrap();
        assert_eq!(cpu.read_csr(CSR_PMPCFG0).unwrap(), 0);
        assert_eq!(cpu.read_csr(CSR_PMPADDR15).unwrap(), 0);
        assert_eq!(cpu.read_csr(CSR_PMACFG0).unwrap(), 0);
        assert_eq!(cpu.read_csr(CSR_PMAADDR15).unwrap(), 0);
        assert_eq!(cpu.read_csr(CSR_ESP_PCER_MACHINE).unwrap(), 0);
        assert_eq!(cpu.read_csr(CSR_ESP_PCMR_MACHINE).unwrap(), 0);
        assert_eq!(cpu.read_csr(CSR_ESP_PCCR_MACHINE).unwrap(), 0);
    }

    #[test]
    fn esp32c6_memory_protection_csrs_are_profile_gated() {
        let cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
        let fault = cpu.read_csr(CSR_PMAADDR0).unwrap_err();
        assert_eq!(fault.kind, CpuFaultKind::Unsupported);
    }

    #[test]
    fn hazard3_zcb_lhu_loads_a_compact_halfword() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 4096, true).unwrap();
        // c.lhu s1,2(a2)
        bus.load(0, &[0x24, 0x86]).unwrap();
        bus.load(0x102, &[0xcd, 0xab]).unwrap();
        let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
        cpu.set_register(RiscVRegister::A2, 0x100).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(RiscVRegister::S1).unwrap(), 0xabcd);
    }

    #[test]
    fn hazard3_zcb_byte_load_and_store_decode_both_immediate_bits() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 4096, true).unwrap();
        // c.lbu s1,3(a2); c.sb s1,3(a2)
        bus.load(0, &[0x64, 0x82, 0x64, 0x8a]).unwrap();
        bus.load(0x103, &[0xa5]).unwrap();
        let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
        cpu.set_register(RiscVRegister::A2, 0x100).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(RiscVRegister::S1).unwrap(), 0xa5);

        cpu.set_register(RiscVRegister::S1, 0x5a).unwrap();
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(
            bus.read(
                0x103,
                AccessWidth::Byte,
                renvo_core::AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            0x5a
        );
    }

    #[test]
    fn hazard3_level_interrupt_cannot_preempt_its_own_handler() {
        let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
        cpu.write_csr(CSR_MSTATUS, MSTATUS_MIE).unwrap();
        cpu.write_csr(CSR_MIE, 1 << 11).unwrap();
        cpu.set_hazard3_external_interrupt(14, true).unwrap();
        cpu.hazard3_irqarray_access(CSR_MEIEA, (1 << 30) | 0, 2)
            .unwrap();

        assert_eq!(cpu.pending_interrupt(), Some(11));
        cpu.take_interrupt(11);
        cpu.write_csr(CSR_MSTATUS, MSTATUS_MIE).unwrap();
        assert_eq!(cpu.pending_interrupt(), None);

        cpu.csrs[usize::from(CSR_MSTATUS)] = MSTATUS_MPIE;
        cpu.execute_system(0x3020_0073, 0, 0, 0).unwrap();
        assert_eq!(cpu.pending_interrupt(), Some(11));
    }

    #[test]
    fn hazard3_external_irq_is_filtered_by_windowed_enable_array() {
        let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
        cpu.write_csr(CSR_MSTATUS, MSTATUS_MIE).unwrap();
        cpu.write_csr(CSR_MIE, 1 << 11).unwrap();
        cpu.set_hazard3_external_interrupt(14, true).unwrap();

        assert_eq!(cpu.pending_interrupt(), None);
        assert_eq!(
            cpu.hazard3_irqarray_access(CSR_MEIPA, 0, 2).unwrap(),
            1 << 30
        );

        cpu.hazard3_irqarray_access(CSR_MEIEA, 1 << 30, 2).unwrap();
        assert_eq!(cpu.pending_interrupt(), Some(11));
        assert_eq!(
            cpu.hazard3_irqarray_access(CSR_MEIEA, 0, 2).unwrap(),
            1 << 30
        );

        cpu.hazard3_irqarray_access(CSR_MEIEA, 1 << 30, 3).unwrap();
        assert_eq!(cpu.pending_interrupt(), None);
    }

    #[test]
    fn hazard3_irq_array_access_selects_nonzero_windows_atomically() {
        let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
        cpu.set_hazard3_external_interrupt(33, true).unwrap();
        let select_window_two = 2;

        assert_eq!(
            cpu.hazard3_irqarray_access(CSR_MEIPA, select_window_two, 2)
                .unwrap(),
            1 << 17
        );
        cpu.hazard3_irqarray_access(CSR_MEIEA, (1 << 17) | select_window_two, 2)
            .unwrap();
        assert_eq!(
            cpu.hazard3_irqarray_access(CSR_MEIEA, select_window_two, 2)
                .unwrap(),
            1 << 17
        );
    }

    #[test]
    fn hazard3_zcmp_push_and_popret_preserve_return_address() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 4096, true).unwrap();
        // cm.push {ra},-16; cm.popret {ra},16
        bus.load(0, &[0x42, 0xb8, 0x42, 0xbe]).unwrap();
        let mut cpu = RiscVCpu::new(RiscVProfile::rp2350_hazard3()).unwrap();
        cpu.set_register(RiscVRegister::Ra, 0x44).unwrap();
        cpu.set_register(RiscVRegister::Sp, 0x100).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(RiscVRegister::Sp).unwrap(), 0xf0);
        cpu.set_register(RiscVRegister::Ra, 0).unwrap();
        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(RiscVRegister::Ra).unwrap(), 0x44);
        assert_eq!(cpu.register(RiscVRegister::Sp).unwrap(), 0x100);
        assert_eq!(cpu.pc(), 0x44);
    }

    #[test]
    fn compressed_addi_executes_on_qingke_profile() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 64, true).unwrap();
        // c.addi x1, 1; c.ebreak
        bus.load(0, &[0x85, 0x00, 0x02, 0x90]).unwrap();
        let mut cpu = RiscVCpu::new(RiscVProfile::ch32v003()).unwrap();
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(RiscVRegister::Ra).unwrap(), 1);
        assert_eq!(
            cpu.step(&mut bus, SimTime::ZERO).unwrap().reason,
            StepReason::Halted
        );
    }
}
