//! Chip and board composition for the six initial Renvo targets.
#![allow(clippy::too_many_lines)]

use renvo_core::SimTime;
use renvo_signals::Logic;
use serde::Serialize;

mod arm;
mod riscv;
mod target;
mod xtensa;

pub use arm::{ArmMachine, ArmMachineError};
pub use riscv::{
    MachineError, RiscVMachine, RunResult, TEST_EXIT, TEST_GPIO, TEST_TIMER, TEST_UART,
};
pub use target::{
    CpuOption, Fidelity, MemoryKind, MemoryRegion, TargetId, TargetManifest, target_manifest,
    target_manifests,
};
pub use xtensa::{XtensaMachine, XtensaMachineError};

/// One deterministic external GPIO drive or release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct PinStimulus {
    /// Simulation timestamp at which the drive changes.
    pub at: SimTime,
    /// Zero-based pin number in the target's primary exposed bank.
    pub pin: u8,
    /// Four-state value to drive.
    pub value: Logic,
}
