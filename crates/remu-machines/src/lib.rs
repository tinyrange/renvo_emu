//! Chip and board composition for the six initial Renvo Emulator targets.
#![allow(clippy::too_many_lines)]

use remu_core::SimTime;
use remu_devices::SignalHub;
use remu_signals::{Logic, SignalChange, SignalError, SignalId};
use serde::Serialize;

mod arm;
mod arm_mcu;
mod avr_mcu;
mod board;
mod board_gpio;
mod board_uart;
mod m5sticks3;
mod m5sticks3_firmware;
mod mcs51_mcu;
mod msp430_mcu;
mod native_wifi;
mod pic16_mcu;
mod radio_rom;
mod riscv;
mod run_control;
mod target;
mod uart;
mod xtensa;

pub use arm::{ArmMachine, ArmMachineError};
pub use arm_mcu::ArmMcuMachine;
pub use avr_mcu::{AvrMachineError, AvrMcuMachine};
pub use board::*;
pub use board_uart::*;
pub use m5sticks3::{M5StickS3Button, M5StickS3Handle, M5StickS3Snapshot};
pub use m5sticks3_firmware::{
    M5StickS3FirmwareError, M5StickS3FirmwareResult, run_m5sticks3_firmware_scenario,
};
pub use mcs51_mcu::{Mcs51MachineError, Mcs51McuMachine};
pub use msp430_mcu::{Msp430MachineError, Msp430McuMachine};
pub use pic16_mcu::{Pic16MachineError, Pic16McuMachine};
pub use radio_rom::{
    ESP32C6_RADIO_ROM_SHA256, ESP32S3_RADIO_ROM_SHA256, EspRadioRomError, verify_esp_radio_rom,
};
pub use riscv::{
    MachineError, RiscVMachine, RunResult, TEST_EXIT, TEST_GPIO, TEST_TIMER, TEST_UART,
};
pub use target::{
    CpuOption, Fidelity, MemoryKind, MemoryRegion, TargetId, TargetManifest, target_manifest,
    target_manifests,
};
pub use uart::{UartEndpoint, UartEndpointId, UartEndpointProvider};
pub use xtensa::{
    Esp32S3BootMapping, Esp32S3BootReport, Esp32S3BootSegment, Esp32S3BootSegmentKind,
    XtensaMachine, XtensaMachineError, plan_esp32s3_boot,
};

/// Deterministic raw-REPL marker emitted by the CLI's final framing chunk.
pub const HOST_SCRIPT_COMPLETE_MARKER: &str = "__REMU_HOST_SCRIPT_COMPLETE__";

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

/// Edge condition used by a named signal stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SignalEdge {
    /// Any real value transition.
    Change,
    /// The low bit changes from a non-one state to one.
    Rising,
    /// The low bit changes from a non-zero state to zero.
    Falling,
}

#[derive(Clone, Debug)]
pub(crate) struct SignalStop {
    signal: SignalId,
    path: String,
    edge: SignalEdge,
}

pub(crate) fn resolve_signal_stop(
    hub: &SignalHub,
    path: &str,
    edge: SignalEdge,
) -> Result<SignalStop, SignalError> {
    let signal = hub
        .with_registry(|registry| registry.find(path))
        .ok_or_else(|| SignalError::UnknownPath(path.to_owned()))?;
    Ok(SignalStop {
        signal,
        path: path.to_owned(),
        edge,
    })
}

pub(crate) fn matching_signal_stop(change: &SignalChange, stops: &[SignalStop]) -> Option<String> {
    stops
        .iter()
        .find(|stop| {
            if stop.signal != change.signal {
                return false;
            }
            match stop.edge {
                SignalEdge::Change => change.previous != change.value,
                SignalEdge::Rising => {
                    change.previous.bit(0) != Some(Logic::One)
                        && change.value.bit(0) == Some(Logic::One)
                }
                SignalEdge::Falling => {
                    change.previous.bit(0) != Some(Logic::Zero)
                        && change.value.bit(0) == Some(Logic::Zero)
                }
            }
        })
        .map(|stop| stop.path.clone())
}
