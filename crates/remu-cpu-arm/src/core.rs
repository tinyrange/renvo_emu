//! Interpreted Arm M-profile CPU implementation.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use remu_core::{
    AccessKind, AccessWidth, Architecture, Bus, Cpu, CpuFault, CpuFaultKind, CpuSnapshot,
    RegisterValue, ResetKind, SimDuration, SimTime, StepOutcome, StepReason,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

mod execution_wide;

const N: u32 = 1 << 31;
const Z: u32 = 1 << 30;
const C: u32 = 1 << 29;
const V: u32 = 1 << 28;

/// Compiler-facing M-profile generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArmProfile {
    /// Armv6-M Cortex-M0+ used by RP2040.
    CortexM0Plus,
    /// Armv7E-M Cortex-M4 with FPv4-SP-D16 used by STM32L432 and RA4M1.
    CortexM4F,
    /// Non-secure Armv8-M Mainline Cortex-M33 used by RP2350.
    CortexM33,
}

/// Pending M-profile exception source modeled by the functional core.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ArmException {
    /// Architectural `SysTick` exception (exception number 15).
    SysTick,
    /// NVIC external interrupt, numbered from zero (exception number 16+).
    External(u16),
}

impl ArmException {
    const fn exception_number(self) -> u16 {
        match self {
            Self::SysTick => 15,
            Self::External(line) => line + 16,
        }
    }
}

/// Named Arm M-profile core register.
///
/// The enum keeps architectural register selection explicit at API
/// boundaries; instruction decoding may still use compact array indices
/// internally.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum ArmRegister {
    /// General-purpose register r0.
    R0 = 0,
    /// General-purpose register r1.
    R1 = 1,
    /// General-purpose register r2.
    R2 = 2,
    /// General-purpose register r3.
    R3 = 3,
    /// General-purpose register r4.
    R4 = 4,
    /// General-purpose register r5.
    R5 = 5,
    /// General-purpose register r6.
    R6 = 6,
    /// General-purpose register r7.
    R7 = 7,
    /// General-purpose register r8.
    R8 = 8,
    /// General-purpose register r9.
    R9 = 9,
    /// General-purpose register r10.
    R10 = 10,
    /// General-purpose register r11.
    R11 = 11,
    /// Intra-procedure-call scratch register r12.
    R12 = 12,
    /// Main/process stack pointer r13.
    Sp = 13,
    /// Link register r14.
    Lr = 14,
    /// Program counter r15.
    Pc = 15,
}

impl ArmRegister {
    const fn index(self) -> usize {
        self as usize
    }
}

impl ArmProfile {
    /// Stable profile name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::CortexM0Plus => "cortex-m0plus-armv6m",
            Self::CortexM4F => "cortex-m4f-armv7em",
            Self::CortexM33 => "cortex-m33-armv8m",
        }
    }
}

/// Functional Thumb interpreter for the initial Raspberry Pi compiler baseline.
pub struct ArmCpu {
    profile: ArmProfile,
    registers: [u32; 16],
    xpsr: u32,
    vector_base: u32,
    primask: bool,
    it_state: u8,
    executing_in_it: bool,
    waiting: bool,
    halted: bool,
    interrupts: BTreeSet<ArmException>,
    active_interrupt: Option<ArmException>,
    exclusive_address: Option<u32>,
    fpu_registers: [u64; 32],
}

impl ArmCpu {
    /// Creates a CPU before vector or direct-load initialization.
    pub const fn new(profile: ArmProfile) -> Self {
        Self {
            profile,
            registers: [0; 16],
            xpsr: 1 << 24,
            vector_base: 0,
            primask: false,
            it_state: 0,
            executing_in_it: false,
            waiting: false,
            halted: false,
            interrupts: BTreeSet::new(),
            active_interrupt: None,
            exclusive_address: None,
            fpu_registers: [0; 32],
        }
    }

    /// Returns the compiler-facing architectural profile selected for this core.
    pub const fn profile(&self) -> ArmProfile {
        self.profile
    }

    /// Establishes a direct-load stack pointer and Thumb entry point.
    pub fn set_direct_state(&mut self, stack: u32, entry: u32) -> Result<(), CpuFault> {
        if entry & 1 == 0 {
            return Err(self.fault(
                CpuFaultKind::Architecture,
                "M-profile entry must have the Thumb bit set",
            ));
        }
        self.registers[13] = stack;
        self.registers[15] = entry & !1;
        self.waiting = false;
        self.halted = false;
        self.active_interrupt = None;
        self.it_state = 0;
        self.executing_in_it = false;
        self.exclusive_address = None;
        self.fpu_registers = [0; 32];
        Ok(())
    }

