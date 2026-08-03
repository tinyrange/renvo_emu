//! Live GPIO endpoint for mounted board components.

use crate::board::validate;
use crate::{BoardAction, BoardComponentKind, BoardError, BoardScenario, PinStimulus};
use remu_core::SimTime;
use remu_devices::SignalHub;
use remu_signals::{Logic, SignalError, SignalId, SignalValue};
use std::collections::BTreeMap;

struct GpioComponentEndpoint {
    source: SignalId,
    component_pin: SignalId,
    state: SignalId,
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
        validate(scenario)?;
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
            };
            let source_path = format!("{source_prefix}.pin{}", mount.pin);
            let source = hub
                .with_registry(|registry| registry.find(&source_path))
                .ok_or_else(|| SignalError::UnknownPath(source_path.clone()))?;
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
    let idle_level = if button.active_low {
        Logic::One
    } else {
        Logic::Zero
    };
    let pressed_level = if button.active_low {
        Logic::Zero
    } else {
        Logic::One
    };
    let old_level = if pressed { idle_level } else { pressed_level };
    let final_level = if pressed { pressed_level } else { idle_level };
    if button.bounce_ticks == 0 {
        stimuli.push(PinStimulus {
            at,
            pin: button.pin,
            value: final_level,
        });
        return;
    }
    let interval = (button.bounce_ticks / 4).max(1);
    for (index, value) in [final_level, old_level, final_level, old_level, final_level]
        .into_iter()
        .enumerate()
    {
        stimuli.push(PinStimulus {
            at: SimTime::from_ticks(
                at.ticks()
                    .saturating_add(interval.saturating_mul(index as u64)),
            ),
            pin: button.pin,
            value,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BoardComponent, BoardMount, RiscVMachine, TargetId};

    #[test]
    fn routes_button_stimuli_and_live_led_state() {
        let hub = SignalHub::new();
        let led_source = hub
            .declare(
                "board.esp32c6.chip_gpio.pin7",
                SignalValue::repeat(Logic::Z, 1).unwrap(),
                None,
            )
            .unwrap();
        hub.declare(
            "board.esp32c6.chip_gpio.pin9",
            SignalValue::repeat(Logic::Z, 1).unwrap(),
            None,
        )
        .unwrap();
        let scenario = BoardScenario {
            name: "nanoc6".to_owned(),
            target: "esp32c6".to_owned(),
            connectors: Vec::new(),
            mounts: vec![
                BoardMount {
                    component: BoardComponent {
                        name: "button".to_owned(),
                        kind: BoardComponentKind::PushButton {
                            active_low: true,
                            bounce_ticks: 10,
                        },
                    },
                    pin: 9,
                    enable_pin: None,
                },
                BoardMount {
                    component: BoardComponent {
                        name: "blue_led".to_owned(),
                        kind: BoardComponentKind::Led { active_low: true },
                    },
                    pin: 7,
                    enable_pin: None,
                },
            ],
            connections: Vec::new(),
            actions: vec![BoardAction::Press {
                component: "button".to_owned(),
                at: 100,
                duration: 50,
            }],
            duration: 150,
        };
        let endpoint =
            BoardGpioEndpoint::new(&scenario, hub.clone(), "board.esp32c6.chip_gpio").unwrap();
        let stimuli = endpoint.button_stimuli(&scenario.actions).unwrap();
        assert_eq!(stimuli.len(), 10);
        assert_eq!(stimuli[0].pin, 9);
        assert_eq!(stimuli[0].value, Logic::Zero);
        assert_eq!(stimuli[4].at, SimTime::from_ticks(108));
        assert_eq!(stimuli[4].value, Logic::Zero);
        assert_eq!(stimuli[5].at, SimTime::from_ticks(150));
        assert_eq!(stimuli[5].value, Logic::One);
        assert_eq!(stimuli[9].at, SimTime::from_ticks(158));
        assert_eq!(stimuli[9].value, Logic::One);

        hub.set(
            hub.with_registry(|registry| registry.find("board.esp32c6.chip_gpio.pin9").unwrap()),
            SignalValue::repeat(Logic::Zero, 1).unwrap(),
            SimTime::from_ticks(100),
        )
        .unwrap();
        hub.set(
            led_source,
            SignalValue::repeat(Logic::Zero, 1).unwrap(),
            SimTime::from_ticks(20),
        )
        .unwrap();
        endpoint.poll(SimTime::from_ticks(20)).unwrap();
        let led_state = hub.with_registry(|registry| {
            let signal = registry.find("board.nanoc6.component.blue_led.state")?;
            registry.value(signal)?.bit(0)
        });
        assert_eq!(led_state, Some(Logic::One));
        let button_state = hub.with_registry(|registry| {
            let signal = registry.find("board.nanoc6.component.button.state")?;
            registry.value(signal)?.bit(0)
        });
        assert_eq!(button_state, Some(Logic::One));
    }

    #[test]
    fn can_attach_to_riscv_machine_hub() {
        let machine = RiscVMachine::new(TargetId::Esp32c6).unwrap();
        let scenario = BoardScenario {
            name: "nanoc6".to_owned(),
            target: "esp32c6".to_owned(),
            connectors: Vec::new(),
            mounts: vec![BoardMount {
                component: BoardComponent {
                    name: "blue_led".to_owned(),
                    kind: BoardComponentKind::Led { active_low: true },
                },
                pin: 7,
                enable_pin: None,
            }],
            connections: Vec::new(),
            actions: Vec::new(),
            duration: 1,
        };
        let endpoint =
            BoardGpioEndpoint::new(&scenario, machine.signal_hub(), "board.esp32c6.chip_gpio")
                .unwrap();
        endpoint.poll(SimTime::from_ticks(0)).unwrap();
    }

    #[test]
    fn button_waveform_respects_active_high_polarity() {
        let hub = SignalHub::new();
        hub.declare(
            "board.synthetic.chip_gpio.pin0",
            SignalValue::repeat(Logic::Z, 1).unwrap(),
            None,
        )
        .unwrap();
        let scenario = BoardScenario {
            name: "synthetic".to_owned(),
            target: "synthetic".to_owned(),
            connectors: Vec::new(),
            mounts: vec![BoardMount {
                component: BoardComponent {
                    name: "button".to_owned(),
                    kind: BoardComponentKind::PushButton {
                        active_low: false,
                        bounce_ticks: 0,
                    },
                },
                pin: 0,
                enable_pin: None,
            }],
            connections: Vec::new(),
            actions: vec![BoardAction::Press {
                component: "button".to_owned(),
                at: 3,
                duration: 4,
            }],
            duration: 7,
        };
        let endpoint = BoardGpioEndpoint::new(&scenario, hub, "board.synthetic.chip_gpio").unwrap();
        let stimuli = endpoint.button_stimuli(&scenario.actions).unwrap();
        assert_eq!(
            stimuli,
            vec![
                PinStimulus {
                    at: SimTime::from_ticks(3),
                    pin: 0,
                    value: Logic::One,
                },
                PinStimulus {
                    at: SimTime::from_ticks(7),
                    pin: 0,
                    value: Logic::Zero,
                },
            ]
        );
    }
}
