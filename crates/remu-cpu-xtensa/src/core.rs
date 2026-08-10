//! Interpreted Xtensa LX7 CPU implementation for the ESP32-S3 baseline.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::verbose_bit_mask
)]

use remu_core::{
    AccessKind, AccessWidth, Architecture, Bus, Cpu, CpuFault, CpuFaultKind, CpuSnapshot,
    RegisterValue, ResetKind, SimDuration, SimTime, StepOutcome, StepReason,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

mod execution;

/// A visible Xtensa address register.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum XtensaRegister {
    /// Address register A0, conventionally the return address.
    A0 = 0,
    /// Address register A1, conventionally the stack pointer.
    A1 = 1,
    /// Address register A2, the first call argument and return value.
    A2 = 2,
    /// Address register A3.
    A3 = 3,
    /// Address register A4.
    A4 = 4,
    /// Address register A5.
    A5 = 5,
    /// Address register A6.
    A6 = 6,
    /// Address register A7.
    A7 = 7,
    /// Address register A8.
    A8 = 8,
    /// Address register A9.
    A9 = 9,
    /// Address register A10.
    A10 = 10,
    /// Address register A11.
    A11 = 11,
    /// Address register A12.
    A12 = 12,
    /// Address register A13.
    A13 = 13,
    /// Address register A14.
    A14 = 14,
    /// Address register A15.
    A15 = 15,
}

impl XtensaRegister {
    /// Returns the architectural register index.
    pub const fn index(self) -> usize {
        self as usize
    }
}

impl TryFrom<u8> for XtensaRegister {
    type Error = CpuFault;

    fn try_from(index: u8) -> Result<Self, Self::Error> {
        match index {
            0 => Ok(Self::A0),
            1 => Ok(Self::A1),
            2 => Ok(Self::A2),
            3 => Ok(Self::A3),
            4 => Ok(Self::A4),
            5 => Ok(Self::A5),
            6 => Ok(Self::A6),
            7 => Ok(Self::A7),
            8 => Ok(Self::A8),
            9 => Ok(Self::A9),
            10 => Ok(Self::A10),
            11 => Ok(Self::A11),
            12 => Ok(Self::A12),
            13 => Ok(Self::A13),
            14 => Ok(Self::A14),
            15 => Ok(Self::A15),
            _ => Err(CpuFault::new(
                CpuFaultKind::Architecture,
                0,
                "Xtensa address register index exceeds A15",
            )),
        }
    }
}

#[derive(Clone)]
struct WindowFrame {
    registers: [u32; 16],
    return_address: u32,
    call_increment: usize,
}

#[derive(Clone)]
struct InterruptContext {
    registers: [u32; 16],
    window_stack: Vec<WindowFrame>,
    thread_pointer: u32,
}

#[derive(Clone, Default)]
struct TaskContexts {
    window_stacks: BTreeMap<u32, Vec<WindowFrame>>,
    registers: BTreeMap<u32, [u32; 16]>,
}

/// Functional ESP32-S3 LX7 application CPU.
#[derive(Clone)]
pub struct XtensaCpu {
    registers: [u32; 16],
    window_stack: Vec<WindowFrame>,
    interrupt_contexts: [Option<InterruptContext>; 8],
    level_one_exception_active: bool,
    task_contexts: Arc<Mutex<TaskContexts>>,
    floating_registers: [u32; 16],
    special_registers: [u32; 256],
    processor_id: u32,
    thread_pointer: u32,
    pc: u32,
    ps: u32,
    sar: u32,
    boolean_registers: u16,
    loop_begin: u32,
    loop_end: u32,
    loop_count: u32,
    loop_active: bool,
    branch_taken: bool,
    waiting: bool,
    halted: bool,
    interrupts: BTreeSet<u16>,
    software_interrupts: u32,
}

impl XtensaCpu {
    const PROCESSOR_ID_CORE0: u32 = 0x0000_cdcd;
    const PROCESSOR_ID_CORE1: u32 = 0x0000_abab;
    const EXCM: u32 = 1 << 4;
    const EXCM_LEVEL: u32 = 3;
    // ESP32-S3 LX7 interrupt-level masks from the core configuration.  These
    // are properties of the processor interrupt inputs, not peripheral
    // routes: the interrupt matrix only selects which of these inputs a
    // source asserts.
    const INTERRUPT_LEVELS: [(u32, u32, u32); 6] = [
        (7, 0x0000_4000, 0x2c0),
        (5, 0x8401_0000, 0x240),
        (4, 0x5300_0000, 0x200),
        (3, 0x28c0_8800, 0x1c0),
        (2, 0x0038_0000, 0x180),
        (1, 0x0006_37ff, 0x340),
    ];