    /// Sets the Thumb return address used when a machine enters application
    /// code from a function-like boot-ROM launch boundary.
    pub fn set_link_register(&mut self, address: u32) -> Result<(), CpuFault> {
        if address & 1 == 0 {
            return Err(self.fault(
                CpuFaultKind::Architecture,
                "M-profile link register must select Thumb state",
            ));
        }
        self.registers[14] = address;
        Ok(())
    }

    /// Reads a core register.
    pub fn register(&self, register: ArmRegister) -> Result<u32, CpuFault> {
        Ok(self.registers[register.index()])
    }

    /// Completes a machine-provided functional call by placing its result in `r0` and returning
    /// through the current link register.
    pub fn complete_host_call(&mut self, result: u32) -> Result<(), CpuFault> {
        let return_address = self.registers[14];
        if return_address & 1 == 0 {
            return Err(self.fault(
                CpuFaultKind::Architecture,
                "functional host call has a non-Thumb return address",
            ));
        }
        self.registers[0] = result;
        self.registers[15] = return_address & !1;
        Ok(())
    }

    /// Completes a functional host call with a scalar result split across `r0` and `r1`.
    pub fn complete_host_call_with_high(
        &mut self,
        result_low: u32,
        result_high: u32,
    ) -> Result<(), CpuFault> {
        self.registers[1] = result_high;
        self.complete_host_call(result_low)
    }

    /// Sets the vector table base used by functional interrupt entry.
    pub fn set_vector_base(&mut self, address: u32) {
        self.vector_base = address & !0x7f;
    }

    /// Latches or clears the architectural `SysTick` exception request.
    pub fn set_systick_interrupt(&mut self, asserted: bool) {
        if asserted {
            self.interrupts.insert(ArmException::SysTick);
            self.waiting = false;
        } else {
            self.interrupts.remove(&ArmException::SysTick);
        }
    }

    fn fault(&self, kind: CpuFaultKind, message: impl Into<String>) -> CpuFault {
        CpuFault::new(kind, u64::from(self.registers[15]), message)
    }

    fn single_register(&self, register: usize) -> u32 {
        let pair = self.fpu_registers[register / 2];
        if register & 1 == 0 {
            pair as u32
        } else {
            (pair >> 32) as u32
        }
    }

    fn set_single_register(&mut self, register: usize, value: u32) {
        let pair = &mut self.fpu_registers[register / 2];
        *pair = if register & 1 == 0 {
            (*pair & 0xffff_ffff_0000_0000) | u64::from(value)
        } else {
            (*pair & 0x0000_0000_ffff_ffff) | (u64::from(value) << 32)
        };
    }

