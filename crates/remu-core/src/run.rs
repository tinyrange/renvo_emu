use crate::{AccessKind, SimTime};
use serde::{Deserialize, Serialize};

/// Deterministic run budgets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLimits {
    /// Maximum number of interpreted instructions or CPU actions.
    pub instructions: Option<u64>,
    /// Inclusive simulation-time deadline.
    pub deadline: Option<SimTime>,
}

impl RunLimits {
    /// Returns whether this invocation has at least one finite execution bound.
    pub const fn is_bounded(self) -> bool {
        self.instructions.is_some() || self.deadline.is_some()
    }

    /// Returns the stable stop reason for a reached instruction/deadline bound.
    ///
    /// Keeping this decision in the simulation core ensures every architecture
    /// checks inclusive deadlines and instruction budgets in the same order.
    pub fn reached(self, instructions: u64, time: SimTime) -> Option<StopReason> {
        if self.instructions.is_some_and(|limit| instructions >= limit) {
            Some(StopReason::InstructionLimit)
        } else if self.deadline.is_some_and(|deadline| time >= deadline) {
            Some(StopReason::TimeLimit)
        } else {
            None
        }
    }
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
    /// A watched data address was read or written.
    Watchpoint {
        /// First byte address of the triggering access.
        address: u64,
        /// Read or write operation that triggered the stop.
        access: AccessKind,
    },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_run_limits_prioritize_instruction_budget() {
        let limits = RunLimits {
            instructions: Some(3),
            deadline: Some(SimTime::from_ticks(3)),
        };
        assert!(limits.is_bounded());
        assert_eq!(
            limits.reached(3, SimTime::from_ticks(3)),
            Some(StopReason::InstructionLimit)
        );
        assert_eq!(
            limits.reached(2, SimTime::from_ticks(3)),
            Some(StopReason::TimeLimit)
        );
        assert_eq!(limits.reached(2, SimTime::from_ticks(2)), None);
    }

    #[test]
    fn unbounded_limits_are_rejected_by_callers() {
        assert!(!RunLimits::default().is_bounded());
    }
}