    /// Creates an uninitialized direct-load CPU.
    pub fn new() -> Self {
        Self {
            registers: [0; 16],
            window_stack: Vec::new(),
            interrupt_contexts: std::array::from_fn(|_| None),
            level_one_exception_active: false,
            task_contexts: Arc::new(Mutex::new(TaskContexts::default())),
            floating_registers: [0; 16],
            special_registers: [0; 256],
            processor_id: Self::PROCESSOR_ID_CORE0,
            thread_pointer: 0,
            pc: 0,
            ps: 0,
            sar: 0,
            boolean_registers: 0,
            loop_begin: 0,
            loop_end: 0,
            loop_count: 0,
            loop_active: false,
            branch_taken: false,
            waiting: false,
            halted: false,
            interrupts: BTreeSet::new(),
            software_interrupts: 0,
        }
        .with_processor_id_register()
    }

    fn with_processor_id_register(mut self) -> Self {
        self.special_registers[235] = self.processor_id;
        self
    }

    /// Establishes a direct-load stack and entry point.
    pub fn set_direct_state(&mut self, stack: u32, entry: u32) {
        self.registers = [0; 16];
        self.window_stack.clear();
        self.interrupt_contexts = std::array::from_fn(|_| None);
        self.level_one_exception_active = false;
        self.floating_registers = [0; 16];
        self.special_registers = [0; 256];
        self.special_registers[235] = self.processor_id;
        self.thread_pointer = 0;
        self.registers[1] = stack;
        self.pc = entry;
        self.sar = 0;
        self.boolean_registers = 0;
        self.loop_begin = 0;
        self.loop_end = 0;
        self.loop_count = 0;
        self.loop_active = false;
        self.branch_taken = false;
        self.waiting = false;
        self.halted = false;
        self.interrupts.clear();
        self.software_interrupts = 0;
    }

    /// Establishes an ESP32-S3 verified-image handoff made through `CALLX8`.
    ///
    /// The application must begin with an `ENTRY` instruction; that
    /// instruction consumes the pending CALLINC state and creates the first
    /// logical register window.
    pub fn set_windowed_entry_state(&mut self, stack: u32, entry: u32) {
        self.set_direct_state(stack, entry);
        self.ps = 2 << 16;
    }

    /// Shares functional FreeRTOS task snapshots with another CPU.
    ///
    /// ESP-IDF may migrate a task between ESP32-S3's application cores. A
    /// saved task context therefore belongs to the machine, not to whichever
    /// core happened to take the preceding scheduler interrupt.
    pub fn share_task_contexts_from(&mut self, other: &Self) {
        self.task_contexts = Arc::clone(&other.task_contexts);
    }

    /// Selects the ESP32-S3 processor identity exposed through `PRID`.
    pub fn set_processor_id(&mut self, core: u8) {
        self.processor_id = match core {
            0 => Self::PROCESSOR_ID_CORE0,
            1 => Self::PROCESSOR_ID_CORE1,
            _ => panic!("ESP32-S3 processor identity must be core zero or one"),
        };
        self.special_registers[235] = self.processor_id;
    }

    /// Reads one visible address register.
    pub const fn register(&self, register: XtensaRegister) -> u32 {
        self.registers[register.index()]
    }

    /// Writes one visible address register for a functional architectural service.
    pub const fn set_register(&mut self, register: XtensaRegister, value: u32) {
        self.registers[register.index()] = value;
    }

    /// Returns the current instruction address.
    pub const fn pc(&self) -> u32 {
        self.pc
    }

    /// Replaces PS.INTLEVEL and returns the previous level.
    pub fn set_interrupt_level(&mut self, level: u32) -> u32 {
        let previous = self.ps & 0xf;
        self.ps = (self.ps & !0xf) | (level & 0xf);
        previous
    }

    /// Returns `(PS, INTERRUPT, INTENABLE)` for machine-level diagnostics.
    pub fn interrupt_state(&self) -> (u32, u32, u32) {
        (
            self.ps,
            self.special_registers[226],
            self.special_registers[228],
        )
    }

