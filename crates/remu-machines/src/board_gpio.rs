//! Typed GPIO bridge between a machine signal hub and board components.

use crate::PinStimulus;
use crate::board::{BoardAction, BoardComponentKind, BoardError, BoardScenario};
use remu_core::SimTime;
use remu_devices::SignalHub;
use remu_signals::{Logic, SignalValue};
use std::collections::BTreeMap;

struct GpioComponentEndpoint {
    source: remu_signals::SignalId,
    component_pin: remu_signals::SignalId,
    state: remu_signals::SignalId,
    active_low: Option<bool>,
}

#[derive(Clone, Copy)]
struct EndpointButton {
    pin: u8,
    active_low: bool,
    bounce_ticks: u64,
}

/// Typed bridge between machine GPIO signals and mounted digital board parts.
///
/// A machine scheduler owns the endpoint and calls [`Self::poll`] at its
/// deterministic time boundary. Button actions are converted to
/// [`PinStimulus`] values, while LED state follows the resolved machine GPIO
/// signal and is published under the board component hierarchy. Protocol
/// connections and waveform-driven parts remain separate endpoint slices.
pub struct BoardGpioEndpoint {
    hub: SignalHub,
    components: Vec<GpioComponentEndpoint>,
    buttons: BTreeMap<String, EndpointButton>,
}

impl BoardGpioEndpoint {
    /// Attaches mounted buttons and LEDs to paths such as
    /// `board.esp32c6.chip_gpio.pin7` in an existing machine signal hub.
    pub fn new(
        scenario: &BoardScenario,
        hub: SignalHub,
        source_prefix: &str,
    ) -> Result<Self, BoardError> {
        super::board::validate(scenario)?;
        validate_gpio_mounts(scenario)?;
        if let Some(connection) = scenario.connections.first() {
            return Err(BoardError::GpioComponent {
                component: connection.component.name.clone(),
                kind: "external protocol",
            });
        }
        let mut components = Vec::with_capacity(scenario.mounts.len());
        let mut buttons = BTreeMap::new();
        for mount in &scenario.mounts {
            let (active_low, kind) = match mount.component.kind {
                BoardComponentKind::PushButton {
                    active_low,
                    bounce_ticks,
                } => {
                    buttons.insert(
                        mount.component.name.clone(),
                        EndpointButton {
                            pin: mount.pin,
                            active_low,
                            bounce_ticks,
                        },
                    );
                    (Some(active_low), "push-button")
                }
                BoardComponentKind::Led { active_low } => (Some(active_low), "LED"),
                BoardComponentKind::Ws2812 { .. } => {
                    return Err(BoardError::GpioComponent {
                        component: mount.component.name.clone(),
                        kind: "WS2812",
                    });
                }
                BoardComponentKind::Sgp30 { .. } => {
                    return Err(BoardError::GpioComponent {
                        component: mount.component.name.clone(),
                        kind: "SGP30",
                    });
                }
                BoardComponentKind::M5Pm1 => {
                    return Err(BoardError::GpioComponent {
                        component: mount.component.name.clone(),
                        kind: "M5PM1",
                    });
                }
                BoardComponentKind::Bmi270 => {
                    return Err(BoardError::GpioComponent {
                        component: mount.component.name.clone(),
                        kind: "BMI270",
                    });
                }
                BoardComponentKind::Es8311 => {
                    return Err(BoardError::GpioComponent {
                        component: mount.component.name.clone(),
                        kind: "ES8311",
                    });
                }
                BoardComponentKind::St7789 { .. } => {
                    return Err(BoardError::GpioComponent {
                        component: mount.component.name.clone(),
                        kind: "ST7789",
                    });
                }
            };
            let source_path = format!("{source_prefix}.pin{}", mount.pin);
            let source = hub
                .with_registry(|registry| registry.find(&source_path))
                .ok_or_else(|| remu_signals::SignalError::UnknownPath(source_path.clone()))?;
            let initial = hub.with_registry(|registry| {
                registry
                    .value(source)
                    .and_then(|value| value.bit(0))
                    .unwrap_or(Logic::X)
            });
            let component_prefix =
                format!("board.{}.component.{}", scenario.name, mount.component.name);
            let component_pin = hub.declare(
                format!("{component_prefix}.pin"),
                SignalValue::repeat(initial, 1)?,
                Some("resolved machine GPIO level".to_owned()),
            )?;
            let state = hub.declare(
                format!("{component_prefix}.state"),
                SignalValue::from_u64(
                    u64::from(active_low.is_some_and(|active| led_is_on(initial, active))),
                    1,
                )?,
                Some(format!("live {kind} state")),
            )?;
            components.push(GpioComponentEndpoint {
                source,
                component_pin,
                state,
                active_low,
            });
        }
        Ok(Self {
            hub,
            components,
            buttons,
        })
    }