    fn read(
        &self,
        bus: &mut dyn Bus,
        address: u32,
        width: AccessWidth,
        kind: AccessKind,
        now: SimTime,
    ) -> Result<u32, CpuFault> {
        bus.read(u64::from(address), width, kind, now)
            .map(|value| value as u32)
            .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))
    }

    fn write(
        &self,
        bus: &mut dyn Bus,
        address: u32,
        width: AccessWidth,
        value: u32,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        bus.write(u64::from(address), width, u64::from(value), now)
            .map_err(|error| self.fault(CpuFaultKind::Bus, error.to_string()))
    }

    fn nz(&mut self, result: u32) {
        self.xpsr &= !(N | Z);
        if result & N != 0 {
            self.xpsr |= N;
        }
        if result == 0 {
            self.xpsr |= Z;
        }
    }

    fn carry(&mut self, asserted: bool) {
        self.xpsr &= !C;
        if asserted {
            self.xpsr |= C;
        }
    }

    fn add_flags(&mut self, left: u32, right: u32, result: u32) {
        self.nz(result);
        self.xpsr &= !(C | V);
        if u64::from(left) + u64::from(right) > u64::from(u32::MAX) {
            self.xpsr |= C;
        }
        if (!(left ^ right) & (left ^ result)) & N != 0 {
            self.xpsr |= V;
        }
    }

    fn sub_flags(&mut self, left: u32, right: u32, result: u32) {
        self.nz(result);
        self.xpsr &= !(C | V);
        if left >= right {
            self.xpsr |= C;
        }
        if ((left ^ right) & (left ^ result)) & N != 0 {
            self.xpsr |= V;
        }
    }

    fn condition(&self, condition: u16) -> bool {
        let n = self.xpsr & N != 0;
        let z = self.xpsr & Z != 0;
        let c = self.xpsr & C != 0;
        let v = self.xpsr & V != 0;
        match condition {
            0 => z,
            1 => !z,
            2 => c,
            3 => !c,
            4 => n,
            5 => !n,
            6 => v,
            7 => !v,
            8 => c && !z,
            9 => !c || z,
            10 => n == v,
            11 => n != v,
            12 => !z && n == v,
            13 => z || n != v,
            14 => true,
            _ => false,
        }
    }

    fn branch_exchange(
        &mut self,
        target: u32,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        if target & 0xffff_fff0 == 0xffff_fff0 {
            let stack = self.registers[13];
            for (index, register) in [0_usize, 1, 2, 3, 12, 14].into_iter().enumerate() {
                self.registers[register] = self.read(
                    bus,
                    stack.wrapping_add(u32::try_from(index).expect("small index fits u32") * 4),
                    AccessWidth::Word,
                    AccessKind::Read,
                    now,
                )?;
            }
            let return_pc = self.read(
                bus,
                stack.wrapping_add(24),
                AccessWidth::Word,
                AccessKind::Read,
                now,
            )?;
            self.xpsr = self.read(
                bus,
                stack.wrapping_add(28),
                AccessWidth::Word,
                AccessKind::Read,
                now,
            )?;
            self.registers[13] = stack.wrapping_add(32);
            self.registers[15] = return_pc & !1;
            self.active_interrupt = None;
            return Ok(());
        }
        if target & 1 == 0 {
            return Err(self.fault(
                CpuFaultKind::Architecture,
                "branch target does not select Thumb state",
            ));
        }
        self.registers[15] = target & !1;
        Ok(())
    }

    fn execute(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<StepReason, CpuFault> {
        let pc = self.registers[15];
        let next = pc.wrapping_add(2);

        if instruction & 0xe000 == 0 && instruction & 0x1800 != 0x1800 {
            let op = (instruction >> 11) & 3;
            let amount = u32::from((instruction >> 6) & 0x1f);
            let source = self.registers[usize::from((instruction >> 3) & 7)];
            let destination = usize::from(instruction & 7);
            let (result, carry) = match op {
                0 => (
                    source.wrapping_shl(amount),
                    (amount != 0).then(|| source & (1 << (32 - amount)) != 0),
                ),
                1 => {
                    let shift = if amount == 0 { 32 } else { amount };
                    (
                        source.checked_shr(shift).unwrap_or(0),
                        Some(source & (1 << (shift - 1)) != 0),
                    )
                }
                2 => {
                    let shift = if amount == 0 { 32 } else { amount };
                    (
                        ((source as i32)
                            .checked_shr(if amount == 0 { 32 } else { amount })
                            .unwrap_or(if source & N == 0 { 0 } else { -1 }))
                            as u32,
                        Some(source & (1 << (shift - 1)) != 0),
                    )
                }
                _ => unreachable!(),
            };
            self.registers[destination] = result;
            if !self.executing_in_it {
                self.nz(result);
                if let Some(carry) = carry {
                    self.carry(carry);
                }
            }
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xf800 == 0x1800 {
            let immediate = instruction & 0x0400 != 0;
            let subtract = instruction & 0x0200 != 0;
            let operand = if immediate {
                u32::from((instruction >> 6) & 7)
            } else {
                self.registers[usize::from((instruction >> 6) & 7)]
            };
            let source = self.registers[usize::from((instruction >> 3) & 7)];
            let destination = usize::from(instruction & 7);
            let result = if subtract {
                source.wrapping_sub(operand)
            } else {
                source.wrapping_add(operand)
            };
            self.registers[destination] = result;
            if !self.executing_in_it {
                if subtract {
                    self.sub_flags(source, operand, result);
                } else {
                    self.add_flags(source, operand, result);
                }
            }
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xe000 == 0x2000 {
            let op = (instruction >> 11) & 3;
            let destination = usize::from((instruction >> 8) & 7);
            let immediate = u32::from(instruction & 0xff);
            let left = self.registers[destination];
            match op {
                0 => {
                    self.registers[destination] = immediate;
                    if !self.executing_in_it {
                        self.nz(immediate);
                    }
                }
                1 => self.sub_flags(left, immediate, left.wrapping_sub(immediate)),
                2 => {
                    let result = left.wrapping_add(immediate);
                    self.registers[destination] = result;
                    if !self.executing_in_it {
                        self.add_flags(left, immediate, result);
                    }
                }
                3 => {
                    let result = left.wrapping_sub(immediate);
                    self.registers[destination] = result;
                    if !self.executing_in_it {
                        self.sub_flags(left, immediate, result);
                    }
                }
                _ => unreachable!(),
            }
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xfc00 == 0x4000 {
            self.alu(instruction);
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xfc00 == 0x4400 {
            let op = (instruction >> 8) & 3;
            let destination = usize::from((instruction & 7) | ((instruction >> 4) & 8));
            let source = usize::from((instruction >> 3) & 0xf);
            let right = if source == 15 {
                pc.wrapping_add(4)
            } else {
                self.registers[source]
            };
            match op {
                0 => {
                    let left = if destination == 15 {
                        pc.wrapping_add(4)
                    } else {
                        self.registers[destination]
                    };
                    let result = left.wrapping_add(right);
                    if destination == 15 {
                        self.registers[15] = result & !1;
                        return Ok(StepReason::Advanced);
                    }
                    self.registers[destination] = result;
                }
                1 => self.sub_flags(
                    self.registers[destination],
                    right,
                    self.registers[destination].wrapping_sub(right),
                ),
                2 if destination != 15 => self.registers[destination] = right,
                2 => {
                    // A high-register MOV to PC stays in Thumb state; unlike BX,
                    // compiler-generated jump tables contain even code addresses.
                    self.registers[15] = right & !1;
                    return Ok(StepReason::Advanced);
                }
                3 => {
                    if instruction & 0x0080 != 0 {
                        self.registers[14] = next | 1;
                    }
                    self.branch_exchange(right, bus, now)?;
                    return Ok(StepReason::Advanced);
                }
                _ => unreachable!(),
            }
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xf800 == 0x4800 {
            let destination = usize::from((instruction >> 8) & 7);
            let address =
                (pc.wrapping_add(4) & !3).wrapping_add(u32::from(instruction & 0xff) << 2);
            self.registers[destination] =
                self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xf000 == 0x5000 {
            self.register_memory(instruction, bus, now)?;
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xe000 == 0x6000 {
            let byte = instruction & 0x1000 != 0;
            let load = instruction & 0x0800 != 0;
            let offset = u32::from((instruction >> 6) & 0x1f);
            let address = self.registers[usize::from((instruction >> 3) & 7)]
                .wrapping_add(if byte { offset } else { offset << 2 });
            let register = usize::from(instruction & 7);
            let width = if byte {
                AccessWidth::Byte
            } else {
                AccessWidth::Word
            };
            if load {
                self.registers[register] = self.read(bus, address, width, AccessKind::Read, now)?;
            } else {
                self.write(bus, address, width, self.registers[register], now)?;
            }
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xf000 == 0x8000 {
            let load = instruction & 0x0800 != 0;
            let address = self.registers[usize::from((instruction >> 3) & 7)]
                .wrapping_add(u32::from((instruction >> 6) & 0x1f) << 1);
            let register = usize::from(instruction & 7);
            if load {
                self.registers[register] =
                    self.read(bus, address, AccessWidth::HalfWord, AccessKind::Read, now)?;
            } else {
                self.write(
                    bus,
                    address,
                    AccessWidth::HalfWord,
                    self.registers[register],
                    now,
                )?;
            }
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xf000 == 0x9000 {
            let load = instruction & 0x0800 != 0;
            let register = usize::from((instruction >> 8) & 7);
            let address = self.registers[13].wrapping_add(u32::from(instruction & 0xff) << 2);
            if load {
                self.registers[register] =
                    self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
            } else {
                self.write(
                    bus,
                    address,
                    AccessWidth::Word,
                    self.registers[register],
                    now,
                )?;
            }
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xf000 == 0xa000 {
            let destination = usize::from((instruction >> 8) & 7);
            let base = if instruction & 0x0800 == 0 {
                pc.wrapping_add(4) & !3
            } else {
                self.registers[13]
            };
            self.registers[destination] = base.wrapping_add(u32::from(instruction & 0xff) << 2);
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xf000 == 0xb000 {
            return self.misc(instruction, bus, now, next);
        }

        if instruction & 0xf000 == 0xc000 {
            let load = instruction & 0x0800 != 0;
            let base = usize::from((instruction >> 8) & 7);
            let base_is_loaded = load && instruction & (1 << base) != 0;
            let mut address = self.registers[base];
            for register in 0..8 {
                if instruction & (1 << register) != 0 {
                    if load {
                        self.registers[register] =
                            self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                    } else {
                        self.write(
                            bus,
                            address,
                            AccessWidth::Word,
                            self.registers[register],
                            now,
                        )?;
                    }
                    address = address.wrapping_add(4);
                }
            }
            // Thumb LDMIA T1 suppresses writeback when the base is in the
            // register list. Compilers use this to unpack small structures,
            // for example `ldmia r0, {r0, r1}`. Writing back here would
            // destroy the value just loaded into the base register.
            if !base_is_loaded {
                self.registers[base] = address;
            }
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xf000 == 0xd000 {
            let condition = (instruction >> 8) & 0xf;
            if condition >= 0xe {
                return Err(self.fault(
                    CpuFaultKind::Unsupported,
                    format!("exception instruction {instruction:#06x} is not modeled"),
                ));
            }
            self.registers[15] = if self.condition(condition) {
                pc.wrapping_add(4)
                    .wrapping_add_signed(sign_extend(u32::from(instruction & 0xff) << 1, 9))
            } else {
                next
            };
            return Ok(StepReason::Advanced);
        }

        if instruction & 0xf800 == 0xe000 {
            self.registers[15] = pc
                .wrapping_add(4)
                .wrapping_add_signed(sign_extend(u32::from(instruction & 0x7ff) << 1, 12));
            return Ok(StepReason::Advanced);
        }

        if matches!(instruction & 0xf800, 0xe800 | 0xf000 | 0xf800) {
            return self.execute_wide(instruction, bus, now, pc, next);
        }

        Err(self.fault(
            CpuFaultKind::IllegalInstruction,
            format!(
                "Thumb instruction {instruction:#06x} is not implemented for {}",
                self.profile.name()
            ),
        ))
    }

    fn alu(&mut self, instruction: u16) {
        let op = (instruction >> 6) & 0xf;
        let right = self.registers[usize::from((instruction >> 3) & 7)];
        let destination = usize::from(instruction & 7);
        let left = self.registers[destination];
        let result = match op {
            0 => left & right,
            1 => left ^ right,
            2 => {
                let amount = right & 0xff;
                let result = left.checked_shl(amount).unwrap_or(0);
                if amount != 0 {
                    self.carry(amount <= 32 && left & (1 << (32 - amount.min(32))) != 0);
                }
                result
            }
            3 => {
                let amount = right & 0xff;
                let result = left.checked_shr(amount).unwrap_or(0);
                if amount != 0 {
                    self.carry(amount <= 32 && left & (1 << (amount.min(32) - 1)) != 0);
                }
                result
            }
            4 => {
                let amount = right & 0xff;
                let result = (left as i32)
                    .checked_shr(amount)
                    .unwrap_or(if left & N == 0 { 0 } else { -1 })
                    as u32;
                if amount != 0 {
                    self.carry(if amount >= 32 {
                        left & N != 0
                    } else {
                        left & (1 << (amount - 1)) != 0
                    });
                }
                result
            }
            7 => {
                let amount = right & 0xff;
                let result = left.rotate_right(amount & 0x1f);
                if amount != 0 {
                    self.carry(result & N != 0);
                }
                result
            }
            8 => {
                self.nz(left & right);
                return;
            }
            9 => {
                let result = 0_u32.wrapping_sub(right);
                self.sub_flags(0, right, result);
                self.registers[destination] = result;
                return;
            }
            10 => {
                self.sub_flags(left, right, left.wrapping_sub(right));
                return;
            }
            11 => {
                self.add_flags(left, right, left.wrapping_add(right));
                return;
            }
            12 => left | right,
            13 => left.wrapping_mul(right),
            14 => left & !right,
            15 => !right,
            5 | 6 => {
                let carry = u32::from(self.xpsr & C != 0);
                if op == 5 {
                    let result = left.wrapping_add(right).wrapping_add(carry);
                    let unsigned = u64::from(left) + u64::from(right) + u64::from(carry);
                    self.nz(result);
                    self.xpsr &= !(C | V);
                    if unsigned > u64::from(u32::MAX) {
                        self.xpsr |= C;
                    }
                    let signed =
                        i64::from(left as i32) + i64::from(right as i32) + i64::from(carry);
                    if signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX) {
                        self.xpsr |= V;
                    }
                    self.registers[destination] = result;
                    return;
                }
                let borrow = 1 - carry;
                let result = left.wrapping_sub(right).wrapping_sub(borrow);
                self.nz(result);
                self.xpsr &= !(C | V);
                if u64::from(left) >= u64::from(right) + u64::from(borrow) {
                    self.xpsr |= C;
                }
                let signed = i64::from(left as i32) - i64::from(right as i32) - i64::from(borrow);
                if signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX) {
                    self.xpsr |= V;
                }
                self.registers[destination] = result;
                return;
            }
            _ => unreachable!(),
        };
        self.registers[destination] = result;
        if !self.executing_in_it {
            self.nz(result);
        }
    }

    fn register_memory(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        let op = (instruction >> 9) & 7;
        let address = self.registers[usize::from((instruction >> 3) & 7)]
            .wrapping_add(self.registers[usize::from((instruction >> 6) & 7)]);
        let register = usize::from(instruction & 7);
        match op {
            0 => self.write(
                bus,
                address,
                AccessWidth::Word,
                self.registers[register],
                now,
            ),
            1 => self.write(
                bus,
                address,
                AccessWidth::HalfWord,
                self.registers[register],
                now,
            ),
            2 => self.write(
                bus,
                address,
                AccessWidth::Byte,
                self.registers[register],
                now,
            ),
            3 => {
                let value = self.read(bus, address, AccessWidth::Byte, AccessKind::Read, now)?;
                self.registers[register] = i32::from(value as u8 as i8) as u32;
                Ok(())
            }
            4..=6 => {
                let width = match op {
                    4 => AccessWidth::Word,
                    5 => AccessWidth::HalfWord,
                    _ => AccessWidth::Byte,
                };
                self.registers[register] = self.read(bus, address, width, AccessKind::Read, now)?;
                Ok(())
            }
            7 => {
                let value =
                    self.read(bus, address, AccessWidth::HalfWord, AccessKind::Read, now)?;
                self.registers[register] = i32::from(value as u16 as i16) as u32;
                Ok(())
            }
            _ => unreachable!(),
        }
    }

    fn misc(
        &mut self,
        instruction: u16,
        bus: &mut dyn Bus,
        now: SimTime,
        next: u32,
    ) -> Result<StepReason, CpuFault> {
        if instruction & 0xf500 == 0xb100 {
            let register = usize::from(instruction & 7);
            let branch_when_nonzero = instruction & 0x0800 != 0;
            let immediate = (u32::from((instruction >> 9) & 1) << 6)
                | (u32::from((instruction >> 3) & 0x1f) << 1);
            let is_zero = self.registers[register] == 0;
            self.registers[15] = if is_zero != branch_when_nonzero {
                self.registers[15].wrapping_add(4).wrapping_add(immediate)
            } else {
                next
            };
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xff00 == 0xbe00 {
            self.registers[15] = next;
            return Ok(StepReason::Breakpoint);
        }
        if instruction & 0xff00 == 0xbf00 {
            if instruction & 0x000f != 0 {
                self.it_state = instruction as u8;
                self.registers[15] = next;
                return Ok(StepReason::Advanced);
            }
            self.waiting = instruction == 0xbf30;
            self.registers[15] = next;
            return Ok(if self.waiting {
                StepReason::WaitForInterrupt
            } else {
                StepReason::Advanced
            });
        }
        if instruction & 0xffe8 == 0xb660 {
            if instruction & 2 != 0 {
                self.primask = instruction & 0x10 != 0;
            }
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xff00 == 0xb000 {
            let amount = u32::from(instruction & 0x7f) << 2;
            self.registers[13] = if instruction & 0x80 == 0 {
                self.registers[13].wrapping_add(amount)
            } else {
                self.registers[13].wrapping_sub(amount)
            };
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffc0 == 0xb200 {
            let destination = usize::from(instruction & 7);
            let source = self.registers[usize::from((instruction >> 3) & 7)];
            self.registers[destination] = i32::from(source as u16 as i16) as u32;
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffc0 == 0xb240 {
            let destination = usize::from(instruction & 7);
            let source = self.registers[usize::from((instruction >> 3) & 7)];
            self.registers[destination] = i32::from(source as u8 as i8) as u32;
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffc0 == 0xb280 {
            let destination = usize::from(instruction & 7);
            let source = self.registers[usize::from((instruction >> 3) & 7)];
            self.registers[destination] = source & 0xffff;
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffc0 == 0xb2c0 {
            let destination = usize::from(instruction & 7);
            let source = self.registers[usize::from((instruction >> 3) & 7)];
            self.registers[destination] = source & 0xff;
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffc0 == 0xba00 {
            let destination = usize::from(instruction & 7);
            let source = self.registers[usize::from((instruction >> 3) & 7)];
            self.registers[destination] = source.swap_bytes();
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffc0 == 0xba40 {
            let destination = usize::from(instruction & 7);
            let source = self.registers[usize::from((instruction >> 3) & 7)];
            self.registers[destination] =
                ((source & 0x00ff_00ff) << 8) | ((source & 0xff00_ff00) >> 8);
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xffc0 == 0xbac0 {
            let destination = usize::from(instruction & 7);
            let source = self.registers[usize::from((instruction >> 3) & 7)];
            let reversed = (source as u16).swap_bytes();
            self.registers[destination] = i32::from(reversed as i16) as u32;
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfe00 == 0xb400 {
            let include_lr = instruction & 0x100 != 0;
            let count = (instruction & 0xff).count_ones() + u32::from(include_lr);
            let mut address = self.registers[13].wrapping_sub(count * 4);
            self.registers[13] = address;
            for register in 0..8 {
                if instruction & (1 << register) != 0 {
                    self.write(
                        bus,
                        address,
                        AccessWidth::Word,
                        self.registers[register],
                        now,
                    )?;
                    address = address.wrapping_add(4);
                }
            }
            if include_lr {
                self.write(bus, address, AccessWidth::Word, self.registers[14], now)?;
            }
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        if instruction & 0xfe00 == 0xbc00 {
            let include_pc = instruction & 0x100 != 0;
            let mut address = self.registers[13];
            for register in 0..8 {
                if instruction & (1 << register) != 0 {
                    self.registers[register] =
                        self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                    address = address.wrapping_add(4);
                }
            }
            if include_pc {
                let target = self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                address = address.wrapping_add(4);
                self.registers[13] = address;
                self.branch_exchange(target, bus, now)?;
                return Ok(StepReason::Advanced);
            }
            self.registers[13] = address;
            self.registers[15] = next;
            return Ok(StepReason::Advanced);
        }
        Err(self.fault(
            CpuFaultKind::IllegalInstruction,
            format!("miscellaneous instruction {instruction:#06x} is not implemented"),
        ))
    }

    fn take_interrupt(
        &mut self,
        exception: ArmException,
        bus: &mut dyn Bus,
        now: SimTime,
    ) -> Result<(), CpuFault> {
        let stack = self.registers[13].wrapping_sub(32);
        for (index, value) in [
            self.registers[0],
            self.registers[1],
            self.registers[2],
            self.registers[3],
            self.registers[12],
            self.registers[14],
            self.registers[15] | 1,
            self.xpsr,
        ]
        .into_iter()
        .enumerate()
        {
            self.write(
                bus,
                stack.wrapping_add(u32::try_from(index).expect("small index fits u32") * 4),
                AccessWidth::Word,
                value,
                now,
            )?;
        }
        self.registers[13] = stack;
        self.registers[14] = 0xffff_fff9;
        self.interrupts.remove(&exception);
        self.active_interrupt = Some(exception);
        let exception_number = exception.exception_number();
        self.xpsr = (self.xpsr & !0x1ff) | u32::from(exception_number);
        let vector = self
            .vector_base
            .wrapping_add(u32::from(exception_number).wrapping_mul(4));
        let handler = self.read(bus, vector, AccessWidth::Word, AccessKind::Read, now)?;
        self.branch_exchange(handler, bus, now)?;
        self.waiting = false;
        Ok(())
    }
}

impl Cpu for ArmCpu {
    fn architecture(&self) -> Architecture {
        Architecture::ArmM
    }

    fn reset(&mut self, _kind: ResetKind, bus: &mut dyn Bus) -> Result<(), CpuFault> {
        self.registers = [0; 16];
        self.xpsr = 1 << 24;
        self.vector_base = 0;
        self.primask = false;
        self.it_state = 0;
        self.executing_in_it = false;
        self.waiting = false;
        self.halted = false;
        self.interrupts.clear();
        self.active_interrupt = None;
        self.exclusive_address = None;
        self.fpu_registers = [0; 32];
        let stack = self.read(bus, 0, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)?;
        let entry = self.read(bus, 4, AccessWidth::Word, AccessKind::Read, SimTime::ZERO)?;
        self.set_direct_state(stack, entry)
    }

    fn step(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<StepOutcome, CpuFault> {
        if self.halted {
            return Ok(StepOutcome {
                elapsed: SimDuration::ZERO,
                reason: StepReason::Halted,
            });
        }
        if !self.primask
            && self.active_interrupt.is_none()
            && let Some(exception) = self.interrupts.iter().next().copied()
        {
            self.take_interrupt(exception, bus, now)?;
            return Ok(StepOutcome::advanced(SimDuration::TICK));
        }
        if self.waiting {
            return Ok(StepOutcome {
                elapsed: SimDuration::TICK,
                reason: StepReason::WaitForInterrupt,
            });
        }
        let instruction = self.read(
            bus,
            self.registers[15],
            AccessWidth::HalfWord,
            AccessKind::Execute,
            now,
        )? as u16;
        let executing_in_it = self.it_state != 0;
        if executing_in_it {
            let condition = u16::from(self.it_state >> 4);
            let execute = self.condition(condition);
            if self.it_state & 7 == 0 {
                self.it_state = 0;
            } else {
                self.it_state = (self.it_state & 0xe0) | ((self.it_state << 1) & 0x1f);
            }
            if !execute {
                let width = if matches!(instruction & 0xf800, 0xe800 | 0xf000 | 0xf800) {
                    4
                } else {
                    2
                };
                self.registers[15] = self.registers[15].wrapping_add(width);
                return Ok(StepOutcome::advanced(SimDuration::TICK));
            }
        }
        self.executing_in_it = executing_in_it;
        let result = self.execute(instruction, bus, now);
        self.executing_in_it = false;
        Ok(StepOutcome {
            elapsed: SimDuration::TICK,
            reason: result?,
        })
    }

    fn set_interrupt(&mut self, line: u16, asserted: bool) -> Result<(), CpuFault> {
        if line >= 240 {
            return Err(self.fault(
                CpuFaultKind::Unsupported,
                format!("external interrupt {line} exceeds the modeled NVIC range"),
            ));
        }
        if asserted {
            self.interrupts.insert(ArmException::External(line));
            self.waiting = false;
        } else {
            self.interrupts.remove(&ArmException::External(line));
        }
        Ok(())
    }

    fn snapshot(&self) -> CpuSnapshot {
        let mut registers = (0..16)
            .map(|index| RegisterValue {
                name: match index {
                    13 => "sp".to_owned(),
                    14 => "lr".to_owned(),
                    15 => "pc".to_owned(),
                    _ => format!("r{index}"),
                },
                value: u64::from(self.registers[index]),
                bits: 32,
            })
            .collect::<Vec<_>>();
        registers.push(RegisterValue {
            name: "xpsr".to_owned(),
            value: u64::from(self.xpsr),
            bits: 32,
        });
        CpuSnapshot {
            architecture: Architecture::ArmM,
            pc: u64::from(self.registers[15]),
            registers,
            waiting: self.waiting,
            halted: self.halted,
        }
    }
}

fn sign_extend(value: u32, bits: u32) -> i32 {
    ((value << (32 - bits)) as i32) >> (32 - bits)
}

fn thumb_expand_immediate(encoded: u32) -> u32 {
    let byte = encoded & 0xff;
    if encoded & 0xc00 == 0 {
        match (encoded >> 8) & 3 {
            0 => byte,
            1 => byte | (byte << 16),
            2 => (byte << 8) | (byte << 24),
            3 => byte | (byte << 8) | (byte << 16) | (byte << 24),
            _ => unreachable!(),
        }
    } else {
        (0x80 | (encoded & 0x7f)).rotate_right((encoded >> 7) & 0x1f)
    }
}

#[cfg(test)]
mod tests;