    /// Returns the visible PS value and logical register-window depth.
    pub fn window_state(&self) -> (u32, usize) {
        (self.ps, self.window_stack.len())
    }

    /// Reports whether the core has explicitly entered `WAITI`.
    pub const fn waiting_for_interrupt(&self) -> bool {
        self.waiting
    }

    /// Reports a deterministic shallow-window boundary suitable for deferred
    /// functional interrupt delivery.
    pub fn functional_interrupt_safe_point(&self) -> bool {
        self.waiting || self.window_stack.len() <= 2
    }

    fn pending_interrupt_level(&self, enabled_pending: u32) -> Option<(u32, u32)> {
        let current_level = self.ps & 0xf;
        let exception_mode = self.ps & Self::EXCM != 0;
        Self::INTERRUPT_LEVELS
            .into_iter()
            .find(|(level, mask, _)| {
                enabled_pending & mask != 0
                    && *level > current_level
                    && (!exception_mode || *level > Self::EXCM_LEVEL)
            })
            .map(|(level, _, vector_offset)| (level, vector_offset))
    }

    fn window_call(&mut self, call_increment: usize, target: u32, return_address: u32) {
        debug_assert!(matches!(call_increment, 4 | 8 | 12));
        let encoded_return =
            return_address | (u32::try_from(call_increment / 4).unwrap_or_default() << 30);
        self.registers[call_increment] = encoded_return;
        let caller = self.registers;
        let mut callee = [0_u32; 16];
        callee[..16 - call_increment].copy_from_slice(&caller[call_increment..]);
        // ENTRY computes the callee stack pointer from the caller's a1 while
        // rotating the argument window. Preserve that source explicitly; it
        // is not supplied through the ordinary overlapping a9/a13 slot.
        callee[1] = caller[1];
        self.window_stack.push(WindowFrame {
            registers: caller,
            return_address,
            call_increment,
        });
        self.registers = callee;
        self.pc = target;
    }

    fn window_return(&mut self) -> Result<(), CpuFault> {
        let frame = self.window_stack.pop().ok_or_else(|| {
            self.fault(
                CpuFaultKind::Architecture,
                "RETW executed without a windowed caller",
            )
        })?;
        let overlap = 16 - frame.call_increment;
        let mut caller = frame.registers;
        caller[frame.call_increment..].copy_from_slice(&self.registers[..overlap]);
        self.registers = caller;
        self.pc = frame.return_address;
        Ok(())
    }

    /// Completes a functional ROM routine using the active Xtensa ABI call.
    pub fn complete_functional_call(&mut self, result: u32) -> Result<(), CpuFault> {
        self.registers[2] = result;
        if self.window_stack.is_empty() {
            // A functional service reached without an emulated window frame
            // was called with CALL0/JX under the call0 ABI. Its a0 is a plain
            // 32-bit return address; only windowed calls encode CALLINC in the
            // top two bits, and those always have a WindowFrame above.
            self.pc = self.registers[0];
            Ok(())
        } else {
            self.window_return()
        }
    }

    /// Starts a windowed guest callback from a functional peripheral service.
    pub fn begin_functional_call(
        &mut self,
        target: u32,
        return_address: u32,
        arguments: &[u32],
    ) -> Result<(), CpuFault> {
        if arguments.len() > 6 {
            return Err(self.fault(
                CpuFaultKind::Unsupported,
                "functional Xtensa callback exceeds six register arguments",
            ));
        }
        self.window_call(8, target, return_address);
        for (index, argument) in arguments.iter().copied().enumerate() {
            self.registers[2 + index] = argument;
        }
        Ok(())
    }

    fn fault(&self, kind: CpuFaultKind, message: impl Into<String>) -> CpuFault {
        CpuFault::new(kind, u64::from(self.pc), message)
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
}

impl Default for XtensaCpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu for XtensaCpu {
    fn architecture(&self) -> Architecture {
        Architecture::XtensaLx7
    }

    fn reset(&mut self, _kind: ResetKind, _bus: &mut dyn Bus) -> Result<(), CpuFault> {
        self.registers = [0; 16];
        self.window_stack.clear();
        self.interrupt_contexts = std::array::from_fn(|_| None);
        self.level_one_exception_active = false;
        {
            let mut contexts = self
                .task_contexts
                .lock()
                .expect("Xtensa task-context lock poisoned");
            contexts.window_stacks.clear();
            contexts.registers.clear();
        }
        self.floating_registers = [0; 16];
        self.special_registers = [0; 256];
        self.special_registers[235] = self.processor_id;
        self.thread_pointer = 0;
        self.pc = 0;
        self.ps = 0;
        self.sar = 0;
        self.boolean_registers = 0;
        self.loop_begin = 0;
        self.loop_end = 0;
        self.loop_count = 0;
        self.loop_active = false;
        self.branch_taken = false;
        self.waiting = false;
        self.halted = false;
        self.interrupts.clear();
        self.software_interrupts = 0;
        Ok(())
    }

