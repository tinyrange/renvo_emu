//! End-to-end `M5StickS3` firmware and board-component execution.

use crate::{
    BoardAction, BoardComponentKind, BoardScenario, M5StickS3Button, M5StickS3Snapshot,
    PinStimulus, RunResult, TargetId, XtensaMachine, XtensaMachineError,
};
use remu_core::{RunLimits, SimTime};
use remu_image::FirmwareImage;
use remu_signals::Logic;
use remu_trace::TraceSink;
use serde::Serialize;
use thiserror::Error;

/// Failure while binding a declarative board to a live ESP32-S3 machine.
#[derive(Debug, Error)]
pub enum M5StickS3FirmwareError {
    /// The scenario does not describe the supported `M5StickS3` fixture.
    #[error("live M5StickS3 firmware requires board=m5sticks3 and target=esp32s3")]
    Scenario,
    /// A button action references a non-button component.
    #[error("M5StickS3 action references unknown button {0:?}")]
    Button(String),
    /// ESP32-S3 construction, attachment, or execution failed.
    #[error(transparent)]
    Machine(#[from] XtensaMachineError),
}

/// Stable result combining CPU execution and physical board state.
#[derive(Clone, Debug, Serialize)]
pub struct M5StickS3FirmwareResult {
    /// Artifact schema.
    pub schema: &'static str,
    /// Declarative board name.
    pub board: String,
    /// ESP32-S3 CPU and transport result.
    pub run: RunResult,
    /// Final state of the display, power controller, and buttons.
    pub components: M5StickS3Snapshot,
    /// Pass marker.
    pub result: &'static str,
}

/// Runs an Xtensa ELF against the live `M5StickS3` component graph.
pub fn run_m5sticks3_firmware_scenario(
    scenario: &BoardScenario,
    firmware: &FirmwareImage,
    limits: RunLimits,
    trace: Option<&mut dyn TraceSink>,
) -> Result<M5StickS3FirmwareResult, M5StickS3FirmwareError> {
    if scenario.name != "m5sticks3" || scenario.target != "esp32s3" {
        return Err(M5StickS3FirmwareError::Scenario);
    }
    let mut machine = XtensaMachine::new(TargetId::Esp32s3)?;
    let board = machine.attach_m5sticks3()?;
    board
        .set_imu_sample([100, -200, 16_000], [5, 6, 7], 512, SimTime::ZERO)
        .map_err(XtensaMachineError::from)?;
    board.set_microphone_sample(0x1234_5678);
    board
        .set_ir_receiver(true, SimTime::ZERO)
        .map_err(XtensaMachineError::from)?;
    machine.load_firmware(firmware)?;
    let stimuli = button_stimuli(scenario)?;
    let run = machine.run_with_stimuli(limits, &stimuli, trace)?;
    let components = board.snapshot().map_err(XtensaMachineError::from)?;
    Ok(M5StickS3FirmwareResult {
        schema: "remu.m5sticks3-firmware-board.v1",
        board: scenario.name.clone(),
        run,
        components,
        result: "pass",
    })
}

fn button_stimuli(scenario: &BoardScenario) -> Result<Vec<PinStimulus>, M5StickS3FirmwareError> {
    let mut stimuli = Vec::new();
    for action in &scenario.actions {
        let BoardAction::Press {
            component,
            at,
            duration,
        } = action
        else {
            continue;
        };
        let (button, active_low, bounce_ticks) = scenario
            .mounts
            .iter()
            .find(|mount| mount.component.name == *component)
            .and_then(|mount| match mount.component.kind {
                BoardComponentKind::PushButton {
                    active_low,
                    bounce_ticks,
                } => Some((
                    match mount.pin {
                        11 => M5StickS3Button::A,
                        12 => M5StickS3Button::B,
                        _ => return None,
                    },
                    active_low,
                    bounce_ticks,
                )),
                _ => None,
            })
            .ok_or_else(|| M5StickS3FirmwareError::Button(component.clone()))?;
        append_button_edge(&mut stimuli, button, active_low, bounce_ticks, true, *at);
        append_button_edge(
            &mut stimuli,
            button,
            active_low,
            bounce_ticks,
            false,
            at.saturating_add(*duration),
        );
    }
    stimuli.sort_by_key(|stimulus| stimulus.at);
    Ok(stimuli)
}

fn append_button_edge(
    stimuli: &mut Vec<PinStimulus>,
    button: M5StickS3Button,
    active_low: bool,
    bounce_ticks: u64,
    pressed: bool,
    at: u64,
) {
    let level = |pressed| {
        if active_low == pressed {
            Logic::Zero
        } else {
            Logic::One
        }
    };
    let final_level = level(pressed);
    if bounce_ticks == 0 {
        stimuli.push(PinStimulus {
            at: SimTime::from_ticks(at),
            pin: button.pin(),
            value: final_level,
        });
        return;
    }
    let old_level = level(!pressed);
    let interval = (bounce_ticks / 4).max(1);
    for (index, value) in [final_level, old_level, final_level, old_level, final_level]
        .into_iter()
        .enumerate()
    {
        stimuli.push(PinStimulus {
            at: SimTime::from_ticks(at.saturating_add(interval.saturating_mul(index as u64))),
            pin: button.pin(),
            value,
        });
    }
}
