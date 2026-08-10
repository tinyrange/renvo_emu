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

use remu_core::{
    AccessKind, AccessWidth, Architecture, Bus, Cpu, CpuFault, CpuFaultKind, CpuSnapshot,
    RegisterValue, ResetKind, SimDuration, SimTime, StepOutcome, StepReason,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

mod execution;
const CSR_MSTATUS: u16 = 0x300;
const CSR_USTATUS: u16 = 0x000;
const CSR_UIE: u16 = 0x004;
const CSR_UTVEC: u16 = 0x005;
const CSR_USCRATCH: u16 = 0x040;
const CSR_UEPC: u16 = 0x041;
const CSR_UCAUSE: u16 = 0x042;
const CSR_UTVAL: u16 = 0x043;
const CSR_UIP: u16 = 0x044;
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
const CSR_ESP_PCER_USER: u16 = 0x800;
const CSR_ESP_PCMR_USER: u16 = 0x801;
const CSR_ESP_PCCR_USER: u16 = 0x802;
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
const CSR_QINGKE_INTSYSCR: u16 = 0x804;
const MSTATUS_MIE: u32 = 1 << 3;
const MSTATUS_MPIE: u32 = 1 << 7;
const MSTATUS_MPP: u32 = 3 << 11;
const USTATUS_UIE: u32 = 1;
const USTATUS_UPIE: u32 = 1 << 4;
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

/// Active architectural privilege level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum RiscVPrivilege {
    /// Unprivileged application execution.
    User = 0,
    /// Machine-mode firmware and trap handlers.
    Machine = 3,
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
    /// Multiply-only `Zmmul` subset used by `QingKe` V2C.
    pub extension_zmmul: bool,
    /// Atomic extension.
    pub extension_a: bool,
    /// Compressed extension.
    pub extension_c: bool,
    /// WCH XW compressed byte/halfword memory operations.
    pub extension_xw: bool,
    /// Compiler-facing bit manipulation subset.
    pub extension_b: bool,
    /// Zcmp compressed push/pop register-list instructions.
    pub extension_zcmp: bool,
    /// CSR instructions.
    pub extension_zicsr: bool,
    /// ESP32-C6 physical-memory-attribute and protection CSRs.
    pub esp32c6_memory_protection_csrs: bool,
    /// User mode and machine/user trap transitions.
    pub user_mode: bool,
    /// Initial reset vector.
    pub reset_vector: u32,
    /// Trap/interrupt integration profile.
    pub interrupt_model: InterruptModel,
    /// Treat EBREAK as a deterministic machine halt.
    pub ebreak_halts: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QingKeXwOperation {
    LoadByteCompact,
    LoadHalfCompact,
    StoreByteCompact,
    StoreHalfCompact,
    LoadByteStack,
    LoadHalfStack,
    StoreByteStack,
    StoreHalfStack,
}

impl RiscVProfile {
    /// WCH CH32V003 `QingKe` V2A compiler profile.
    pub fn ch32v003() -> Self {
        Self {
            name: "wch-ch32v003-qingke-v2a".to_owned(),
            registers: 16,
            extension_m: false,
            extension_zmmul: false,
            extension_a: false,
            extension_c: true,
            extension_xw: true,
            extension_b: false,
            extension_zcmp: false,
            extension_zicsr: true,
            esp32c6_memory_protection_csrs: false,
            user_mode: false,
            reset_vector: 0,
            interrupt_model: InterruptModel::QingKe,
            ebreak_halts: true,
        }
    }

    /// WCH CH32V006 `QingKe` V2C compiler profile.
    pub fn ch32v006() -> Self {
        Self {
            name: "wch-ch32v006-qingke-v2c".to_owned(),
            extension_zmmul: true,
            ..Self::ch32v003()
        }
    }

    /// ESP32-C6 high-performance RV32IMAC core profile.
    pub fn esp32c6() -> Self {
        Self {
            name: "espressif-esp32c6-hp".to_owned(),
            registers: 32,
            extension_m: true,
            extension_zmmul: false,
            extension_a: true,
            extension_c: true,
            extension_xw: false,
            extension_b: false,
            extension_zcmp: false,
            extension_zicsr: true,
            esp32c6_memory_protection_csrs: true,
            user_mode: true,
            reset_vector: 0x4000_0000,
            interrupt_model: InterruptModel::Machine,
            ebreak_halts: true,
        }
    }

    /// ESP32-C6 low-power RV32IMAC core profile.
    pub fn esp32c6_lp() -> Self {
        Self {
            name: "espressif-esp32c6-lp".to_owned(),
            esp32c6_memory_protection_csrs: false,
            user_mode: false,
            reset_vector: 0x5000_0080,
            ..Self::esp32c6()
        }
    }

    /// RP2350 Hazard3 compiler profile.
    pub fn rp2350_hazard3() -> Self {
        Self {
            name: "raspberrypi-rp2350-hazard3".to_owned(),
            registers: 32,
            extension_m: true,
            extension_zmmul: false,
            extension_a: true,
            extension_c: true,
            extension_xw: false,
            extension_b: true,
            extension_zcmp: true,
            extension_zicsr: true,
            esp32c6_memory_protection_csrs: false,
            user_mode: false,
            reset_vector: 0,
            interrupt_model: InterruptModel::Hazard3,
            ebreak_halts: true,
        }
    }

    const fn supports_m_operation(&self, funct3: u32) -> bool {
        self.extension_m || (self.extension_zmmul && funct3 <= 3)
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
    privilege: RiscVPrivilege,
    asserted_interrupts: BTreeSet<u16>,
    qingke_external_interrupts: BTreeSet<u16>,
    hazard3_external_interrupts: BTreeSet<u16>,
    hazard3_external_enabled: [u16; HAZARD3_IRQ_WINDOWS],
    hazard3_external_forced: [u16; HAZARD3_IRQ_WINDOWS],
    hazard3_external_priorities: [u16; HAZARD3_IRQ_WINDOWS],
    hazard3_external_active: bool,
    esp32c6_active_interrupts: Vec<u16>,
    reservation: Option<u32>,
    pending_memory_trap: Option<(u32, u32)>,
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
            privilege: RiscVPrivilege::Machine,
            asserted_interrupts: BTreeSet::new(),
            qingke_external_interrupts: BTreeSet::new(),
            hazard3_external_interrupts: BTreeSet::new(),
            hazard3_external_enabled: [0; HAZARD3_IRQ_WINDOWS],
            hazard3_external_forced: [0; HAZARD3_IRQ_WINDOWS],
            hazard3_external_priorities: [0; HAZARD3_IRQ_WINDOWS],
            hazard3_external_active: false,
            esp32c6_active_interrupts: Vec::new(),
            reservation: None,
            pending_memory_trap: None,
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

    /// Current architectural privilege level.
    pub const fn privilege(&self) -> RiscVPrivilege {
        self.privilege
    }

    /// Releases a WFI wait after a platform power-controller wake event.
    pub fn wake_from_wait(&mut self) {
        self.waiting = false;
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

    /// Asserts or clears one WCH PFIC interrupt input.
    ///
    /// PFIC interrupt numbers are not `mie` bit positions: `QingKe` table mode
    /// uses the full device interrupt number to select an entry from the
    /// vector table rooted at `mtvec`.
    pub fn set_qingke_external_interrupt(
        &mut self,
        line: u16,
        asserted: bool,
    ) -> Result<(), CpuFault> {
        if self.profile.interrupt_model != InterruptModel::QingKe || line >= 256 {
            return Err(CpuFault::new(
                CpuFaultKind::Unsupported,
                self.pc.into(),
                format!("QingKe external interrupt line {line} is unavailable"),
            ));
        }
        if asserted {
            self.qingke_external_interrupts.insert(line);
        } else {
            self.qingke_external_interrupts.remove(&line);
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

    fn hazard3_update_context_from_next(&mut self) {
        const IRQ_MASK: u32 = 0x0000_1ff0;
        const NOIRQ: u32 = 0x0000_8000;
        const PREEMPT_MASK: u32 = 0x001f_0000;

        let next = self.hazard3_next_external();
        let mut context =
            self.csrs[usize::from(CSR_MEICONTEXT)] & !(IRQ_MASK | NOIRQ | PREEMPT_MASK);
        if let Some(line) = next {
            context |= u32::from(line) << 4;
            context |= u32::from(self.hazard3_irq_priority(line).saturating_add(1)) << 16;

            // A forced request in MEIFA is acknowledged by the same MEINEXT
            // update that selects it. Level-sensitive peripheral requests
            // remain asserted until their device status is cleared.
            let window = usize::from(line / 16);
            let bit = line % 16;
            self.hazard3_external_forced[window] &= !(1 << bit);
        } else {
            context |= NOIRQ | PREEMPT_MASK;
        }
        self.csrs[usize::from(CSR_MEICONTEXT)] = context;
        self.refresh_hazard3_machine_external();
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
        if self.profile.user_mode {
            misa |= 1 << (b'U' - b'A');
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
        self.check_pmp_access(address, AccessWidth::HalfWord, AccessKind::Execute)?;
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
        self.check_pmp_access(address, width, AccessKind::Read)?;
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
        self.check_pmp_access(address, width, AccessKind::Write)?;
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

    fn pmp_config(&self, entry: usize) -> u8 {
        let csr = CSR_PMPCFG0 + u16::try_from(entry / 4).expect("PMP entry index fits");
        ((self.csrs[usize::from(csr)] >> ((entry % 4) * 8)) & 0xff) as u8
    }

    fn pmp_range(&self, entry: usize, config: u8) -> Option<(u64, u64)> {
        let encoded = u64::from(
            self.csrs
                [usize::from(CSR_PMPADDR0 + u16::try_from(entry).expect("PMP entry index fits"))],
        );
        match (config >> 3) & 3 {
            0 => None,
            1 => {
                let lower = if entry == 0 {
                    0
                } else {
                    u64::from(
                        self.csrs[usize::from(
                            CSR_PMPADDR0 + u16::try_from(entry - 1).expect("PMP entry index fits"),
                        )],
                    ) << 2
                };
                Some((lower, encoded << 2))
            }
            2 => Some((encoded << 2, (encoded << 2).saturating_add(4))),
            3 => {
                let trailing_ones = encoded.trailing_ones().min(29);
                let size = 1_u64 << (trailing_ones + 3);
                let encoded_mask = (1_u64 << trailing_ones) - 1;
                let base = (encoded & !encoded_mask) << 2;
                Some((base, base.saturating_add(size)))
            }
            _ => unreachable!(),
        }
    }

    fn check_pmp_access(
        &mut self,
        address: u32,
        width: AccessWidth,
        kind: AccessKind,
    ) -> Result<(), CpuFault> {
        if !self.profile.esp32c6_memory_protection_csrs {
            return Ok(());
        }
        let bytes = match width {
            AccessWidth::Byte => 1,
            AccessWidth::HalfWord => 2,
            AccessWidth::Word => 4,
            AccessWidth::DoubleWord => 8,
        };
        let start = u64::from(address);
        let end = start.saturating_add(bytes);
        let mut permission = None;
        for entry in 0..16 {
            let config = self.pmp_config(entry);
            let Some((lower, upper)) = self.pmp_range(entry, config) else {
                continue;
            };
            if start < upper && end > lower {
                let wholly_contained = start >= lower && end <= upper;
                let applies = config & 0x80 != 0 || self.privilege == RiscVPrivilege::User;
                let allowed = match kind {
                    AccessKind::Execute => config & 4 != 0,
                    AccessKind::Read => config & 1 != 0,
                    AccessKind::Write => config & 2 != 0 && config & 1 != 0,
                };
                permission = Some(!applies || wholly_contained && allowed);
                break;
            }
        }
        let allowed = permission.unwrap_or(self.privilege == RiscVPrivilege::Machine);
        if allowed {
            return Ok(());
        }
        let cause = match kind {
            AccessKind::Execute => 1,
            AccessKind::Read => 5,
            AccessKind::Write => 7,
        };
        self.pending_memory_trap = Some((cause, address));
        Err(CpuFault::new(
            CpuFaultKind::Architecture,
            u64::from(address),
            format!("PMP denied {kind:?} access at {address:#010x}"),
        ))
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
            CSR_PMPCFG0..=CSR_PMPCFG3
            | CSR_PMPADDR0..=CSR_PMPADDR15
            | CSR_PMACFG0..=CSR_PMACFG15
            | CSR_PMAADDR0..=CSR_PMAADDR15
            | CSR_ESP_PCER_MACHINE
            | CSR_ESP_PCMR_MACHINE
                if self.profile.esp32c6_memory_protection_csrs =>
            {
                self.csrs[usize::from(address)]
            }
            CSR_ESP_PCCR_MACHINE | CSR_ESP_PCCR_USER
                if self.profile.esp32c6_memory_protection_csrs =>
            {
                self.cycle as u32
            }
            CSR_ESP_PCER_USER if self.profile.esp32c6_memory_protection_csrs => {
                self.csrs[usize::from(CSR_ESP_PCER_MACHINE)]
            }
            CSR_ESP_PCMR_USER if self.profile.esp32c6_memory_protection_csrs => {
                self.csrs[usize::from(CSR_ESP_PCMR_MACHINE)]
            }
            CSR_QINGKE_INTSYSCR if self.profile.interrupt_model == InterruptModel::QingKe => {
                self.csrs[usize::from(address)]
            }
            CSR_MSTATUS | CSR_MISA | CSR_MEDELEG | CSR_MIDELEG | CSR_MIE | CSR_MTVEC
            | CSR_MCOUNTEREN | CSR_MSCRATCH | CSR_MEPC | CSR_MCAUSE | CSR_MTVAL | CSR_MIP => {
                self.csrs[usize::from(address)]
            }
            CSR_USTATUS | CSR_UIE | CSR_UTVEC | CSR_USCRATCH | CSR_UEPC | CSR_UCAUSE
            | CSR_UTVAL | CSR_UIP
                if self.profile.user_mode =>
            {
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
            CSR_MEIPA => {}
            CSR_MEINEXT => {
                if value & 1 != 0 {
                    self.hazard3_update_context_from_next();
                }
            }
            CSR_MEICONTEXT => {
                const MRETEIRQ: u32 = 1;
                const CLEARTS: u32 = 1 << 1;
                const MSIESAVE: u32 = 1 << 2;
                const MTIESAVE: u32 = 1 << 3;
                const WRITABLE_CONTEXT: u32 = 0xff1f_9ff1;

                if value & CLEARTS != 0 {
                    self.csrs[usize::from(CSR_MIE)] &= !((1 << 3) | (1 << 7));
                } else {
                    if value & MSIESAVE != 0 {
                        self.csrs[usize::from(CSR_MIE)] |= 1 << 3;
                    }
                    if value & MTIESAVE != 0 {
                        self.csrs[usize::from(CSR_MIE)] |= 1 << 7;
                    }
                }
                self.csrs[usize::from(address)] = (value & WRITABLE_CONTEXT) | (value & MRETEIRQ);
                self.refresh_hazard3_machine_external();
            }
            CSR_PMPCFG0..=CSR_PMPCFG3 if self.profile.esp32c6_memory_protection_csrs => {
                let first = usize::from(address - CSR_PMPCFG0) * 4;
                let mut merged = self.csrs[usize::from(address)];
                for byte in 0..4 {
                    if self.pmp_config(first + byte) & 0x80 == 0 {
                        let mask = 0xff_u32 << (byte * 8);
                        merged = (merged & !mask) | (value & mask);
                    }
                }
                self.csrs[usize::from(address)] = merged;
            }
            CSR_PMPADDR0..=CSR_PMPADDR15 if self.profile.esp32c6_memory_protection_csrs => {
                let entry = usize::from(address - CSR_PMPADDR0);
                let own_locked = self.pmp_config(entry) & 0x80 != 0;
                let tor_locked = entry < 15 && self.pmp_config(entry + 1) & 0x98 == 0x88;
                if !own_locked && !tor_locked {
                    self.csrs[usize::from(address)] = value;
                }
            }
            CSR_PMACFG0..=CSR_PMACFG15
            | CSR_PMAADDR0..=CSR_PMAADDR15
            | CSR_ESP_PCER_MACHINE
            | CSR_ESP_PCMR_MACHINE
                if self.profile.esp32c6_memory_protection_csrs =>
            {
                self.csrs[usize::from(address)] = value;
            }
            CSR_ESP_PCCR_MACHINE | CSR_ESP_PCCR_USER
                if self.profile.esp32c6_memory_protection_csrs =>
            {
                self.cycle = (self.cycle & 0xffff_ffff_0000_0000) | u64::from(value);
            }
            CSR_ESP_PCER_USER if self.profile.esp32c6_memory_protection_csrs => {
                self.csrs[usize::from(CSR_ESP_PCER_MACHINE)] = value;
            }
            CSR_ESP_PCMR_USER if self.profile.esp32c6_memory_protection_csrs => {
                self.csrs[usize::from(CSR_ESP_PCMR_MACHINE)] = value;
            }
            CSR_QINGKE_INTSYSCR if self.profile.interrupt_model == InterruptModel::QingKe => {
                self.csrs[usize::from(address)] = value;
            }
            CSR_MSTATUS | CSR_MEDELEG | CSR_MIDELEG | CSR_MIE | CSR_MTVEC | CSR_MCOUNTEREN
            | CSR_MSCRATCH | CSR_MEPC | CSR_MCAUSE | CSR_MTVAL | CSR_MIP => {
                self.csrs[usize::from(address)] = value;
                if address == CSR_MIDELEG {
                    self.csrs[usize::from(CSR_UIP)] = self.csrs[usize::from(CSR_MIP)] & value;
                }
            }
            CSR_USTATUS | CSR_UIE | CSR_UTVEC | CSR_USCRATCH | CSR_UEPC | CSR_UCAUSE
            | CSR_UTVAL | CSR_UIP
                if self.profile.user_mode =>
            {
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
        if self.privilege == RiscVPrivilege::Machine
            && self.csrs[usize::from(CSR_MSTATUS)] & MSTATUS_MIE == 0
        {
            return None;
        }
        if self.profile.interrupt_model == InterruptModel::QingKe
            && let Some(line) = self.qingke_external_interrupts.iter().next()
        {
            return Some(*line);
        }
        self.asserted_interrupts
            .iter()
            .copied()
            .filter(|line| {
                !(self.profile.esp32c6_memory_protection_csrs
                    && self.esp32c6_active_interrupts.contains(line)
                    || *line == 11
                        && self.profile.interrupt_model == InterruptModel::Hazard3
                        && self.hazard3_external_active)
            })
            .find(|line| {
                let bit = 1_u32 << line;
                let delegated = self.privilege == RiscVPrivilege::User
                    && self.profile.user_mode
                    && self.csrs[usize::from(CSR_MIDELEG)] & bit != 0;
                if delegated {
                    self.csrs[usize::from(CSR_USTATUS)] & USTATUS_UIE != 0
                        && self.csrs[usize::from(CSR_UIE)] & bit != 0
                } else {
                    self.csrs[usize::from(CSR_MIE)] & bit != 0
                }
            })
    }

    fn take_interrupt(&mut self, line: u16) {
        let cause = u32::from(line);
        self.enter_trap(cause, 0, true);
        if line == 11 && self.profile.interrupt_model == InterruptModel::Hazard3 {
            // Hazard3 saves the external-interrupt preemption context on entry.
            // Until MRET restores that context, an interrupt cannot preempt
            // itself merely because its level-sensitive request remains high.
            self.hazard3_external_active = true;
            self.csrs[0xbe5] |= 1; // meicontext.mreteirq
        } else if self.profile.esp32c6_memory_protection_csrs {
            // ESP32-C6's interrupt controller raises the effective threshold
            // on entry. A level request cannot recursively preempt its own
            // handler merely because the common prologue re-enables MIE.
            self.esp32c6_active_interrupts.push(line);
        }
    }

    fn enter_trap(&mut self, cause: u32, trap_value: u32, interrupt: bool) {
        let delegation = if interrupt { CSR_MIDELEG } else { CSR_MEDELEG };
        let delegated_to_user = self.profile.user_mode
            && self.privilege == RiscVPrivilege::User
            && cause < 32
            && self.csrs[usize::from(delegation)] & (1_u32 << cause) != 0;
        if delegated_to_user {
            self.csrs[usize::from(CSR_UEPC)] = self.pc;
            self.csrs[usize::from(CSR_UCAUSE)] = cause | if interrupt { 1 << 31 } else { 0 };
            self.csrs[usize::from(CSR_UTVAL)] = trap_value;
            let status = self.csrs[usize::from(CSR_USTATUS)];
            let previous_ie = (status & USTATUS_UIE) << 4;
            self.csrs[usize::from(CSR_USTATUS)] =
                (status & !(USTATUS_UIE | USTATUS_UPIE)) | previous_ie;
            let utvec = self.csrs[usize::from(CSR_UTVEC)];
            self.pc = if interrupt && utvec & 3 == 1 {
                (utvec & !3).wrapping_add(cause.wrapping_mul(4))
            } else {
                utvec & !3
            };
            self.waiting = false;
            return;
        }
        self.csrs[usize::from(CSR_MEPC)] = self.pc;
        self.csrs[usize::from(CSR_MCAUSE)] = cause | if interrupt { 1 << 31 } else { 0 };
        self.csrs[usize::from(CSR_MTVAL)] = trap_value;
        let status = self.csrs[usize::from(CSR_MSTATUS)];
        let previous_ie = (status & MSTATUS_MIE) << 4;
        let previous_privilege = (self.privilege as u32) << 11;
        self.csrs[usize::from(CSR_MSTATUS)] = (status
            & !(MSTATUS_MIE | MSTATUS_MPIE | MSTATUS_MPP))
            | previous_ie
            | previous_privilege;
        self.privilege = RiscVPrivilege::Machine;
        let mtvec = self.csrs[usize::from(CSR_MTVEC)];
        self.pc = if interrupt && mtvec & 0x3 == 1 {
            (mtvec & !0x3).wrapping_add(cause.wrapping_mul(4))
        } else {
            mtvec & !0x3
        };
        self.waiting = false;
    }

    fn take_interrupt_with_bus(
        &mut self,
        line: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        let qingke_table_interrupt = self.profile.interrupt_model == InterruptModel::QingKe
            && self.qingke_external_interrupts.contains(&line)
            && self.csrs[usize::from(CSR_MTVEC)] & 0x3 == 3;
        self.take_interrupt(line);
        if qingke_table_interrupt {
            let table = self.csrs[usize::from(CSR_MTVEC)] & !0x3;
            let entry = table.wrapping_add(u32::from(line).wrapping_mul(4));
            self.pc = self.load(bus, entry, AccessWidth::Word, now)?;
        }
        Ok(())
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
        self.privilege = RiscVPrivilege::Machine;
        self.asserted_interrupts.clear();
        self.qingke_external_interrupts.clear();
        self.hazard3_external_interrupts.clear();
        self.hazard3_external_enabled = [0; HAZARD3_IRQ_WINDOWS];
        self.hazard3_external_forced = [0; HAZARD3_IRQ_WINDOWS];
        self.hazard3_external_priorities = [0; HAZARD3_IRQ_WINDOWS];
        self.hazard3_external_active = false;
        self.esp32c6_active_interrupts.clear();
        self.reservation = None;
        self.pending_memory_trap = None;
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
            self.take_interrupt_with_bus(interrupt, bus, now)?;
            self.cycle = self.cycle.wrapping_add(1);
            return Ok(StepOutcome::advanced(SimDuration::TICK));
        }
        if self.waiting {
            return Ok(StepOutcome {
                elapsed: SimDuration::TICK,
                reason: StepReason::WaitForInterrupt,
            });
        }

        self.pending_memory_trap = None;
        let execution = (|| {
            let low = self.fetch16(bus, self.pc, now)?;
            if low & 0x3 == 0x3 {
                self.check_pmp_access(self.pc, AccessWidth::Word, AccessKind::Execute)?;
                let high = self.fetch16(bus, self.pc.wrapping_add(2), now)?;
                self.execute32(u32::from(low) | (u32::from(high) << 16), bus, now)
            } else if self.profile.extension_c {
                self.execute16(low, bus, now)
            } else {
                self.illegal16(low)
            }
        })();
        let reason = match execution {
            Ok(reason) => reason,
            Err(error) => {
                if let Some((cause, address)) = self.pending_memory_trap.take() {
                    self.enter_trap(cause, address, false);
                    self.registers[0] = 0;
                    self.cycle = self.cycle.wrapping_add(1);
                    return Ok(StepOutcome::advanced(SimDuration::TICK));
                }
                return Err(error);
            }
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
            if self.csrs[usize::from(CSR_MIDELEG)] & (1_u32 << line) != 0 {
                self.csrs[usize::from(CSR_UIP)] |= 1_u32 << line;
            }
        } else {
            self.asserted_interrupts.remove(&line);
            self.csrs[usize::from(CSR_MIP)] &= !(1_u32 << line);
            self.csrs[usize::from(CSR_UIP)] &= !(1_u32 << line);
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
                name: "privilege".to_owned(),
                value: self.privilege as u64,
                bits: 2,
            },
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
mod tests;
