use super::*;

/// Arm machine construction or execution failure.
#[derive(Debug, Error)]
pub enum ArmMachineError {
    /// Target does not have an implemented Arm mode.
    #[error("target {0} does not currently have a runnable Arm M-profile")]
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
    /// Firmware has the wrong ELF architecture.
    #[error("firmware architecture {actual:?} does not match Arm target {target}")]
    Architecture {
        /// Target being loaded.
        target: TargetId,
        /// ELF architecture.
        actual: FirmwareArchitecture,
    },
    /// Entry does not fit a 32-bit address.
    #[error("firmware entry {0:#x} exceeds the Arm address space")]
    EntryRange(u64),
    /// Segment does not fall within the target map.
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
    /// Machine configuration request is not valid for this target.
    #[error("invalid Arm machine configuration: {0}")]
    Configuration(String),
    /// UF2 parsing or flash reconstruction failed.
    #[error(transparent)]
    Uf2(#[from] Uf2Error),
    /// The UF2 target family does not match the selected processor.
    #[error("UF2 family {actual:#010x} does not match {target}; expected {expected:#010x}")]
    Uf2Family {
        /// Selected machine.
        target: TargetId,
        /// Family identifier required by that machine.
        expected: u32,
        /// Family identifier present in the artifact.
        actual: u32,
    },
    /// A boot-ROM handoff is not implemented for the selected target.
    #[error("boot-ROM handoff is not implemented for {0}")]
    BootHandoffUnsupported(TargetId),
    /// An official image contains an invalid post-boot vector table.
    #[error("invalid {target} vector table at {vector_base:#010x}: {message}")]
    BootVector {
        /// Selected machine.
        target: TargetId,
        /// Address of the vector table.
        vector_base: u32,
        /// Validation diagnostic.
        message: String,
    },
}
