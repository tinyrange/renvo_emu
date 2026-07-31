use crate::{Bus, SimDuration, SimTime};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// CPU architecture family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Architecture {
    /// 32-bit RISC-V profile.
    RiscV32,
    /// Arm M-profile.
    ArmM,
    /// Xtensa LX7 profile.
    XtensaLx7,
    /// Test-only architecture.
    Synthetic,
}

/// Reset source visible to the CPU model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResetKind {
    /// Initial power-on reset.
    PowerOn,
    /// External reset pin.
    External,
    /// Software-requested system reset.
    Software,
    /// Watchdog reset.
    Watchdog,
}

/// Result of one interpreted CPU step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StepOutcome {
    /// Approximate elapsed simulation time.
    pub elapsed: SimDuration,
    /// Why the step returned to the simulation kernel.
    pub reason: StepReason,
}

impl StepOutcome {
    /// Ordinary completed instruction.
    pub const fn advanced(elapsed: SimDuration) -> Self {
        Self {
            elapsed,
            reason: StepReason::Advanced,
        }
    }
}

/// Reason that a CPU step yielded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StepReason {
    /// One instruction or architectural action completed.
    Advanced,
    /// CPU is waiting for an interrupt or event.
    WaitForInterrupt,
    /// CPU executed its halt convention.
    Halted,
    /// An enabled breakpoint was reached before execution.
    Breakpoint,
}

/// CPU execution failure classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CpuFaultKind {
    /// Instruction encoding is not valid for the selected profile.
    IllegalInstruction,
    /// CPU bus access failed.
    Bus,
    /// Architectural invariant or state transition failed.
    Architecture,
    /// Simulation implementation detected an unsupported operation.
    Unsupported,
}

/// Structured CPU execution failure.
#[derive(Clone, Debug, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("{kind:?} CPU fault at PC {pc:#010x}: {message}")]
pub struct CpuFault {
    /// Failure classification.
    pub kind: CpuFaultKind,
    /// Program counter associated with the failure.
    pub pc: u64,
    /// Diagnostic details.
    pub message: String,
}

impl CpuFault {
    /// Constructs a CPU fault.
    pub fn new(kind: CpuFaultKind, pc: u64, message: impl Into<String>) -> Self {
        Self {
            kind,
            pc,
            message: message.into(),
        }
    }
}

/// Named architectural register value used by debuggers and artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterValue {
    /// Architecture-defined register name.
    pub name: String,
    /// Unsigned bit pattern.
    pub value: u64,
    /// Number of meaningful low bits.
    pub bits: u8,
}

/// Architecture-neutral CPU state snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CpuSnapshot {
    /// CPU architecture family.
    pub architecture: Architecture,
    /// Current instruction address.
    pub pc: u64,
    /// Named registers in deterministic display order.
    pub registers: Vec<RegisterValue>,
    /// Whether the CPU is currently waiting.
    pub waiting: bool,
    /// Whether the CPU is halted.
    pub halted: bool,
}

/// Interpreted CPU contract used by machine models.
pub trait Cpu {
    /// Returns the architecture family.
    fn architecture(&self) -> Architecture;

    /// Applies an architectural reset.
    fn reset(&mut self, kind: ResetKind, bus: &mut dyn Bus) -> Result<(), CpuFault>;

    /// Executes one instruction or one pending architectural action.
    fn step(&mut self, bus: &mut dyn Bus, now: SimTime) -> Result<StepOutcome, CpuFault>;

    /// Sets or clears a numbered external interrupt input.
    fn set_interrupt(&mut self, line: u16, asserted: bool) -> Result<(), CpuFault>;

    /// Captures inspectable architectural state.
    fn snapshot(&self) -> CpuSnapshot;
}
