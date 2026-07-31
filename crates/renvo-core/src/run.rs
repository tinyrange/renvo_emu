use crate::SimTime;
use serde::{Deserialize, Serialize};

/// Deterministic run budgets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLimits {
    /// Maximum number of interpreted instructions or CPU actions.
    pub instructions: Option<u64>,
    /// Inclusive simulation-time deadline.
    pub deadline: Option<SimTime>,
}

/// Terminal simulation outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    /// CPU executed an explicit halt or machine exit convention.
    Halted,
    /// Instruction budget was exhausted.
    InstructionLimit,
    /// Virtual-time deadline was reached.
    TimeLimit,
    /// CPU or bus fault with a stable diagnostic.
    Fault(String),
    /// User breakpoint.
    Breakpoint,
    /// Named signal edge or condition.
    Signal(String),
    /// Machine has no runnable CPU and no pending events.
    Quiescent,
    /// The queued deterministic host input completed and the guest returned to its prompt.
    HostInputComplete,
}

/// Stable execution counters emitted with run artifacts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStats {
    /// Interpreted CPU instructions or actions.
    pub instructions: u64,
    /// Final simulation timestamp.
    pub time: SimTime,
    /// Number of scheduled device events dispatched.
    pub events: u64,
}