    fn step(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<StepOutcome, CpuFault> {
        if self.halted {
            return Ok(StepOutcome {
                elapsed: SimDuration::ZERO,
                reason: StepReason::Halted,
            });
        }
        // CCOUNT is a free-running architectural cycle counter. One
        // functional instruction tick is sufficient for deterministic delay
        // loops and entropy-mixing code without claiming silicon timing.
        self.special_registers[234] = self.special_registers[234].wrapping_add(1);
        // CCOMPARE0 raises the ESP32-S3 architectural timer interrupt. The
        // interrupt remains pending until firmware programs the next compare
        // value, matching the Xtensa timer contract used by FreeRTOS.
        if self.special_registers[234] == self.special_registers[240] {
            self.software_interrupts |= 1 << 6;
            self.special_registers[226] |= 1 << 6;
        }
        let enabled_pending = self.special_registers[226] & self.special_registers[228];
        if let Some((interrupt_level, vector_offset)) =
            self.pending_interrupt_level(enabled_pending)
        {
            self.waiting = false;
            if std::env::var_os("REMU_DEBUG_XTENSA_CONTEXT").is_some() {
                eprintln!(
                    "irq core={:#06x} level={interrupt_level} pc={:#010x} ps={:#010x} tp={:#010x} depth={} bits={enabled_pending:#010x} loop={:#010x}..{:#010x}/{} active={}",
                    self.processor_id,
                    self.pc,
                    self.ps,
                    self.thread_pointer,
                    self.window_stack.len(),
                    self.loop_begin,
                    self.loop_end,
                    self.loop_count,
                    self.loop_active,
                );
            }
            if interrupt_level == 1 {
                {
                    let mut contexts = self
                        .task_contexts
                        .lock()
                        .expect("Xtensa task-context lock poisoned");
                    contexts
                        .window_stacks
                        .insert(self.thread_pointer, self.window_stack.clone());
                    contexts
                        .registers
                        .insert(self.thread_pointer, self.registers);
                }
                self.window_stack.clear();
                self.level_one_exception_active = true;
                self.special_registers[177] = self.pc;
                self.special_registers[193] = self.ps;
                self.special_registers[232] = 4;
                // A level-one interrupt enters through the user-exception
                // path: EXCM is asserted, while PS.INTLEVEL still describes
                // the interrupted context. The vector raises INTLEVEL itself
                // after saving that value into the task frame.
                self.ps |= Self::EXCM;
            } else {
                // Medium/high interrupts have dedicated EPC/EPS banks and
                // vectors.  They do not enter exception mode or disturb the
                // level-one task-window snapshot; their assembly prologue
                // saves the live registers through EXCSAVEn before calling a
                // handler and returns with RFI n.
                self.special_registers[176 + interrupt_level as usize] = self.pc;
                self.special_registers[192 + interrupt_level as usize] = self.ps;
                self.interrupt_contexts[interrupt_level as usize] = Some(InterruptContext {
                    registers: self.registers,
                    window_stack: self.window_stack.clone(),
                    thread_pointer: self.thread_pointer,
                });
                self.ps = (self.ps & !0xf) | interrupt_level;
            }
            self.pc = self.special_registers[231].wrapping_add(vector_offset);
            return Ok(StepOutcome {
                elapsed: SimDuration::TICK,
                reason: StepReason::Advanced,
            });
        }
        if self.waiting {
            return Ok(StepOutcome {
                elapsed: SimDuration::TICK,
                reason: StepReason::WaitForInterrupt,
            });
        }
        let first = self.read(bus, self.pc, AccessWidth::Byte, AccessKind::Execute, now)?;
        let halfword = self.read(
            bus,
            self.pc,
            AccessWidth::HalfWord,
            AccessKind::Execute,
            now,
        )? as u16;
        let instruction_pc = self.pc;
        self.branch_taken = false;
        let narrow = matches!(first & 0xf, 0x8..=0xd);
        let sequential_pc = instruction_pc.wrapping_add(if narrow { 2 } else { 3 });
        let reason = if halfword == 0xf01d {
            self.window_return()?;
            StepReason::Advanced
        } else if halfword == 0xf00d {
            self.pc = self.registers[0];
            StepReason::Advanced
        } else if narrow {
            let instruction = halfword;
            self.execute_narrow(instruction, bus, now)?
        } else {
            let low = u32::from(halfword);
            let high = self.read(
                bus,
                self.pc.wrapping_add(2),
                AccessWidth::Byte,
                AccessKind::Execute,
                now,
            )?;
            self.execute_wide(low | (high << 16), bus, now)?
        };
        // LBEG/LEND/LCOUNT survive an interrupt.  The exception vector runs
        // outside the interrupted loop range and may save/restore those
        // special registers before RFE, so only advance or abandon a loop
        // after an instruction that was actually in its body.  Treating the
        // first vector instruction as a branch out of the body truncates ROM
        // memcpy/memset loops whenever the FreeRTOS tick arrives.
        let executed_in_loop =
            self.loop_active && instruction_pc >= self.loop_begin && instruction_pc < self.loop_end;
        if matches!(reason, StepReason::Advanced) && executed_in_loop {
            if self.pc == self.loop_end && self.pc == sequential_pc && !self.branch_taken {
                // LCOUNT is the architectural remaining-iterations-minus-one
                // value loaded by LOOP.
                if self.loop_count != 0 {
                    self.loop_count = self.loop_count.wrapping_sub(1);
                    self.pc = self.loop_begin;
                } else {
                    self.loop_active = false;
                }
            } else if self.pc < self.loop_begin || self.pc >= self.loop_end {
                // A taken control transfer escaped the hardware-loop body.
                // Landing on LEND is not the same as reaching it by ordinary
                // sequential execution.
                self.loop_begin = 0;
                self.loop_end = 0;
                self.loop_count = 0;
                self.loop_active = false;
            }
        }
        Ok(StepOutcome {
            elapsed: SimDuration::TICK,
            reason,
        })
    }

    fn set_interrupt(&mut self, line: u16, asserted: bool) -> Result<(), CpuFault> {
        if line >= 32 {
            return Err(self.fault(
                CpuFaultKind::Unsupported,
                "Xtensa interrupt line exceeds modeled range",
            ));
        }
        if asserted {
            self.interrupts.insert(line);
            self.waiting = false;
        } else {
            self.interrupts.remove(&line);
        }
        let mut pending = self.software_interrupts;
        for interrupt in &self.interrupts {
            pending |= 1_u32 << u32::from(*interrupt);
        }
        self.special_registers[226] = pending;
        Ok(())
    }

    fn snapshot(&self) -> CpuSnapshot {
        let mut registers = self
            .registers
            .iter()
            .enumerate()
            .map(|(index, value)| RegisterValue {
                name: format!("a{index}"),
                value: u64::from(*value),
                bits: 32,
            })
            .collect::<Vec<_>>();
        registers.push(RegisterValue {
            name: "ps".to_owned(),
            value: u64::from(self.ps),
            bits: 32,
        });
        registers.push(RegisterValue {
            name: "sar".to_owned(),
            value: u64::from(self.sar),
            bits: 32,
        });
        registers.push(RegisterValue {
            name: "threadptr".to_owned(),
            value: u64::from(self.thread_pointer),
            bits: 32,
        });
        registers.push(RegisterValue {
            name: "window_depth".to_owned(),
            value: self.window_stack.len() as u64,
            bits: 32,
        });
        for (name, index) in [
            ("epc1", 177),
            ("eps1", 193),
            ("interrupt", 226),
            ("intenable", 228),
            ("exccause", 232),
            ("ccount", 234),
            ("prid", 235),
            ("excvaddr", 238),
        ] {
            registers.push(RegisterValue {
                name: name.to_owned(),
                value: u64::from(self.special_registers[index]),
                bits: 32,
            });
        }
        CpuSnapshot {
            architecture: Architecture::XtensaLx7,
            pc: u64::from(self.pc),
            registers,
            waiting: self.waiting,
            halted: self.halted,
        }
    }
}

fn sign_extend(value: u32, bits: u32) -> i32 {
    ((value << (32 - bits)) as i32) >> (32 - bits)
}

#[cfg(test)]
mod tests;
