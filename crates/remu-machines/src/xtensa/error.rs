use crate::TargetId;
use remu_bus::MapError;
use remu_core::CpuFault;
use remu_image::FirmwareArchitecture;
use remu_signals::SignalError;
use remu_trace::TraceError;
use thiserror::Error;

/// ESP32-S3 machine construction or execution failure.
#[derive(Debug, Error)]
pub enum XtensaMachineError {
    /// Only ESP32-S3 uses this initial LX7 machine.
    #[error("target {0} does not have the runnable Xtensa LX7 profile")]
    UnsupportedTarget(TargetId),
    /// Address map construction failed.
    #[error(transparent)]
    Map(#[from] MapError),
    /// CPU operation failed.
    #[error(transparent)]
    Cpu(#[from] CpuFault),
    /// Signal construction failed.
    #[error(transparent)]
    Signal(#[from] SignalError),
    /// Host peripheral operation failed.
    #[error(transparent)]
    Device(#[from] remu_bus::DeviceError),
    /// Trace output failed.
    #[error(transparent)]
    Trace(#[from] TraceError),
    /// Firmware has the wrong architecture.
    #[error("firmware architecture {0:?} does not match ESP32-S3 Xtensa")]
    Architecture(FirmwareArchitecture),
    /// Entry exceeds 32-bit address space.
    #[error("firmware entry {0:#x} exceeds the Xtensa address space")]
    EntryRange(u64),
    /// Segment is outside the direct-load map.
    #[error("cannot load firmware segment at {address:#x}: {message}")]
    Load {
        /// Segment start.
        address: u64,
        /// Bus diagnostic.
        message: String,
    },
    /// Runs must be bounded.
    #[error("at least one run limit is required")]
    MissingRunLimit,
    /// Virtual time overflowed.
    #[error("simulation time overflow")]
    TimeOverflow,
}
