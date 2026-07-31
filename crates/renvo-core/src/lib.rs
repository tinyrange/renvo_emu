//! Architecture-neutral contracts and deterministic simulation primitives.

mod bus;
mod cpu;
mod event;
mod run;
mod time;

pub use bus::{AccessKind, AccessWidth, Bus, BusFault, BusFaultKind};
pub use cpu::{
    Architecture, Cpu, CpuFault, CpuFaultKind, CpuSnapshot, RegisterValue, ResetKind, StepOutcome,
    StepReason,
};
pub use event::{EventId, EventQueue, QueueError, ScheduledEvent};
pub use run::{RunLimits, RunStats, StopReason};
pub use time::{SimDuration, SimTime, TimeError};