    /// Converts board button actions into target GPIO input stimuli.
    pub fn button_stimuli(&self, actions: &[BoardAction]) -> Result<Vec<PinStimulus>, BoardError> {
        let mut stimuli = Vec::new();
        for action in actions {
            let BoardAction::Press {
                component,
                at,
                duration,
            } = action
            else {
                continue;
            };
            let button = self
                .buttons
                .get(component)
                .copied()
                .ok_or_else(|| BoardError::GpioButton(component.clone()))?;
            append_button_edge(&mut stimuli, button, true, SimTime::from_ticks(*at));
            append_button_edge(
                &mut stimuli,
                button,
                false,
                SimTime::from_ticks(at.saturating_add(*duration)),
            );
        }
        stimuli.sort_by_key(|stimulus| stimulus.at);
        Ok(stimuli)
    }

    /// Mirrors resolved machine GPIO values into board-component signals.
    pub fn poll(&self, at: SimTime) -> Result<(), BoardError> {
        for component in &self.components {
            let value = self.hub.with_registry(|registry| {
                registry
                    .value(component.source)
                    .and_then(|signal| signal.bit(0))
                    .unwrap_or(Logic::X)
            });
            self.hub
                .set(component.component_pin, SignalValue::repeat(value, 1)?, at)?;
            if let Some(active_low) = component.active_low {
                self.hub.set(
                    component.state,
                    SignalValue::from_u64(u64::from(led_is_on(value, active_low)), 1)?,
                    at,
                )?;
            }
        }
        Ok(())
    }
}

fn led_is_on(level: Logic, active_low: bool) -> bool {
    level == if active_low { Logic::Zero } else { Logic::One }
}

fn append_button_edge(
    stimuli: &mut Vec<PinStimulus>,
    button: EndpointButton,
    pressed: bool,
    at: SimTime,
) {
    let old_level = if button.active_low == !pressed {
        Logic::Zero
    } else {
        Logic::One
    };
    let final_level = if button.active_low == pressed {
        Logic::Zero
    } else {
        Logic::One
    };
    if button.bounce_ticks == 0 {
        stimuli.push(PinStimulus {
            at,
            pin: button.pin,
            value: final_level,
        });
        return;
    }
    let offsets = [
        0,
        button.bounce_ticks / 4,
        button.bounce_ticks / 2,
        button.bounce_ticks.saturating_mul(3) / 4,
        button.bounce_ticks,
    ];
    for (offset, value) in
        offsets
            .into_iter()
            .zip([final_level, old_level, final_level, old_level, final_level])
    {
        stimuli.push(PinStimulus {
            at: SimTime::from_ticks(at.ticks().saturating_add(offset)),
            pin: button.pin,
            value,
        });
    }
}

fn validate_gpio_mounts(scenario: &BoardScenario) -> Result<(), BoardError> {
    let mut pins = BTreeMap::new();
    for mount in &scenario.mounts {
        if let Some(first) = pins.insert(mount.pin, mount.component.name.clone()) {
            return Err(BoardError::GpioPinConflict {
                pin: mount.pin,
                first,
                second: mount.component.name.clone(),
            });
        }
    }
    Ok(())
}
