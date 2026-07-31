//! Interpreted Arm M-profile CPU implementation.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use renvo_core::{
    AccessKind, AccessWidth, Architecture, Bus, Cpu, CpuFault, CpuFaultKind, CpuSnapshot,
    RegisterValue, ResetKind, SimDuration, SimTime, StepOutcome, StepReason,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const N: u32 = 1 << 31;
const Z: u32 = 1 << 30;
const C: u32 = 1 << 29;
const V: u32 = 1 << 28;

/// Compiler-facing M-profile generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArmProfile {
    /// Armv6-M Cortex-M0+ used by RP2040.
    CortexM0Plus,
    /// Non-secure Armv8-M Mainline Cortex-M33 used by RP2350.
    CortexM33,
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
    interrupts: BTreeSet<u16>,
    active_interrupt: Option<u16>,
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
            let second =
                self.read(bus, next, AccessWidth::HalfWord, AccessKind::Execute, now)? as u16;
            if instruction == 0xee10 && second == 0x0430 {
                // RP2350's optional double-precision coprocessor uses this MRC as RCMP to clear
                // its engaged flag during per-core startup. The functional arithmetic model has
                // no persistent engaged state.
                self.registers[0] = 0;
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if self.profile == ArmProfile::CortexM33
                && instruction & 0xff10 == 0xee00
                && second & 0x0f10 == 0x0010
                && instruction & 0xf == 0
            {
                // RP2350 GPIO coprocessor MCR. The coprocessor is an
                // architectural alias for the SIO GPIO registers; route the
                // low GPIO bank through the bus so the device model remains
                // the single owner of signal state and VCD changes.
                let operation = (instruction >> 5) & 7;
                let source = usize::from((second >> 12) & 0xf);
                let bank = second & 0xf;
                let offset = match (bank, operation) {
                    (0, 0) => Some(0x10),
                    (0, 1) => Some(0x28),
                    (0, 2) => Some(0x18),
                    (0, 3) => Some(0x20),
                    (4, 0) => Some(0x30),
                    (4, 1) => Some(0x48),
                    (4, 2) => Some(0x38),
                    (4, 3) => Some(0x40),
                    (0, 5) => Some(0x28),
                    (0, 6) => Some(0x18),
                    (0, 7) => Some(0x20),
                    (4, 5) => Some(0x48),
                    (4, 6) => Some(0x38),
                    (4, 7) => Some(0x40),
                    _ => None,
                };
                if let Some(offset) = offset {
                    let value = if operation >= 5 {
                        1_u32.checked_shl(self.registers[source]).unwrap_or(0)
                    } else {
                        self.registers[source]
                    };
                    if value != 0 || operation < 5 {
                        self.write(bus, 0xd000_0000 + offset, AccessWidth::Word, value, now)?;
                    }
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if self.profile == ArmProfile::CortexM33
                && instruction & 0xff10 == 0xee10
                && second & 0x0f10 == 0x0010
                && instruction & 0xf == 0
                && (instruction >> 5) & 7 == 0
            {
                // RP2350 GPIO coprocessor MRC.
                let destination = usize::from((second >> 12) & 0xf);
                let offset = match second & 0xf {
                    0 => Some(0x10),
                    4 => Some(0x30),
                    8 => Some(0x04),
                    1 | 5 | 9 => None,
                    _ => None,
                };
                self.registers[destination] = if let Some(offset) = offset {
                    self.read(
                        bus,
                        0xd000_0000 + offset,
                        AccessWidth::Word,
                        AccessKind::Read,
                        now,
                    )?
                } else {
                    0
                };
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if self.profile == ArmProfile::CortexM33
                && instruction & 0xfff0 == 0xec40
                && second & 0x0f00 == 0
            {
                // RP2350 GPIO coprocessor MCRR. The implemented low bank
                // covers all 30 package GPIOs used by Pico 2; high-bank
                // values concern QSPI and USB pads outside this signal model.
                let first = self.registers[usize::from((second >> 12) & 0xf)];
                let second_value = self.registers[usize::from(instruction & 0xf)];
                let operation = (second >> 4) & 0xf;
                let bank = second & 0xf;
                let register_offsets = match bank {
                    0 => Some([0x10, 0x28, 0x18, 0x20]),
                    4 => Some([0x30, 0x48, 0x38, 0x40]),
                    _ => None,
                };
                if let Some(offsets) = register_offsets {
                    match operation {
                        0..=3 => self.write(
                            bus,
                            0xd000_0000 + offsets[usize::from(operation)],
                            AccessWidth::Word,
                            first,
                            now,
                        )?,
                        4..=7 => {
                            let mask = 1_u32.checked_shl(first).unwrap_or(0);
                            let selected = if operation == 4 {
                                if second_value == 0 {
                                    offsets[3]
                                } else {
                                    offsets[2]
                                }
                            } else {
                                offsets[usize::from(operation - 4)]
                            };
                            if mask != 0 && (operation == 4 || second_value != 0) {
                                self.write(
                                    bus,
                                    0xd000_0000 + selected,
                                    AccessWidth::Word,
                                    mask,
                                    now,
                                )?;
                            }
                        }
                        8..=11 if second_value == 0 => self.write(
                            bus,
                            0xd000_0000 + offsets[usize::from(operation - 8)],
                            AccessWidth::Word,
                            first,
                            now,
                        )?,
                        _ => {}
                    }
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if self.profile == ArmProfile::CortexM33
                && instruction & 0xfff0 == 0xec50
                && second & 0x0f00 == 0
                && (second >> 4) & 0xf == 0
            {
                // RP2350 GPIO coprocessor MRRC.
                let low_register = usize::from((second >> 12) & 0xf);
                let high_register = usize::from(instruction & 0xf);
                let offset = match second & 0xf {
                    0 => Some(0x10),
                    4 => Some(0x30),
                    8 => Some(0x04),
                    _ => None,
                };
                self.registers[low_register] = if let Some(offset) = offset {
                    self.read(
                        bus,
                        0xd000_0000 + offset,
                        AccessWidth::Word,
                        AccessKind::Read,
                        now,
                    )?
                } else {
                    0
                };
                self.registers[high_register] = 0;
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xffe0 == 0xee00 && second & 0x0f7f == 0x0a10 {
                let core_register = usize::from((second >> 12) & 0xf);
                let single_register =
                    usize::from(instruction & 0xf) * 2 + usize::from((second >> 7) & 1);
                if instruction & 0x10 == 0 {
                    self.set_single_register(single_register, self.registers[core_register]);
                } else {
                    self.registers[core_register] = self.single_register(single_register);
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xffbf == 0xeeb8 && second & 0x0f50 == 0x0a40 {
                let destination =
                    usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
                let source = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
                let integer = self.single_register(source);
                let value = if second & 0x80 != 0 {
                    integer as i32 as f32
                } else {
                    integer as f32
                };
                self.set_single_register(destination, value.to_bits());
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xffbe == 0xeebc && second & 0x0f50 == 0x0a40 {
                let destination =
                    usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
                let source = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
                let value = f32::from_bits(self.single_register(source));
                let integer = if instruction & 1 != 0 {
                    (value as i32) as u32
                } else {
                    value as u32
                };
                self.set_single_register(destination, integer);
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xffb0 == 0xeeb0 && second & 0x0ff0 == 0x0a00 {
                let destination =
                    usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
                let immediate = ((u32::from(instruction & 0xf)) << 4) | u32::from(second & 0xf);
                let bit6 = immediate & 0x40 != 0;
                let exponent = (u32::from(!bit6) << 7)
                    | ((if bit6 { 0x1f } else { 0 }) << 2)
                    | ((immediate >> 4) & 3);
                let bits =
                    ((immediate & 0x80) << 24) | (exponent << 23) | ((immediate & 0xf) << 19);
                self.set_single_register(destination, bits);
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xffbf == 0xeeb0 && second & 0x0f50 == 0x0a40 {
                let destination =
                    usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
                let source = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
                let mut value = self.single_register(source);
                if second & 0x80 != 0 {
                    value &= 0x7fff_ffff;
                }
                self.set_single_register(destination, value);
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xffbf == 0xeeb1 && second & 0x0f50 == 0x0a40 {
                let destination =
                    usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
                let source = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
                self.set_single_register(destination, self.single_register(source) ^ N);
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(instruction & 0xffb0, 0xee20 | 0xee30 | 0xee80) && second & 0x0f10 == 0x0a00
            {
                let destination =
                    usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
                let left_register =
                    usize::from(instruction & 0xf) * 2 + usize::from((second >> 7) & 1);
                let right_register = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
                let left = f32::from_bits(self.single_register(left_register));
                let right = f32::from_bits(self.single_register(right_register));
                let value = match instruction & 0xffb0 {
                    0xee20 => left * right,
                    0xee30 if second & 0x40 == 0 => left + right,
                    0xee30 => left - right,
                    0xee80 => left / right,
                    _ => unreachable!(),
                };
                self.set_single_register(destination, value.to_bits());
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xffb0 == 0xeea0 && second & 0x0f10 == 0x0a00 {
                let destination =
                    usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
                let left_register =
                    usize::from(instruction & 0xf) * 2 + usize::from((second >> 7) & 1);
                let right_register = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
                let accumulator = f32::from_bits(self.single_register(destination));
                let left = f32::from_bits(self.single_register(left_register));
                let right = f32::from_bits(self.single_register(right_register));
                let value = if second & 0x40 == 0 {
                    left.mul_add(right, accumulator)
                } else {
                    (-left).mul_add(right, accumulator)
                };
                self.set_single_register(destination, value.to_bits());
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xffbf == 0xeeb5 && second & 0x0f3f == 0x0a00 {
                let left_register =
                    usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
                let left = f32::from_bits(self.single_register(left_register));
                self.xpsr &= !(N | Z | C | V);
                if left.is_nan() {
                    self.xpsr |= C | V;
                } else if left == 0.0 {
                    self.xpsr |= Z | C;
                } else if left < 0.0 {
                    self.xpsr |= N;
                } else {
                    self.xpsr |= C;
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xffbf == 0xeeb4 && second & 0x0f50 == 0x0a40 {
                let left_register =
                    usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
                let right_register = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
                let left = f32::from_bits(self.single_register(left_register));
                let right = f32::from_bits(self.single_register(right_register));
                self.xpsr &= !(N | Z | C | V);
                if left.is_nan() || right.is_nan() {
                    self.xpsr |= C | V;
                } else if left == right {
                    self.xpsr |= Z | C;
                } else if left < right {
                    self.xpsr |= N;
                } else {
                    self.xpsr |= C;
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction == 0xeef1 && second == 0xfa10 {
                // VMRS APSR_nzcv,FPSCR. Functional VFP comparisons materialize their flags
                // directly in APSR, so this architectural transfer has no additional work.
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xff00 == 0xfe00 && second & 0x0f10 == 0x0a00 {
                let destination =
                    usize::from((second >> 12) & 0xf) * 2 + usize::from((instruction >> 6) & 1);
                let true_register =
                    usize::from(instruction & 0xf) * 2 + usize::from((second >> 7) & 1);
                let false_register = usize::from(second & 0xf) * 2 + usize::from((second >> 5) & 1);
                let condition = match (instruction >> 4) & 3 {
                    0 => 0,  // EQ
                    1 => 6,  // VS
                    2 => 10, // GE
                    3 => 12, // GT
                    _ => unreachable!(),
                };
                let source = if self.condition(condition) {
                    true_register
                } else {
                    false_register
                };
                self.set_single_register(destination, self.single_register(source));
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xff00 == 0xed00
                && second & 0x0f00 == 0x0a00
                && !matches!(instruction, 0xed2d | 0xecbd)
            {
                let base_register = usize::from(instruction & 0xf);
                let base = if base_register == 15 {
                    pc.wrapping_add(4) & !3
                } else {
                    self.registers[base_register]
                };
                let offset = u32::from(second & 0xff) << 2;
                let address = if instruction & 0x0080 != 0 {
                    base.wrapping_add(offset)
                } else {
                    base.wrapping_sub(offset)
                };
                let single_register =
                    (usize::from((second >> 12) & 0xf) << 1) | usize::from((instruction >> 6) & 1);
                let double_register = single_register / 2;
                let high_half = single_register & 1 != 0;
                if instruction & 0x10 != 0 {
                    let value =
                        self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                    let existing = self.fpu_registers[double_register];
                    self.fpu_registers[double_register] = if high_half {
                        (existing & 0x0000_0000_ffff_ffff) | (u64::from(value) << 32)
                    } else {
                        (existing & 0xffff_ffff_0000_0000) | u64::from(value)
                    };
                } else {
                    let pair = self.fpu_registers[double_register];
                    let value = if high_half {
                        (pair >> 32) as u32
                    } else {
                        pair as u32
                    };
                    self.write(bus, address, AccessWidth::Word, value, now)?;
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xff00 == 0xed00
                && second & 0x0f00 == 0x0b00
                && !matches!(instruction, 0xed2d | 0xecbd)
            {
                let base = self.registers[usize::from(instruction & 0xf)];
                let offset = u32::from(second & 0xff) << 2;
                let address = if instruction & 0x0080 != 0 {
                    base.wrapping_add(offset)
                } else {
                    base.wrapping_sub(offset)
                };
                let register =
                    usize::from((second >> 12) & 0xf) | (usize::from((instruction >> 6) & 1) << 4);
                if instruction & 0x10 != 0 {
                    let low = self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                    let high = self.read(
                        bus,
                        address.wrapping_add(4),
                        AccessWidth::Word,
                        AccessKind::Read,
                        now,
                    )?;
                    self.fpu_registers[register] = u64::from(low) | (u64::from(high) << 32);
                } else {
                    let value = self.fpu_registers[register];
                    self.write(bus, address, AccessWidth::Word, value as u32, now)?;
                    self.write(
                        bus,
                        address.wrapping_add(4),
                        AccessWidth::Word,
                        (value >> 32) as u32,
                        now,
                    )?;
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(instruction, 0xed2d | 0xecbd) && second & 0x0f00 == 0x0b00 {
                let load = instruction == 0xecbd;
                let first_register =
                    usize::from((second >> 12) & 0xf) | (usize::from((instruction >> 6) & 1) << 4);
                let words = u32::from(second & 0xff);
                if words == 0 || words & 1 != 0 {
                    return Err(self.fault(
                        CpuFaultKind::IllegalInstruction,
                        "VPOP/VPUSH double-register list has invalid size",
                    ));
                }
                let count = usize::try_from(words / 2).expect("VFP list count fits usize");
                if first_register + count > self.fpu_registers.len() {
                    return Err(self.fault(
                        CpuFaultKind::IllegalInstruction,
                        "VPOP/VPUSH register list exceeds D31",
                    ));
                }
                let mut address = if load {
                    self.registers[13]
                } else {
                    self.registers[13].wrapping_sub(words * 4)
                };
                for register in first_register..first_register + count {
                    if load {
                        let low =
                            self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                        let high = self.read(
                            bus,
                            address.wrapping_add(4),
                            AccessWidth::Word,
                            AccessKind::Read,
                            now,
                        )?;
                        self.fpu_registers[register] = u64::from(low) | (u64::from(high) << 32);
                    } else {
                        let value = self.fpu_registers[register];
                        self.write(bus, address, AccessWidth::Word, value as u32, now)?;
                        self.write(
                            bus,
                            address.wrapping_add(4),
                            AccessWidth::Word,
                            (value >> 32) as u32,
                            now,
                        )?;
                    }
                    address = address.wrapping_add(8);
                }
                self.registers[13] = if load {
                    self.registers[13].wrapping_add(words * 4)
                } else {
                    self.registers[13].wrapping_sub(words * 4)
                };
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xe840 && second & 0xf0f0 == 0xf000 {
                // Armv8-M Test Target reports the security/MPU attribution of an address.
                // Renvo currently has one non-secure functional address space, represented by
                // an all-zero TT response. SDK code uses bit 22 of this result to select its
                // boot-ROM lookup mask.
                let destination = usize::from((second >> 8) & 0xf);
                self.registers[destination] = 0;
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xe8c0 && second & 0x0fff == 0x0f8f {
                // Store-release byte has ordinary byte-store data effects in the
                // single-threaded interpreter; all earlier accesses are already complete.
                let address = self.registers[usize::from(instruction & 0xf)];
                let source = usize::from((second >> 12) & 0xf);
                self.write(bus, address, AccessWidth::Byte, self.registers[source], now)?;
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xe8d0 && second & 0x0fff == 0x0fcf {
                let address = self.registers[usize::from(instruction & 0xf)];
                let destination = usize::from((second >> 12) & 0xf);
                self.registers[destination] =
                    self.read(bus, address, AccessWidth::Byte, AccessKind::Read, now)?;
                self.exclusive_address = Some(address);
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xe8d0 && second & 0xffe0 == 0xf000 {
                let base_register = usize::from(instruction & 0xf);
                let index_register = usize::from(second & 0xf);
                let halfword = second & 0x10 != 0;
                let base = if base_register == 15 {
                    pc.wrapping_add(4)
                } else {
                    self.registers[base_register]
                };
                let index = self.registers[index_register] << u32::from(halfword);
                let width = if halfword {
                    AccessWidth::HalfWord
                } else {
                    AccessWidth::Byte
                };
                let displacement =
                    self.read(bus, base.wrapping_add(index), width, AccessKind::Read, now)?;
                self.registers[15] = pc
                    .wrapping_add(4)
                    .wrapping_add(displacement.wrapping_mul(2));
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xe8c0 && second & 0x0ff0 == 0x0f40 {
                let address = self.registers[usize::from(instruction & 0xf)];
                let value_register = usize::from((second >> 12) & 0xf);
                let status_register = usize::from(second & 0xf);
                if self.exclusive_address == Some(address) {
                    self.write(
                        bus,
                        address,
                        AccessWidth::Byte,
                        self.registers[value_register],
                        now,
                    )?;
                    self.registers[status_register] = 0;
                } else {
                    self.registers[status_register] = 1;
                }
                self.exclusive_address = None;
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfe40 == 0xe840 && instruction & 0x0120 != 0 {
                let base_register = usize::from(instruction & 0xf);
                let first_register = usize::from((second >> 12) & 0xf);
                let second_register = usize::from((second >> 8) & 0xf);
                let offset = u32::from(second & 0xff) << 2;
                let base = self.registers[base_register];
                let adjusted = if instruction & 0x0080 != 0 {
                    base.wrapping_add(offset)
                } else {
                    base.wrapping_sub(offset)
                };
                let address = if instruction & 0x0100 != 0 {
                    adjusted
                } else {
                    base
                };
                if instruction & 0x10 != 0 {
                    self.registers[first_register] =
                        self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                    self.registers[second_register] = self.read(
                        bus,
                        address.wrapping_add(4),
                        AccessWidth::Word,
                        AccessKind::Read,
                        now,
                    )?;
                } else {
                    self.write(
                        bus,
                        address,
                        AccessWidth::Word,
                        self.registers[first_register],
                        now,
                    )?;
                    self.write(
                        bus,
                        address.wrapping_add(4),
                        AccessWidth::Word,
                        self.registers[second_register],
                        now,
                    )?;
                }
                if instruction & 0x20 != 0 {
                    self.registers[base_register] = adjusted;
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(instruction & 0xfff0, 0xfb90 | 0xfbb0) && second & 0xf0f0 == 0xf0f0 {
                let numerator = self.registers[usize::from(instruction & 0xf)];
                let denominator = self.registers[usize::from(second & 0xf)];
                let destination = usize::from((second >> 8) & 0xf);
                self.registers[destination] = if denominator == 0 {
                    0
                } else if instruction & 0x20 != 0 {
                    numerator / denominator
                } else {
                    (numerator as i32).wrapping_div(denominator as i32) as u32
                };
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xfab0 && second & 0xf0f0 == 0xf080 {
                let source = self.registers[usize::from(second & 0xf)];
                let destination = usize::from((second >> 8) & 0xf);
                self.registers[destination] = source.leading_zeros();
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xfa90 && second & 0xf0f0 == 0xf0a0 {
                let source = self.registers[usize::from(second & 0xf)];
                let destination = usize::from((second >> 8) & 0xf);
                self.registers[destination] = source.reverse_bits();
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xff80 == 0xfa00 && second & 0xf0f0 == 0xf000 {
                let source = self.registers[usize::from(instruction & 0xf)];
                let amount = self.registers[usize::from(second & 0xf)] & 0xff;
                let destination = usize::from((second >> 8) & 0xf);
                self.registers[destination] = match (instruction >> 5) & 3 {
                    0 => source.checked_shl(amount).unwrap_or(0),
                    1 => source.checked_shr(amount).unwrap_or(0),
                    2 => (source as i32)
                        .checked_shr(amount)
                        .unwrap_or(if source & N == 0 { 0 } else { -1 })
                        as u32,
                    3 => source.rotate_right(amount & 31),
                    _ => unreachable!(),
                };
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(instruction, 0xfa0f | 0xfa1f | 0xfa4f | 0xfa5f) && second & 0xf0c0 == 0xf080
            {
                let source = self.registers[usize::from(second & 0xf)]
                    .rotate_right(u32::from((second >> 4) & 3) * 8);
                let destination = usize::from((second >> 8) & 0xf);
                self.registers[destination] = match instruction {
                    0xfa0f => i32::from(source as u16 as i16) as u32,
                    0xfa1f => source & 0xffff,
                    0xfa4f => i32::from(source as u8 as i8) as u32,
                    0xfa5f => source & 0xff,
                    _ => unreachable!(),
                };
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(instruction & 0xfff0, 0xfa00 | 0xfa10 | 0xfa40 | 0xfa50)
                && second & 0xf0c0 == 0xf080
            {
                let operation = instruction & 0xfff0;
                let left = self.registers[usize::from(instruction & 0xf)];
                let source = self.registers[usize::from(second & 0xf)]
                    .rotate_right(u32::from((second >> 4) & 3) * 8);
                let extended = match operation {
                    0xfa00 => i32::from(source as u16 as i16) as u32,
                    0xfa10 => source & 0xffff,
                    0xfa40 => i32::from(source as u8 as i8) as u32,
                    0xfa50 => source & 0xff,
                    _ => unreachable!(),
                };
                let destination = usize::from((second >> 8) & 0xf);
                self.registers[destination] = left.wrapping_add(extended);
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xfb00 && second & 0x00e0 == 0 {
                let left = self.registers[usize::from(instruction & 0xf)];
                let right = self.registers[usize::from(second & 0xf)];
                let accumulator = usize::from((second >> 12) & 0xf);
                let destination = usize::from((second >> 8) & 0xf);
                let product = left.wrapping_mul(right);
                self.registers[destination] = if second & 0x10 != 0 {
                    self.registers[accumulator].wrapping_sub(product)
                } else if accumulator == 15 {
                    product
                } else {
                    product.wrapping_add(self.registers[accumulator])
                };
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xfb10 && second & 0x00c0 == 0 {
                let left_word = self.registers[usize::from(instruction & 0xf)];
                let right_word = self.registers[usize::from(second & 0xf)];
                let left = if second & 0x20 == 0 {
                    left_word as u16 as i16
                } else {
                    (left_word >> 16) as u16 as i16
                };
                let right = if second & 0x10 == 0 {
                    right_word as u16 as i16
                } else {
                    (right_word >> 16) as u16 as i16
                };
                let accumulator = usize::from((second >> 12) & 0xf);
                let destination = usize::from((second >> 8) & 0xf);
                let product = i32::from(left).wrapping_mul(i32::from(right)) as u32;
                self.registers[destination] = if accumulator == 15 {
                    product
                } else {
                    product.wrapping_add(self.registers[accumulator])
                };
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(instruction & 0xfff0, 0xfb80 | 0xfba0) && second & 0x00f0 == 0 {
                let left = self.registers[usize::from(instruction & 0xf)];
                let right = self.registers[usize::from(second & 0xf)];
                let product = if instruction & 0x20 != 0 {
                    u64::from(left) * u64::from(right)
                } else {
                    ((i64::from(left as i32) * i64::from(right as i32)) as u64) & u64::MAX
                };
                let low = usize::from((second >> 12) & 0xf);
                let high = usize::from((second >> 8) & 0xf);
                self.registers[low] = product as u32;
                self.registers[high] = (product >> 32) as u32;
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(instruction & 0xfff0, 0xfbc0 | 0xfbe0) && second & 0x00f0 == 0 {
                let left = self.registers[usize::from(instruction & 0xf)];
                let right = self.registers[usize::from(second & 0xf)];
                let low = usize::from((second >> 12) & 0xf);
                let high = usize::from((second >> 8) & 0xf);
                let accumulator =
                    (u64::from(self.registers[high]) << 32) | u64::from(self.registers[low]);
                let product = if instruction & 0x20 != 0 {
                    u64::from(left) * u64::from(right)
                } else {
                    (i64::from(left as i32) * i64::from(right as i32)) as u64
                };
                let result = accumulator.wrapping_add(product);
                self.registers[low] = result as u32;
                self.registers[high] = (result >> 32) as u32;
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(instruction & 0xfff0, 0xf340 | 0xf3c0) && second & 0x8000 == 0 {
                let source = self.registers[usize::from(instruction & 0xf)];
                let destination = usize::from((second >> 8) & 0xf);
                let least_significant = u32::from(((second >> 12) & 7) << 2 | ((second >> 6) & 3));
                let width = u32::from(second & 0x1f) + 1;
                let mask = if width == 32 {
                    u32::MAX
                } else {
                    (1_u32 << width) - 1
                };
                let extracted = (source >> least_significant) & mask;
                self.registers[destination] = if instruction & 0x0080 != 0 {
                    extracted
                } else if extracted & (1 << (width - 1)) != 0 {
                    extracted | !mask
                } else {
                    extracted
                };
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xf360 && second & 0x8000 == 0 {
                let source_register = usize::from(instruction & 0xf);
                let destination = usize::from((second >> 8) & 0xf);
                let least_significant = u32::from(((second >> 12) & 7) << 2 | ((second >> 6) & 3));
                let most_significant = u32::from(second & 0x1f);
                if most_significant < least_significant {
                    return Err(self.fault(
                        CpuFaultKind::IllegalInstruction,
                        "BFI most-significant bit precedes least-significant bit",
                    ));
                }
                let width = most_significant - least_significant + 1;
                let field_mask = if width == 32 {
                    u32::MAX
                } else {
                    ((1_u32 << width) - 1) << least_significant
                };
                let source = if source_register == 15 {
                    0
                } else {
                    self.registers[source_register]
                };
                self.registers[destination] = (self.registers[destination] & !field_mask)
                    | ((source << least_significant) & field_mask);
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            let shifted_arithmetic_operation = instruction & 0xffe0;
            if matches!(
                shifted_arithmetic_operation,
                0xeb00 | 0xeb40 | 0xeb60 | 0xeba0 | 0xebc0
            ) {
                let left = self.registers[usize::from(instruction & 0xf)];
                let right_register = usize::from(second & 0xf);
                let shift_amount = u32::from(((second >> 12) & 7) << 2 | ((second >> 6) & 3));
                let right = match (second >> 4) & 3 {
                    0 => self.registers[right_register].wrapping_shl(shift_amount),
                    1 => self.registers[right_register]
                        .checked_shr(if shift_amount == 0 { 32 } else { shift_amount })
                        .unwrap_or(0),
                    2 => {
                        ((self.registers[right_register] as i32)
                            .checked_shr(if shift_amount == 0 { 32 } else { shift_amount })
                            .unwrap_or(if self.registers[right_register] & N == 0 {
                                0
                            } else {
                                -1
                            })) as u32
                    }
                    3 => self.registers[right_register].rotate_right(shift_amount),
                    _ => unreachable!(),
                };
                let carry = u32::from(self.xpsr & C != 0);
                let result = match shifted_arithmetic_operation {
                    0xeb00 => left.wrapping_add(right),
                    0xeb40 => left.wrapping_add(right).wrapping_add(carry),
                    0xeb60 => left.wrapping_sub(right).wrapping_sub(1 - carry),
                    0xeba0 => left.wrapping_sub(right),
                    0xebc0 => right.wrapping_sub(left),
                    _ => unreachable!(),
                };
                let destination = usize::from((second >> 8) & 0xf);
                if destination != 15 {
                    self.registers[destination] = result;
                }
                if instruction & 0x10 != 0 {
                    match shifted_arithmetic_operation {
                        0xeb00 => self.add_flags(left, right, result),
                        0xeba0 => self.sub_flags(left, right, result),
                        0xebc0 => self.sub_flags(right, left, result),
                        0xeb40 | 0xeb60 => self.nz(result),
                        _ => unreachable!(),
                    }
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            let shifted_logical_operation = (instruction >> 5) & 0xf;
            if instruction & 0xfe00 == 0xea00 && shifted_logical_operation <= 4 {
                let source = usize::from(instruction & 0xf);
                let destination = usize::from((second >> 8) & 0xf);
                let right_register = usize::from(second & 0xf);
                let shift_amount = u32::from(((second >> 12) & 7) << 2 | ((second >> 6) & 3));
                let right = match (second >> 4) & 3 {
                    0 => self.registers[right_register].wrapping_shl(shift_amount),
                    1 => self.registers[right_register]
                        .checked_shr(if shift_amount == 0 { 32 } else { shift_amount })
                        .unwrap_or(0),
                    2 => {
                        ((self.registers[right_register] as i32)
                            .checked_shr(if shift_amount == 0 { 32 } else { shift_amount })
                            .unwrap_or(if self.registers[right_register] & N == 0 {
                                0
                            } else {
                                -1
                            })) as u32
                    }
                    3 if shift_amount == 0 => {
                        (u32::from(self.xpsr & C != 0) << 31)
                            | (self.registers[right_register] >> 1)
                    }
                    3 => self.registers[right_register].rotate_right(shift_amount),
                    _ => unreachable!(),
                };
                let left = if source == 15 && matches!(shifted_logical_operation, 2 | 3) {
                    0
                } else {
                    self.registers[source]
                };
                let result = match shifted_logical_operation {
                    0 => left & right,
                    1 => left & !right,
                    2 => left | right,
                    3 => left | !right,
                    4 => left ^ right,
                    _ => unreachable!(),
                };
                if destination != 15 {
                    self.registers[destination] = result;
                }
                if instruction & 0x10 != 0 {
                    self.nz(result);
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            let multiple_operation = instruction & 0xffd0;
            if matches!(multiple_operation, 0xe880 | 0xe890 | 0xe900 | 0xe910) {
                let load = multiple_operation & 0x10 != 0;
                let decrement_before = multiple_operation & 0x0100 != 0;
                let writeback = instruction & 0x20 != 0;
                let base_register = usize::from(instruction & 0xf);
                let count = second.count_ones();
                let base = self.registers[base_register];
                let mut address = if decrement_before {
                    base.wrapping_sub(count * 4)
                } else {
                    base
                };
                let mut loaded_pc = None;
                for register in 0..16 {
                    if second & (1 << register) == 0 {
                        continue;
                    }
                    if load {
                        let value =
                            self.read(bus, address, AccessWidth::Word, AccessKind::Read, now)?;
                        if register == 15 {
                            loaded_pc = Some(value);
                        } else {
                            self.registers[register] = value;
                        }
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
                if writeback {
                    self.registers[base_register] = if decrement_before {
                        base.wrapping_sub(count * 4)
                    } else {
                        base.wrapping_add(count * 4)
                    };
                }
                if let Some(target) = loaded_pc {
                    self.branch_exchange(target, bus, now)?;
                } else {
                    self.registers[15] = pc.wrapping_add(4);
                }
                return Ok(StepReason::Advanced);
            }
            let memory_operation = instruction & 0xfff0;
            if matches!(memory_operation, 0xf910 | 0xf930 | 0xf990 | 0xf9b0) {
                let base_register = usize::from(instruction & 0xf);
                let base = self.registers[base_register];
                let (address, writeback) = if instruction & 0x0080 != 0 {
                    (base.wrapping_add(u32::from(second & 0x0fff)), None)
                } else if second & 0x0fc0 == 0 {
                    let offset_register = usize::from(second & 0xf);
                    let shift = u32::from((second >> 4) & 3);
                    (
                        base.wrapping_add(self.registers[offset_register] << shift),
                        None,
                    )
                } else {
                    let offset = u32::from(second & 0xff);
                    let adjusted = if second & 0x0200 != 0 {
                        base.wrapping_add(offset)
                    } else {
                        base.wrapping_sub(offset)
                    };
                    (
                        if second & 0x0400 != 0 { adjusted } else { base },
                        (second & 0x0100 != 0).then_some(adjusted),
                    )
                };
                let destination = usize::from((second >> 12) & 0xf);
                self.registers[destination] = if matches!(memory_operation, 0xf910 | 0xf990) {
                    let value =
                        self.read(bus, address, AccessWidth::Byte, AccessKind::Read, now)?;
                    i32::from(value as u8 as i8) as u32
                } else {
                    let value =
                        self.read(bus, address, AccessWidth::HalfWord, AccessKind::Read, now)?;
                    i32::from(value as u16 as i16) as u32
                };
                if let Some(updated) = writeback {
                    self.registers[base_register] = updated;
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(
                memory_operation,
                0xf800
                    | 0xf810
                    | 0xf820
                    | 0xf830
                    | 0xf840
                    | 0xf850
                    | 0xf880
                    | 0xf890
                    | 0xf8a0
                    | 0xf8b0
                    | 0xf8c0
                    | 0xf8d0
            ) {
                let load = memory_operation & 0x10 != 0;
                let width = match memory_operation & 0x60 {
                    0x00 => AccessWidth::Byte,
                    0x20 => AccessWidth::HalfWord,
                    0x40 => AccessWidth::Word,
                    _ => unreachable!(),
                };
                let base_register = usize::from(instruction & 0xf);
                let transfer_register = usize::from((second >> 12) & 0xf);
                let base = self.registers[base_register];
                let (address, writeback) = if base_register == 15 {
                    let aligned_pc = pc.wrapping_add(4) & !3;
                    let offset = u32::from(second & 0x0fff);
                    (
                        if instruction & 0x0080 != 0 {
                            aligned_pc.wrapping_add(offset)
                        } else {
                            aligned_pc.wrapping_sub(offset)
                        },
                        None,
                    )
                } else if instruction & 0x0080 != 0 {
                    (base.wrapping_add(u32::from(second & 0x0fff)), None)
                } else if second & 0x0fc0 == 0 {
                    let offset_register = usize::from(second & 0xf);
                    let shift = u32::from((second >> 4) & 3);
                    (
                        base.wrapping_add(self.registers[offset_register] << shift),
                        None,
                    )
                } else {
                    let offset = u32::from(second & 0xff);
                    let adjusted = if second & 0x0200 != 0 {
                        base.wrapping_add(offset)
                    } else {
                        base.wrapping_sub(offset)
                    };
                    let address = if second & 0x0400 != 0 { adjusted } else { base };
                    (address, (second & 0x0100 != 0).then_some(adjusted))
                };
                if load {
                    let value = self.read(bus, address, width, AccessKind::Read, now)?;
                    if transfer_register == 15 {
                        self.branch_exchange(value, bus, now)?;
                    } else {
                        self.registers[transfer_register] = value;
                    }
                } else {
                    self.write(bus, address, width, self.registers[transfer_register], now)?;
                }
                if let Some(updated) = writeback {
                    self.registers[base_register] = updated;
                }
                if !(load && transfer_register == 15) {
                    self.registers[15] = pc.wrapping_add(4);
                }
                return Ok(StepReason::Advanced);
            }
            if instruction == 0xf3ef && second & 0xf000 == 0x8000 {
                let destination = usize::from((second >> 8) & 0xf);
                let system_register = second & 0xff;
                self.registers[destination] = match system_register {
                    0 | 1 | 8 => self.registers[13],
                    5 => self.xpsr & 0x1ff,
                    6 => self.xpsr & 0x0700_fc00,
                    7 => self.xpsr & 0x0700_fdff,
                    0x10 => u32::from(self.primask),
                    0x11 | 0x12 | 0x13 | 0x14 => 0,
                    _ => {
                        return Err(self.fault(
                            CpuFaultKind::Unsupported,
                            format!("MRS system register {system_register:#04x} is not modeled"),
                        ));
                    }
                };
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfff0 == 0xf380 && second & 0xff00 == 0x8800 {
                let source = usize::from(instruction & 0xf);
                let system_register = second & 0xff;
                match system_register {
                    0 | 1 | 8 => self.registers[13] = self.registers[source],
                    0x10 => self.primask = self.registers[source] & 1 != 0,
                    0x11 | 0x12 | 0x13 | 0x14 => {}
                    _ => {
                        return Err(self.fault(
                            CpuFaultKind::Unsupported,
                            format!("MSR system register {system_register:#04x} is not modeled"),
                        ));
                    }
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction == 0xf3bf && matches!(second, 0x8f4f | 0x8f5f | 0x8f6f) {
                // DSB, DMB, and ISB preserve ordering. The interpreter completes every bus
                // operation synchronously, so each architectural barrier is already satisfied.
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(instruction & 0xfbf0, 0xf240 | 0xf2c0) && second & 0x8000 == 0 {
                let destination = usize::from((second >> 8) & 0xf);
                let immediate = (u32::from(instruction & 0xf) << 12)
                    | (u32::from((instruction >> 10) & 1) << 11)
                    | (u32::from((second >> 12) & 7) << 8)
                    | u32::from(second & 0xff);
                if instruction & 0x0080 == 0 {
                    self.registers[destination] = immediate;
                } else {
                    self.registers[destination] =
                        (self.registers[destination] & 0xffff) | (immediate << 16);
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if matches!(instruction & 0xfbf0, 0xf200 | 0xf2a0) && second & 0x8000 == 0 {
                let source_register = usize::from(instruction & 0xf);
                let source = if source_register == 15 {
                    // ADDW/SUBW with PC is the ADR form and observes the architecturally
                    // visible, word-aligned PC rather than the current instruction address.
                    pc.wrapping_add(4) & !3
                } else {
                    self.registers[source_register]
                };
                let destination = usize::from((second >> 8) & 0xf);
                let immediate = (u32::from((instruction >> 10) & 1) << 11)
                    | (u32::from((second >> 12) & 7) << 8)
                    | u32::from(second & 0xff);
                self.registers[destination] = if instruction & 0x0080 != 0 {
                    source.wrapping_sub(immediate)
                } else {
                    source.wrapping_add(immediate)
                };
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfbef == 0xf06f && second & 0x8000 == 0 {
                let destination = usize::from((second >> 8) & 0xf);
                let immediate = (u32::from((instruction >> 10) & 1) << 11)
                    | (u32::from((second >> 12) & 7) << 8)
                    | u32::from(second & 0xff);
                let result = !thumb_expand_immediate(immediate);
                self.registers[destination] = result;
                self.nz(result);
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xfbef == 0xf04f && second & 0x8000 == 0 {
                let destination = usize::from((second >> 8) & 0xf);
                let immediate = (u32::from((instruction >> 10) & 1) << 11)
                    | (u32::from((second >> 12) & 7) << 8)
                    | u32::from(second & 0xff);
                let result = thumb_expand_immediate(immediate);
                self.registers[destination] = result;
                if instruction & 0x10 != 0 {
                    self.nz(result);
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            let arithmetic_operation = instruction & 0xfbe0;
            if matches!(
                arithmetic_operation,
                0xf100 | 0xf140 | 0xf160 | 0xf1a0 | 0xf1c0
            ) && second & 0x8000 == 0
            {
                let source = usize::from(instruction & 0xf);
                let destination = usize::from((second >> 8) & 0xf);
                let immediate = (u32::from((instruction >> 10) & 1) << 11)
                    | (u32::from((second >> 12) & 7) << 8)
                    | u32::from(second & 0xff);
                let operand = thumb_expand_immediate(immediate);
                let left = self.registers[source];
                let carry = u32::from(self.xpsr & C != 0);
                let result = match arithmetic_operation {
                    0xf100 => left.wrapping_add(operand),
                    0xf140 => left.wrapping_add(operand).wrapping_add(carry),
                    0xf160 => left.wrapping_sub(operand).wrapping_sub(1 - carry),
                    0xf1a0 => left.wrapping_sub(operand),
                    0xf1c0 => operand.wrapping_sub(left),
                    _ => unreachable!(),
                };
                self.registers[destination] = result;
                if instruction & 0x10 != 0 {
                    match arithmetic_operation {
                        0xf100 => self.add_flags(left, operand, result),
                        0xf140 => {
                            self.nz(result);
                            self.xpsr &= !(C | V);
                            if u64::from(left) + u64::from(operand) + u64::from(carry)
                                > u64::from(u32::MAX)
                            {
                                self.xpsr |= C;
                            }
                            let signed = i64::from(left as i32)
                                + i64::from(operand as i32)
                                + i64::from(carry);
                            if signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX) {
                                self.xpsr |= V;
                            }
                        }
                        0xf160 => {
                            self.nz(result);
                            self.xpsr &= !(C | V);
                            let subtrahend = u64::from(operand) + u64::from(1 - carry);
                            if u64::from(left) >= subtrahend {
                                self.xpsr |= C;
                            }
                            let signed = i64::from(left as i32)
                                - i64::from(operand as i32)
                                - i64::from(1 - carry);
                            if signed < i64::from(i32::MIN) || signed > i64::from(i32::MAX) {
                                self.xpsr |= V;
                            }
                        }
                        0xf1a0 => self.sub_flags(left, operand, result),
                        0xf1c0 => self.sub_flags(operand, left, result),
                        _ => unreachable!(),
                    }
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            let logical_operation = (instruction >> 5) & 0xf;
            if instruction & 0xfa00 == 0xf000 && logical_operation <= 4 && second & 0x8000 == 0 {
                let source = usize::from(instruction & 0xf);
                let destination = usize::from((second >> 8) & 0xf);
                let immediate = (u32::from((instruction >> 10) & 1) << 11)
                    | (u32::from((second >> 12) & 7) << 8)
                    | u32::from(second & 0xff);
                let operand = thumb_expand_immediate(immediate);
                let left = self.registers[source];
                let result = match logical_operation {
                    0 => left & operand,
                    1 => left & !operand,
                    2 => left | operand,
                    3 => left | !operand,
                    4 => left ^ operand,
                    _ => unreachable!(),
                };
                if destination != 15 {
                    self.registers[destination] = result;
                }
                if instruction & 0x10 != 0 {
                    self.nz(result);
                }
                self.registers[15] = pc.wrapping_add(4);
                return Ok(StepReason::Advanced);
            }
            let branch_kind = second & 0xd000;
            if instruction & 0xf800 == 0xf000 && branch_kind == 0x8000 {
                let condition = (instruction >> 6) & 0xf;
                if condition >= 0xe {
                    return Err(self.fault(
                        CpuFaultKind::IllegalInstruction,
                        "invalid condition in 32-bit Thumb branch",
                    ));
                }
                let encoded = (u32::from((instruction >> 10) & 1) << 20)
                    | (u32::from((second >> 11) & 1) << 19)
                    | (u32::from((second >> 13) & 1) << 18)
                    | (u32::from(instruction & 0x3f) << 12)
                    | (u32::from(second & 0x7ff) << 1);
                self.registers[15] = if self.condition(condition) {
                    pc.wrapping_add(4)
                        .wrapping_add_signed(sign_extend(encoded, 21))
                } else {
                    pc.wrapping_add(4)
                };
                return Ok(StepReason::Advanced);
            }
            if instruction & 0xf800 != 0xf000 || !matches!(branch_kind, 0x9000 | 0xd000) {
                return Err(self.fault(
                    CpuFaultKind::IllegalInstruction,
                    "unsupported 32-bit Thumb encoding",
                ));
            }
            let s = u32::from((instruction >> 10) & 1);
            let j1 = u32::from((second >> 13) & 1);
            let j2 = u32::from((second >> 11) & 1);
            let i1 = (!(j1 ^ s)) & 1;
            let i2 = (!(j2 ^ s)) & 1;
            let encoded = (s << 24)
                | (i1 << 23)
                | (i2 << 22)
                | (u32::from(instruction & 0x03ff) << 12)
                | (u32::from(second & 0x07ff) << 1);
            if branch_kind == 0xd000 {
                self.registers[14] = pc.wrapping_add(4) | 1;
            }
            self.registers[15] = pc
                .wrapping_add(4)
                .wrapping_add_signed(sign_extend(encoded, 25));
            return Ok(StepReason::Advanced);
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
            self.halted = true;
            self.registers[15] = next;
            return Ok(StepReason::Halted);
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
        line: u16,
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
        self.active_interrupt = Some(line);
        self.xpsr = (self.xpsr & !0x1ff) | (u32::from(line) + 16);
        let vector = self
            .vector_base
            .wrapping_add((u32::from(line) + 16).wrapping_mul(4));
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
            && let Some(line) = self.interrupts.iter().next().copied()
        {
            self.take_interrupt(line, bus, now)?;
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
            self.interrupts.insert(line);
            self.waiting = false;
        } else {
            self.interrupts.remove(&line);
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
mod tests {
    use super::*;
    use renvo_bus::AddressSpace;

    #[test]
    fn executes_thumb_arithmetic_and_halts() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x2000, true).unwrap();
        // movs r0,#7; adds r0,#5; bkpt #0
        bus.load(0, &[0x07, 0x20, 0x05, 0x30, 0x00, 0xbe]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
        cpu.set_direct_state(0x1000, 1).unwrap();
        for tick in 0..2 {
            cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
        }
        assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 12);
        assert_eq!(
            cpu.step(&mut bus, SimTime::from_ticks(2)).unwrap().reason,
            StepReason::Halted
        );
    }

    #[test]
    fn it_conditionally_skips_without_becoming_wfi() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // movs r0,#2; cmp r0,#1; it cc; movcc r1,#7; movs r2,#9
        bus.load(
            0,
            &[0x02, 0x20, 0x01, 0x28, 0x38, 0xbf, 0x07, 0x21, 0x09, 0x22],
        )
        .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();

        for tick in 0..5 {
            cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
        }

        assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 0);
        assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 9);
        assert!(!cpu.waiting);
    }

    #[test]
    fn push_and_pop_restore_a_register() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x2000, true).unwrap();
        // push {r0}; movs r0,#0; pop {r0}
        bus.load(0, &[0x01, 0xb4, 0x00, 0x20, 0x01, 0xbc]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
        cpu.set_direct_state(0x1000, 1).unwrap();
        cpu.registers[0] = 42;
        for tick in 0..3 {
            cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
        }
        assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 42);
        assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x1000);
    }

    #[test]
    fn ldmia_preserves_a_loaded_base_register() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x2000, true).unwrap();
        // ldmia r0, {r0, r1}
        bus.load(0, &[0x03, 0xc8]).unwrap();
        bus.load(0x100, &0x1234_5678_u32.to_le_bytes()).unwrap();
        bus.load(0x104, &0x9abc_def0_u32.to_le_bytes()).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
        cpu.set_direct_state(0x1000, 1).unwrap();
        cpu.registers[0] = 0x100;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 0x1234_5678);
        assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 0x9abc_def0);
    }

    #[test]
    fn cbz_and_cbnz_branch_from_the_prefetched_pc() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // cbz r0,+4; movs r1,#1; movs r1,#2; cbnz r0,+4
        bus.load(0, &[0x10, 0xb1, 0x01, 0x21, 0x02, 0x21, 0x10, 0xb9])
            .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 8);

        cpu.registers[15] = 6;
        cpu.registers[0] = 1;
        cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();
        assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 14);
    }

    #[test]
    fn thumb2_modified_immediate_subtracts_from_sp() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // sub.w sp,sp,#256
        bus.load(0, &[0xad, 0xf5, 0x80, 0x7d]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x2000, 1).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x1f00);
        assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 4);
    }

    #[test]
    fn thumb2_tst_and_bic_modified_immediates() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // tst.w r0,#2; bic.w r1,r1,#1
        bus.load(0, &[0x10, 0xf0, 0x02, 0x0f, 0x31, 0xf0, 0x01, 0x01])
            .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[0] = 2;
        cpu.registers[1] = 3;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.xpsr & Z, 0);
        cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();
        assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 2);
    }

    #[test]
    fn thumb2_movw_and_movt_form_a_constant() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // movw r0,#0xa0eb; movt r0,#0x1234
        bus.load(0, &[0x4a, 0xf2, 0xeb, 0x00, 0xc1, 0xf2, 0x34, 0x20])
            .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

        assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 0x1234_a0eb);
    }

    #[test]
    fn thumb2_subw_uses_the_unexpanded_twelve_bit_immediate() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // subw r2,r2,#0x8cb
        bus.load(0, &[0xa2, 0xf6, 0xcb, 0x02]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[2] = 0x1000;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0x735);
    }

    #[test]
    fn thumb2_signed_halfword_multiply_accumulates() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // smlabb r0,r9,r0,ip
        bus.load(0, &[0x19, 0xfb, 0x00, 0xc0]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[0] = 3;
        cpu.registers[9] = 0x0000_fffe;
        cpu.registers[12] = 10;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 4);
    }

    #[test]
    fn rp2350_gpio_coprocessor_routes_single_bit_output_and_enable_to_sio() {
        let mut bus = AddressSpace::default();
        bus.map_ram("code", 0, 0x100, true).unwrap();
        bus.map_ram("sio", 0xd000_0000, 0x200, false).unwrap();
        // mcrr p0,#4,r0,r6,c0; mcrr p0,#4,r0,r3,c4
        bus.load(0, &[0x46, 0xec, 0x40, 0x00, 0x43, 0xec, 0x44, 0x00])
            .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[0] = 3;
        cpu.registers[6] = 1;
        cpu.registers[3] = 1;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

        assert_eq!(
            bus.read(
                0xd000_0018,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
            1 << 3
        );
        assert_eq!(
            bus.read(
                0xd000_0038,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
            1 << 3
        );
    }

    #[test]
    fn rp2350_gpio_coprocessor_reads_low_input_bank() {
        let mut bus = AddressSpace::default();
        bus.map_ram("code", 0, 0x100, true).unwrap();
        bus.map_ram("sio", 0xd000_0000, 0x200, false).unwrap();
        // mrc p0,#0,r2,c0,c8
        bus.load(0, &[0x10, 0xee, 0x18, 0x20]).unwrap();
        bus.load(0xd000_0004, &0x5a5a_a5a5_u32.to_le_bytes())
            .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0x5a5a_a5a5);
    }

    #[test]
    fn thumb2_post_indexed_load_updates_its_base() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x200, true).unwrap();
        // ldr.w r3,[r4],#4
        bus.load(0, &[0x54, 0xf8, 0x04, 0x3b]).unwrap();
        bus.load(0x100, &0x1234_5678_u32.to_le_bytes()).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x1f0, 1).unwrap();
        cpu.registers[4] = 0x100;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 0x1234_5678);
        assert_eq!(cpu.register(ArmRegister::R4).unwrap(), 0x104);
    }

    #[test]
    fn thumb2_signed_byte_load_sign_extends() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x200, true).unwrap();
        // ldrsb.w r0,[r2]
        bus.load(0, &[0x92, 0xf9, 0x00, 0x00]).unwrap();
        bus.load(0x100, &[0x80]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x1f0, 1).unwrap();
        cpu.registers[2] = 0x100;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 0xffff_ff80);
    }

    #[test]
    fn thumb2_literal_load_to_pc_follows_a_veneer() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x200, true).unwrap();
        // ldr.w pc,[pc]; literal is at pc+4
        bus.load(0, &[0x5f, 0xf8, 0x00, 0xf0, 0x81, 0x00, 0x00, 0x00])
            .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x100, 1).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 0x80);
    }

    #[test]
    fn thumb2_strd_predecrements_and_ldrd_restores_the_pair() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x200, true).unwrap();
        // strd ip,lr,[sp,#-16]!; ldrd r2,r3,[sp,#8]
        bus.load(0, &[0x6d, 0xe9, 0x04, 0xce, 0xdd, 0xe9, 0x02, 0x23])
            .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x100, 1).unwrap();
        cpu.registers[12] = 0x1234_5678;
        cpu.registers[14] = 0x9abc_def0;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0xf0);
        cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

        assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0);
        assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 0);
        assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0xf0);
    }

    #[test]
    fn armv8m_tt_reports_the_functional_nonsecure_address_space() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // tt r2,r2
        bus.load(0, &[0x42, 0xe8, 0x00, 0xf2]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[2] = 0x1000_0000;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0);
    }

    #[test]
    fn armv8m_store_release_byte_has_ordered_store_effects() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // stlb r2,[r1]
        bus.load(0, &[0xc1, 0xe8, 0x8f, 0x2f]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[1] = 0x40;
        cpu.registers[2] = 0x1234_56ab;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(
            bus.read(0x40, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
                .unwrap(),
            0xab
        );
    }

    #[test]
    fn armv8m_byte_exclusive_pair_succeeds_on_one_core() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // ldaexb r1,[r3]; strexb r1,r2,[r3]
        bus.load(0, &[0xd3, 0xe8, 0xcf, 0x1f, 0xc3, 0xe8, 0x41, 0x2f])
            .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[3] = 0x40;
        cpu.registers[2] = 0xab;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

        assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 0);
        assert_eq!(
            bus.read(0x40, AccessWidth::Byte, AccessKind::Read, SimTime::ZERO)
                .unwrap(),
            0xab
        );
    }

    #[test]
    fn thumb2_table_branch_byte_indexes_from_prefetched_pc() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // tbb [pc,r3]; table bytes 1,3
        bus.load(0, &[0xdf, 0xe8, 0x03, 0xf0, 1, 3]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[3] = 1;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 10);
    }

    #[test]
    fn cortex_m33_vstr_and_vldr_preserve_a_double_register() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x200, true).unwrap();
        // vstr d8,[r0,#48]; vldr d9,[r0,#48]
        bus.load(0, &[0x80, 0xed, 0x0c, 0x8b, 0x90, 0xed, 0x0c, 0x9b])
            .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x1f0, 1).unwrap();
        cpu.registers[0] = 0x80;
        cpu.fpu_registers[8] = 0x1234_5678_9abc_def0;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

        assert_eq!(cpu.fpu_registers[9], 0x1234_5678_9abc_def0);
    }

    #[test]
    fn cortex_m33_vpush_and_vpop_round_trip_a_double_register() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x200, true).unwrap();
        // vpush {d8}; vpop {d8}
        bus.load(0, &[0x2d, 0xed, 0x02, 0x8b, 0xbd, 0xec, 0x02, 0x8b])
            .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x180, 1).unwrap();
        cpu.fpu_registers[8] = 0x1234_5678_9abc_def0;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x178);
        cpu.fpu_registers[8] = 0;
        cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();

        assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x180);
        assert_eq!(cpu.fpu_registers[8], 0x1234_5678_9abc_def0);
    }

    #[test]
    fn thumb2_ldmia_restores_high_registers_and_writes_back() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x200, true).unwrap();
        // ldmia.w sp!,{r4,lr}
        bus.load(0, &[0xbd, 0xe8, 0x10, 0x40]).unwrap();
        bus.load(0x100, &0x1234_u32.to_le_bytes()).unwrap();
        bus.load(0x104, &0x81_u32.to_le_bytes()).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x100, 1).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R4).unwrap(), 0x1234);
        assert_eq!(cpu.register(ArmRegister::Lr).unwrap(), 0x81);
        assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x108);
    }

    #[test]
    fn thumb2_bics_shifted_register_updates_flags() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // bics.w r2,r3,r2
        bus.load(0, &[0x33, 0xea, 0x02, 0x02]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[2] = 0x0f;
        cpu.registers[3] = 0xff;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0xf0);
        assert_eq!(cpu.xpsr & Z, 0);
    }

    #[test]
    fn thumb2_mov_shifted_register_alias_does_not_use_pc() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // mov.w r8,r3,lsl #2 (ORR with Rn=PC)
        bus.load(0, &[0x4f, 0xea, 0x83, 0x08]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[3] = 5;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R8).unwrap(), 20);
    }

    #[test]
    fn thumb2_unconditional_wide_branch_does_not_link() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // b.w from 0 to 0x20
        bus.load(0, &[0x00, 0xf0, 0x0e, 0xb8]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[14] = 0x55;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 0x20);
        assert_eq!(cpu.register(ArmRegister::Lr).unwrap(), 0x55);
    }

    #[test]
    fn thumb2_conditional_wide_branch_uses_the_current_flags() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x400, true).unwrap();
        // bne.w +416
        bus.load(0, &[0x40, 0xf0, 0xd0, 0x80]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x300, 1).unwrap();

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 420);
    }

    #[test]
    fn thumb2_unsigned_division_matches_cortex_m33() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // udiv r4,r4,r1
        bus.load(0, &[0xb4, 0xfb, 0xf1, 0xf4]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[4] = 100;
        cpu.registers[1] = 6;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R4).unwrap(), 16);
    }

    #[test]
    fn thumb2_unsigned_bitfield_extracts_the_requested_width() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // ubfx r4,r4,#0,#12
        bus.load(0, &[0xc4, 0xf3, 0x0b, 0x04]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[4] = 0x1234_5abc;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R4).unwrap(), 0xabc);
    }

    #[test]
    fn thumb2_bitfield_insert_replaces_only_the_field() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // bfi r3,r0,#0,#1
        bus.load(0, &[0x60, 0xf3, 0x00, 0x03]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[0] = 1;
        cpu.registers[3] = 0xffff_fffe;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R3).unwrap(), u32::MAX);
    }

    #[test]
    fn thumb2_clz_counts_all_leading_zeroes() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // clz r7,r2
        bus.load(0, &[0xb2, 0xfa, 0x82, 0xf7]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[2] = 0x0000_0800;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R7).unwrap(), 20);
    }

    #[test]
    fn thumb2_register_controlled_shift_uses_low_byte_of_amount() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // lsl.w ip,lr,r7
        bus.load(0, &[0x0e, 0xfa, 0x07, 0xfc]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[14] = 3;
        cpu.registers[7] = 4;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R12).unwrap(), 48);
    }

    #[test]
    fn thumb2_uxtb_accepts_a_high_source_register() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // uxtb.w r7,r11
        bus.load(0, &[0x5f, 0xfa, 0x8b, 0xf7]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[11] = 0x1234_56ab;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R7).unwrap(), 0xab);
    }

    #[test]
    fn thumb2_uxtah_extends_then_adds() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // uxtah r5,r7,r5
        bus.load(0, &[0x17, 0xfa, 0x85, 0xf5]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[5] = 0x1234_ffff;
        cpu.registers[7] = 2;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R5).unwrap(), 0x1_0001);
    }

    #[test]
    fn thumb2_mls_subtracts_a_product_from_the_accumulator() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // mls ip,lr,r8,ip
        bus.load(0, &[0x0e, 0xfb, 0x18, 0xcc]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[14] = 4;
        cpu.registers[8] = 5;
        cpu.registers[12] = 100;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R12).unwrap(), 80);
    }

    #[test]
    fn thumb2_umull_writes_both_halves() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // umull r3,r1,r3,r2
        bus.load(0, &[0xa3, 0xfb, 0x02, 0x31]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[3] = u32::MAX;
        cpu.registers[2] = 2;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 0xffff_fffe);
        assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 1);
    }

    #[test]
    fn thumb2_umlal_accumulates_into_both_halves() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // umlal r2,r3,r4,r1
        bus.load(0, &[0xe4, 0xfb, 0x01, 0x23]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[1] = 3;
        cpu.registers[2] = u32::MAX;
        cpu.registers[3] = 1;
        cpu.registers[4] = 2;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 5);
        assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 2);
    }

    #[test]
    fn thumb2_subtract_shifted_register_handles_high_registers() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // sub.w r3,r3,r9
        bus.load(0, &[0xa3, 0xeb, 0x09, 0x03]).unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x80, 1).unwrap();
        cpu.registers[3] = 100;
        cpu.registers[9] = 23;

        cpu.step(&mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.register(ArmRegister::R3).unwrap(), 77);
    }

    #[test]
    fn official_rp2350_strlen_sequence_returns_a_plain_length() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x400, true).unwrap();
        bus.load(
            0,
            &[
                0x20, 0xf0, 0x03, 0x01, 0x10, 0xf0, 0x03, 0x00, 0xc0, 0xf1, 0x00, 0x00, 0x51, 0xf8,
                0x04, 0x3b, 0x00, 0xf1, 0x04, 0x0c, 0x4f, 0xea, 0xcc, 0x0c, 0x6f, 0xf0, 0x00, 0x02,
                0x1c, 0xbf, 0x22, 0xfa, 0x0c, 0xf2, 0x13, 0x43, 0x4f, 0xf0, 0x01, 0x0c, 0x4c, 0xea,
                0x0c, 0x2c, 0x4c, 0xea, 0x0c, 0x4c, 0xa3, 0xeb, 0x0c, 0x02, 0x22, 0xea, 0x03, 0x02,
                0x12, 0xea, 0xcc, 0x12, 0x04, 0xbf, 0x51, 0xf8, 0x04, 0x3b, 0x04, 0x30, 0xf4, 0xd0,
                0xc2, 0xf1, 0x00, 0x01, 0x02, 0xea, 0x01, 0x02, 0xb2, 0xfa, 0x82, 0xf2, 0xc2, 0xf1,
                0x1f, 0x02, 0x00, 0xeb, 0xd2, 0x00, 0x70, 0x47,
            ],
        )
        .unwrap();
        bus.load(0x100, b"rp2.py\0").unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM33);
        cpu.set_direct_state(0x300, 1).unwrap();
        cpu.registers[0] = 0x100;
        cpu.registers[14] = 0x201;

        for tick in 0..100 {
            if cpu.registers[15] == 0x200 {
                break;
            }
            cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
        }

        assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 0x200);
        assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 6);
    }

    #[test]
    fn architectural_barriers_advance_after_synchronous_bus_work() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // dmb sy; dsb sy; isb sy
        bus.load(
            0,
            &[
                0xbf, 0xf3, 0x5f, 0x8f, 0xbf, 0xf3, 0x4f, 0x8f, 0xbf, 0xf3, 0x6f, 0x8f,
            ],
        )
        .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
        cpu.set_direct_state(0x80, 1).unwrap();
        for tick in 0..3 {
            cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
        }
        assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 12);
    }

    #[test]
    fn primask_moves_and_cps_are_observable() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x100, true).unwrap();
        // mrs r0,primask; cpsid i; mrs r1,primask; msr primask,r0; mrs r2,primask
        bus.load(
            0,
            &[
                0xef, 0xf3, 0x10, 0x80, 0x72, 0xb6, 0xef, 0xf3, 0x10, 0x81, 0x80, 0xf3, 0x10, 0x88,
                0xef, 0xf3, 0x10, 0x82,
            ],
        )
        .unwrap();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
        cpu.set_direct_state(0x80, 1).unwrap();
        for tick in 0..5 {
            cpu.step(&mut bus, SimTime::from_ticks(tick)).unwrap();
        }
        assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 0);
        assert_eq!(cpu.register(ArmRegister::R1).unwrap(), 1);
        assert_eq!(cpu.register(ArmRegister::R2).unwrap(), 0);
    }

    #[test]
    fn high_add_to_pc_branches_using_the_prefetched_pc() {
        let mut bus = AddressSpace::default();
        let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
        cpu.registers[15] = 0x100;
        cpu.registers[1] = 6;

        // add pc, r1
        cpu.execute(0x448f, &mut bus, SimTime::ZERO).unwrap();

        assert_eq!(cpu.registers[15], 0x10a);
    }

    #[test]
    fn external_interrupt_stacks_and_returns() {
        let mut bus = AddressSpace::default();
        bus.map_ram("memory", 0, 0x1000, true).unwrap();
        bus.load(16 * 4, &0x121_u32.to_le_bytes()).unwrap();
        bus.load(0x100, &[0x30, 0xbf, 0x00, 0xbe]).unwrap(); // wfi; bkpt
        bus.load(0x120, &[0x2a, 0x20, 0x70, 0x47]).unwrap(); // movs r0,#42; bx lr
        let mut cpu = ArmCpu::new(ArmProfile::CortexM0Plus);
        cpu.set_vector_base(0);
        cpu.set_direct_state(0x800, 0x101).unwrap();
        cpu.step(&mut bus, SimTime::ZERO).unwrap();
        cpu.set_interrupt(0, true).unwrap();
        cpu.step(&mut bus, SimTime::from_ticks(1)).unwrap();
        cpu.step(&mut bus, SimTime::from_ticks(2)).unwrap();
        assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 42);
        cpu.set_interrupt(0, false).unwrap();
        cpu.step(&mut bus, SimTime::from_ticks(3)).unwrap();
        assert_eq!(cpu.register(ArmRegister::R0).unwrap(), 0);
        assert_eq!(cpu.register(ArmRegister::Sp).unwrap(), 0x800);
        assert_eq!(cpu.register(ArmRegister::Pc).unwrap(), 0x102);
    }
}
