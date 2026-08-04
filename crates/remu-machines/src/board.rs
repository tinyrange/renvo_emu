//! Declarative board topology and deterministic board-component simulation.

use remu_core::SimTime;
use remu_devices::{
    DigitalLed, LedSnapshot, PushButton, Rgb, SGP30_ADDRESS, Sgp30, Sgp30Error, Sgp30Snapshot,
    Ws2812, Ws2812Error,
};
use remu_signals::{Logic, SignalError, SignalId, SignalRegistry, SignalValue};
use remu_trace::{TraceDigest, TraceError, TraceSink};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Protocol inferred for a named physical connector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectorProtocol {
    /// Two-wire open-drain I2C bus.
    I2c,
    /// Clocked SPI bus with data and clock pins recorded by the topology.
    Spi,
    /// Two independent digital pins.
    Digital,
    /// Transmit/receive UART pair.
    Uart,
}

impl std::str::FromStr for ConnectorProtocol {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "i2c" => Ok(Self::I2c),
            "spi" => Ok(Self::Spi),
            "digital" | "gpio" => Ok(Self::Digital),
            "uart" => Ok(Self::Uart),
            _ => Err(format!("unsupported connector protocol {value:?}")),
        }
    }
}

/// Named connector defined by a board's Starlark model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BoardConnector {
    /// Connector name used by `board.connect()`.
    pub name: String,
    /// Default protocol for this board definition.
    pub protocol: ConnectorProtocol,
    /// Data, SDA, RX, or first generic pin.
    pub data_pin: u8,
    /// Clock, SCL, TX, or second generic pin.
    pub clock_pin: u8,
    /// Connector supply in millivolts.
    pub voltage_mv: u16,
}

/// Reusable component configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BoardComponentKind {
    /// Momentary digital input.
    PushButton {
        /// True when a press drives low.
        active_low: bool,
        /// Deterministic bounce window.
        bounce_ticks: u64,
    },
    /// Single-color digital LED.
    Led {
        /// True when a low pin illuminates the LED.
        active_low: bool,
    },
    /// GRB WS2812 chain.
    Ws2812 {
        /// Pixel count.
        count: usize,
    },
    /// Sensirion SGP30 gas sensor.
    Sgp30 {
        /// Initial CO2-equivalent value.
        eco2: u16,
        /// Initial total-VOC value.
        tvoc: u16,
    },
}

impl BoardComponentKind {
    /// Protocol required when connected through a named connector.
    pub const fn connector_protocol(&self) -> Option<ConnectorProtocol> {
        match self {
            Self::Sgp30 { .. } => Some(ConnectorProtocol::I2c),
            Self::PushButton { .. } | Self::Led { .. } | Self::Ws2812 { .. } => {
                Some(ConnectorProtocol::Digital)
            }
        }
    }
}

/// Named component supplied by Rust or a loaded Starlark board definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BoardComponent {
    /// Stable component name.
    pub name: String,
    /// Behavioral model and configuration.
    pub kind: BoardComponentKind,
}

/// Permanently mounted board component and its MCU pins.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BoardMount {
    /// Component configuration.
    pub component: BoardComponent,
    /// Primary MCU GPIO.
    pub pin: u8,
    /// Optional enable/power GPIO.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enable_pin: Option<u8>,
}

/// External component attached to a named connector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BoardConnection {
    /// Connector name.
    pub connector: String,
    /// Attached component.
    pub component: BoardComponent,
}

/// One operation accumulated by the declarative Starlark API.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BoardAction {
    /// Presses and releases a mounted button.
    Press {
        /// Mounted component name.
        component: String,
        /// Press timestamp.
        at: u64,
        /// Held duration.
        duration: u64,
    },
    /// Drives a mounted LED to a logical visible state.
    SetLed {
        /// Mounted component name.
        component: String,
        /// Desired visible state.
        on: bool,
        /// Timestamp.
        at: u64,
    },
    /// Sends one decoded-target frame over a mounted WS2812 data pin.
    Ws2812Frame {
        /// Mounted component name.
        component: String,
        /// RGB values encoded as `0xRRGGBB`.
        colors: Vec<u32>,
        /// Start timestamp.
        at: u64,
    },
    /// Changes environmental values supplied by an SGP30.
    SetAirQuality {
        /// External component name.
        component: String,
        /// CO2-equivalent ppm.
        eco2: u16,
        /// Total VOC ppb.
        tvoc: u16,
        /// Timestamp.
        at: u64,
    },
    /// Executes a complete I2C write/read transfer.
    I2cTransfer {
        /// Connector name.
        connector: String,
        /// Seven-bit address.
        address: u8,
        /// Bytes written before the optional repeated start.
        write: Vec<u8>,
        /// Bytes requested from the target.
        read_len: usize,
        /// Transfer start timestamp.
        at: u64,
    },
}

/// Immutable board scenario emitted by Starlark and consumed by Rust.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BoardScenario {
    /// Board-model name.
    pub name: String,
    /// MCU target selected by the board definition.
    pub target: String,
    /// Physical connectors.
    pub connectors: Vec<BoardConnector>,
    /// Permanently mounted components.
    pub mounts: Vec<BoardMount>,
    /// Test-attached external components.
    pub connections: Vec<BoardConnection>,
    /// Ordered operations.
    pub actions: Vec<BoardAction>,
    /// End of the requested simulation interval.
    pub duration: u64,
}

/// One completed board-level event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BoardEvent {
    /// A button completed a press/release sequence.
    Button {
        /// Component name.
        component: String,
        /// Press time.
        at: u64,
        /// Release time including bounce.
        released_at: u64,
    },
    /// An LED was driven.
    Led {
        /// Component name.
        component: String,
        /// Visible state.
        on: bool,
        /// Timestamp.
        at: u64,
    },
    /// A WS2812 frame was decoded and latched.
    Ws2812 {
        /// Component name.
        component: String,
        /// Decoded colors.
        colors: Vec<Rgb>,
        /// Timestamp at the end of reset-low.
        at: u64,
    },
    /// An SGP30 environment input changed.
    AirQuality {
        /// Component name.
        component: String,
        /// CO2-equivalent ppm.
        eco2: u16,
        /// Total VOC ppb.
        tvoc: u16,
        /// Timestamp.
        at: u64,
    },
    /// A complete I2C transfer finished.
    I2c {
        /// Connector name.
        connector: String,
        /// Address.
        address: u8,
        /// Host write bytes.
        write: Vec<u8>,
        /// Device response bytes.
        read: Vec<u8>,
        /// Transfer start.
        at: u64,
        /// Transfer finish.
        completed_at: u64,
    },
}

/// Final state of one simulated component.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BoardComponentSnapshot {
    /// Push-button state.
    PushButton {
        /// Component name.
        name: String,
        /// Final pressed state.
        pressed: bool,
    },
    /// Single-color LED state.
    Led {
        /// Component name.
        name: String,
        /// Final LED statistics.
        state: LedSnapshot,
    },
    /// WS2812 state.
    Ws2812 {
        /// Component name.
        name: String,
        /// Decoded pixels.
        pixels: Vec<Rgb>,
        /// Latched frame count.
        frames: u64,
    },
    /// SGP30 state.
    Sgp30 {
        /// Component name.
        name: String,
        /// Sensor state.
        state: Sgp30Snapshot,
    },
}

/// Stable board-simulation result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BoardRunResult {
    /// Artifact schema.
    pub schema: &'static str,
    /// Board name.
    pub board: String,
    /// MCU target declared by the board.
    pub target: String,
    /// Simulated duration in nanosecond ticks.
    pub duration: u64,
    /// Resolved topology.
    pub connectors: Vec<BoardConnector>,
    /// On-board components and their MCU pin assignments.
    pub mounts: Vec<BoardMount>,
    /// External components attached to connectors.
    pub connections: Vec<BoardConnection>,
    /// Completed event stream.
    pub events: Vec<BoardEvent>,
    /// Final component states.
    pub components: Vec<BoardComponentSnapshot>,
    /// Canonical digest of board-level signal changes.
    pub trace_digest: String,
    /// Pass marker.
    pub result: &'static str,
}

/// Board scenario validation or execution error.
#[derive(Debug, Error)]
pub enum BoardError {
    /// Board identity is missing.
    #[error("board name and target must be non-empty")]
    Identity,
    /// A name was declared more than once.
    #[error("duplicate board {kind} name {name:?}")]
    Duplicate {
        /// Entity class.
        kind: &'static str,
        /// Duplicate name.
        name: String,
    },
    /// A referenced entity does not exist.
    #[error("unknown board {kind} {name:?}")]
    Unknown {
        /// Entity class.
        kind: &'static str,
        /// Missing name.
        name: String,
    },
    /// An external device cannot use the connector's protocol.
    #[error(
        "component {component:?} requires {required:?}, but connector {connector:?} is {actual:?}"
    )]
    Protocol {
        /// Component name.
        component: String,
        /// Connector name.
        connector: String,
        /// Required protocol.
        required: ConnectorProtocol,
        /// Connector protocol.
        actual: ConnectorProtocol,
    },
    /// An action moved backwards on the simulation timeline.
    #[error("board action time regressed from {previous} to {next}")]
    TimeRegression {
        /// Previous action time.
        previous: u64,
        /// New action time.
        next: u64,
    },
    /// I2C address is absent or belongs to another protocol model.
    #[error("no I2C target at address {address:#04x} on connector {connector:?}")]
    I2cAddress {
        /// Connector name.
        connector: String,
        /// Requested address.
        address: u8,
    },
    /// A model-specific SGP30 command failed.
    #[error(transparent)]
    Sgp30(#[from] Sgp30Error),
    /// A WS2812 waveform was malformed.
    #[error(transparent)]
    Ws2812(#[from] Ws2812Error),
    /// Signal registration or update failed.
    #[error(transparent)]
    Signal(#[from] SignalError),
    /// Trace output failed.
    #[error(transparent)]
    Trace(#[from] TraceError),
}

enum RuntimeComponent {
    Button(PushButton),
    Led(DigitalLed),
    Ws2812(Ws2812),
    Sgp30(Sgp30),
}

struct ComponentSignals {
    pin: Option<SignalId>,
    state: SignalId,
}

/// Validates and executes a board-only deterministic scenario.
pub fn run_board_scenario(
    scenario: &BoardScenario,
    mut trace: Option<&mut dyn TraceSink>,
) -> Result<BoardRunResult, BoardError> {
    validate(scenario)?;
    let mut registry = SignalRegistry::new();
    let mut signals = BTreeMap::new();
    let mut runtime = BTreeMap::new();
    for mount in &scenario.mounts {
        install_component(
            &scenario.name,
            &mount.component,
            true,
            &mut registry,
            &mut signals,
            &mut runtime,
        )?;
    }
    for connection in &scenario.connections {
        install_component(
            &scenario.name,
            &connection.component,
            false,
            &mut registry,
            &mut signals,
            &mut runtime,
        )?;
    }
    let mut connector_signals = BTreeMap::new();
    for connector in &scenario.connectors {
        let prefix = format!("board.{}.connector.{}", scenario.name, connector.name);
        let data = registry.declare(
            format!("{prefix}.data"),
            SignalValue::from_u64(1, 1)?,
            Some("connector data/SDA/RX line".to_owned()),
        )?;
        let clock = registry.declare(
            format!("{prefix}.clock"),
            SignalValue::from_u64(1, 1)?,
            Some("connector clock/SCL/TX line".to_owned()),
        )?;
        connector_signals.insert(connector.name.clone(), (data, clock));
    }
    if let Some(sink) = trace.as_deref_mut() {
        sink.begin(&registry)?;
    }
    let mut digest = TraceDigest::new();
    digest.begin(&registry);
    let mut events = Vec::new();
    let mut previous_at = 0;
    for action in &scenario.actions {
        let at = action_at(action);
        if at < previous_at {
            return Err(BoardError::TimeRegression {
                previous: previous_at,
                next: at,
            });
        }
        previous_at = at;
        match action {
            BoardAction::Press {
                component,
                at,
                duration,
            } => {
                let device = runtime
                    .get_mut(component)
                    .ok_or_else(|| BoardError::Unknown {
                        kind: "component",
                        name: component.clone(),
                    })?;
                let RuntimeComponent::Button(button) = device else {
                    return Err(BoardError::Unknown {
                        kind: "push-button component",
                        name: component.clone(),
                    });
                };
                let ids = signals.get(component).expect("validated component signals");
                for transition in button.set_pressed(true, SimTime::from_ticks(*at)) {
                    emit_logic(
                        &mut registry,
                        ids.pin.expect("mounted button has pin signal"),
                        transition.level,
                        transition.at,
                        &mut trace,
                        &mut digest,
                    )?;
                }
                emit_u64(
                    &mut registry,
                    ids.state,
                    1,
                    1,
                    SimTime::from_ticks(at.saturating_add(button.bounce_ticks())),
                    &mut trace,
                    &mut digest,
                )?;
                let release_at = at.saturating_add(*duration);
                let mut released_at = release_at;
                for transition in button.set_pressed(false, SimTime::from_ticks(release_at)) {
                    released_at = transition.at.ticks();
                    emit_logic(
                        &mut registry,
                        ids.pin.expect("mounted button has pin signal"),
                        transition.level,
                        transition.at,
                        &mut trace,
                        &mut digest,
                    )?;
                }
                emit_u64(
                    &mut registry,
                    ids.state,
                    0,
                    1,
                    SimTime::from_ticks(released_at),
                    &mut trace,
                    &mut digest,
                )?;
                events.push(BoardEvent::Button {
                    component: component.clone(),
                    at: *at,
                    released_at,
                });
            }
            BoardAction::SetLed { component, on, at } => {
                let device = runtime
                    .get_mut(component)
                    .ok_or_else(|| BoardError::Unknown {
                        kind: "component",
                        name: component.clone(),
                    })?;
                let RuntimeComponent::Led(led) = device else {
                    return Err(BoardError::Unknown {
                        kind: "LED component",
                        name: component.clone(),
                    });
                };
                let ids = signals.get(component).expect("validated component signals");
                let electrical = if *on == led_level_is_high(&mount_kind(scenario, component)?) {
                    Logic::One
                } else {
                    Logic::Zero
                };
                led.observe(electrical, SimTime::from_ticks(*at));
                emit_logic(
                    &mut registry,
                    ids.pin.expect("mounted LED has pin signal"),
                    electrical,
                    SimTime::from_ticks(*at),
                    &mut trace,
                    &mut digest,
                )?;
                emit_u64(
                    &mut registry,
                    ids.state,
                    u64::from(*on),
                    1,
                    SimTime::from_ticks(*at),
                    &mut trace,
                    &mut digest,
                )?;
                events.push(BoardEvent::Led {
                    component: component.clone(),
                    on: *on,
                    at: *at,
                });
            }
            BoardAction::Ws2812Frame {
                component,
                colors,
                at,
            } => {
                let device = runtime
                    .get_mut(component)
                    .ok_or_else(|| BoardError::Unknown {
                        kind: "component",
                        name: component.clone(),
                    })?;
                let RuntimeComponent::Ws2812(ws) = device else {
                    return Err(BoardError::Unknown {
                        kind: "WS2812 component",
                        name: component.clone(),
                    });
                };
                let ids = signals.get(component).expect("validated component signals");
                let data = ids.pin.expect("mounted WS2812 has pin signal");
                let completed_at = emit_ws2812(
                    ws,
                    colors,
                    SimTime::from_ticks(*at),
                    &mut registry,
                    data,
                    &mut trace,
                    &mut digest,
                )?;
                let first = ws.pixels().first().copied().unwrap_or(Rgb {
                    red: 0,
                    green: 0,
                    blue: 0,
                });
                emit_u64(
                    &mut registry,
                    ids.state,
                    (u64::from(first.red) << 16)
                        | (u64::from(first.green) << 8)
                        | u64::from(first.blue),
                    24,
                    completed_at,
                    &mut trace,
                    &mut digest,
                )?;
                events.push(BoardEvent::Ws2812 {
                    component: component.clone(),
                    colors: ws.pixels().to_vec(),
                    at: completed_at.ticks(),
                });
            }
            BoardAction::SetAirQuality {
                component,
                eco2,
                tvoc,
                at,
            } => {
                let device = runtime
                    .get_mut(component)
                    .ok_or_else(|| BoardError::Unknown {
                        kind: "component",
                        name: component.clone(),
                    })?;
                let RuntimeComponent::Sgp30(sensor) = device else {
                    return Err(BoardError::Unknown {
                        kind: "SGP30 component",
                        name: component.clone(),
                    });
                };
                sensor.set_air_quality(*eco2, *tvoc);
                let ids = signals.get(component).expect("validated component signals");
                emit_u64(
                    &mut registry,
                    ids.state,
                    (u64::from(*eco2) << 16) | u64::from(*tvoc),
                    32,
                    SimTime::from_ticks(*at),
                    &mut trace,
                    &mut digest,
                )?;
                events.push(BoardEvent::AirQuality {
                    component: component.clone(),
                    eco2: *eco2,
                    tvoc: *tvoc,
                    at: *at,
                });
            }
            BoardAction::I2cTransfer {
                connector,
                address,
                write,
                read_len,
                at,
            } => {
                let connection = scenario
                    .connections
                    .iter()
                    .find(|connection| {
                        connection.connector == *connector
                            && matches!(connection.component.kind, BoardComponentKind::Sgp30 { .. })
                            && *address == SGP30_ADDRESS
                    })
                    .ok_or_else(|| BoardError::I2cAddress {
                        connector: connector.clone(),
                        address: *address,
                    })?;
                let RuntimeComponent::Sgp30(sensor) = runtime
                    .get_mut(&connection.component.name)
                    .expect("validated connection has runtime component")
                else {
                    unreachable!("SGP30 connection constructed an SGP30 runtime")
                };
                let response = sensor.transact(write, *read_len, SimTime::from_ticks(*at))?;
                let (data, clock) = connector_signals
                    .get(connector)
                    .copied()
                    .expect("validated connector has signals");
                let completed_at = emit_i2c(
                    *address,
                    write,
                    &response,
                    SimTime::from_ticks(*at),
                    &mut registry,
                    data,
                    clock,
                    &mut trace,
                    &mut digest,
                )?;
                events.push(BoardEvent::I2c {
                    connector: connector.clone(),
                    address: *address,
                    write: write.clone(),
                    read: response,
                    at: *at,
                    completed_at: completed_at.ticks(),
                });
            }
        }
    }
    let end = SimTime::from_ticks(scenario.duration);
    let mut components = Vec::new();
    for (name, device) in &mut runtime {
        components.push(match device {
            RuntimeComponent::Button(button) => BoardComponentSnapshot::PushButton {
                name: name.clone(),
                pressed: button.pressed(),
            },
            RuntimeComponent::Led(led) => BoardComponentSnapshot::Led {
                name: name.clone(),
                state: led.snapshot(end),
            },
            RuntimeComponent::Ws2812(ws) => BoardComponentSnapshot::Ws2812 {
                name: name.clone(),
                pixels: ws.pixels().to_vec(),
                frames: ws.frames(),
            },
            RuntimeComponent::Sgp30(sensor) => BoardComponentSnapshot::Sgp30 {
                name: name.clone(),
                state: sensor.snapshot(),
            },
        });
    }
    if let Some(sink) = trace {
        sink.finish()?;
    }
    Ok(BoardRunResult {
        schema: "remu.board-simulation.v1",
        board: scenario.name.clone(),
        target: scenario.target.clone(),
        duration: scenario.duration,
        connectors: scenario.connectors.clone(),
        mounts: scenario.mounts.clone(),
        connections: scenario.connections.clone(),
        events,
        components,
        trace_digest: digest.finish(),
        result: "pass",
    })
}

fn validate(scenario: &BoardScenario) -> Result<(), BoardError> {
    if scenario.name.is_empty() || scenario.target.is_empty() {
        return Err(BoardError::Identity);
    }
    let mut connectors = BTreeMap::new();
    for connector in &scenario.connectors {
        if connectors.insert(&connector.name, connector).is_some() {
            return Err(BoardError::Duplicate {
                kind: "connector",
                name: connector.name.clone(),
            });
        }
    }
    let mut components = BTreeSet::new();
    for component in scenario.mounts.iter().map(|mount| &mount.component).chain(
        scenario
            .connections
            .iter()
            .map(|connection| &connection.component),
    ) {
        if !components.insert(component.name.clone()) {
            return Err(BoardError::Duplicate {
                kind: "component",
                name: component.name.clone(),
            });
        }
    }
    for connection in &scenario.connections {
        let connector =
            connectors
                .get(&connection.connector)
                .ok_or_else(|| BoardError::Unknown {
                    kind: "connector",
                    name: connection.connector.clone(),
                })?;
        if let Some(required) = connection.component.kind.connector_protocol()
            && required != connector.protocol
        {
            return Err(BoardError::Protocol {
                component: connection.component.name.clone(),
                connector: connector.name.clone(),
                required,
                actual: connector.protocol,
            });
        }
    }
    Ok(())
}

fn install_component(
    board: &str,
    component: &BoardComponent,
    mounted: bool,
    registry: &mut SignalRegistry,
    signals: &mut BTreeMap<String, ComponentSignals>,
    runtime: &mut BTreeMap<String, RuntimeComponent>,
) -> Result<(), BoardError> {
    let prefix = format!("board.{board}.component.{}", component.name);
    let (device, width, initial_pin, initial_state, description) = match component.kind {
        BoardComponentKind::PushButton {
            active_low,
            bounce_ticks,
        } => (
            RuntimeComponent::Button(PushButton::new(active_low, bounce_ticks)),
            1,
            u64::from(active_low),
            0,
            "logical button state",
        ),
        BoardComponentKind::Led { active_low } => (
            RuntimeComponent::Led(DigitalLed::new(active_low)),
            1,
            u64::from(active_low),
            0,
            "visible LED state",
        ),
        BoardComponentKind::Ws2812 { count } => (
            RuntimeComponent::Ws2812(Ws2812::new(count)),
            24,
            0,
            0,
            "first decoded WS2812 RGB pixel",
        ),
        BoardComponentKind::Sgp30 { eco2, tvoc } => (
            RuntimeComponent::Sgp30(Sgp30::new(eco2, tvoc)),
            32,
            0,
            (u64::from(eco2) << 16) | u64::from(tvoc),
            "SGP30 eCO2 and TVOC environmental input",
        ),
    };
    let pin = if mounted {
        Some(registry.declare(
            format!("{prefix}.pin"),
            SignalValue::from_u64(initial_pin, 1)?,
            Some("physical component data pin".to_owned()),
        )?)
    } else {
        None
    };
    let state = registry.declare(
        format!("{prefix}.state"),
        SignalValue::from_u64(initial_state, width)?,
        Some(description.to_owned()),
    )?;
    runtime.insert(component.name.clone(), device);
    signals.insert(component.name.clone(), ComponentSignals { pin, state });
    Ok(())
}

fn action_at(action: &BoardAction) -> u64 {
    match action {
        BoardAction::Press { at, .. }
        | BoardAction::SetLed { at, .. }
        | BoardAction::Ws2812Frame { at, .. }
        | BoardAction::SetAirQuality { at, .. }
        | BoardAction::I2cTransfer { at, .. } => *at,
    }
}

fn mount_kind(scenario: &BoardScenario, component: &str) -> Result<BoardComponentKind, BoardError> {
    scenario
        .mounts
        .iter()
        .find(|mount| mount.component.name == component)
        .map(|mount| mount.component.kind.clone())
        .ok_or_else(|| BoardError::Unknown {
            kind: "mounted component",
            name: component.to_owned(),
        })
}

const fn led_level_is_high(kind: &BoardComponentKind) -> bool {
    match kind {
        BoardComponentKind::Led { active_low } => !*active_low,
        _ => false,
    }
}

fn emit_logic(
    registry: &mut SignalRegistry,
    signal: SignalId,
    value: Logic,
    at: SimTime,
    trace: &mut Option<&mut dyn TraceSink>,
    digest: &mut TraceDigest,
) -> Result<(), BoardError> {
    let value = SignalValue::new(vec![value])?;
    emit_value(registry, signal, value, at, trace, digest)
}

fn emit_u64(
    registry: &mut SignalRegistry,
    signal: SignalId,
    value: u64,
    width: u16,
    at: SimTime,
    trace: &mut Option<&mut dyn TraceSink>,
    digest: &mut TraceDigest,
) -> Result<(), BoardError> {
    emit_value(
        registry,
        signal,
        SignalValue::from_u64(value, width)?,
        at,
        trace,
        digest,
    )
}

fn emit_value(
    registry: &mut SignalRegistry,
    signal: SignalId,
    value: SignalValue,
    at: SimTime,
    trace: &mut Option<&mut dyn TraceSink>,
    digest: &mut TraceDigest,
) -> Result<(), BoardError> {
    if let Some(change) = registry.set(signal, value, at)? {
        digest.change(&change);
        if let Some(sink) = trace.as_deref_mut() {
            sink.change(&change)?;
        }
    }
    Ok(())
}

fn emit_ws2812(
    ws: &mut Ws2812,
    colors: &[u32],
    start: SimTime,
    registry: &mut SignalRegistry,
    data: SignalId,
    trace: &mut Option<&mut dyn TraceSink>,
    digest: &mut TraceDigest,
) -> Result<SimTime, BoardError> {
    let mut now = start.ticks();
    for color in colors {
        let red = ((color >> 16) & 0xff) as u8;
        let green = ((color >> 8) & 0xff) as u8;
        let blue = (color & 0xff) as u8;
        for byte in [green, red, blue] {
            for bit in (0..8).rev() {
                let high = if byte & (1 << bit) == 0 { 350 } else { 700 };
                let low = 1_250 - high;
                let rise = SimTime::from_ticks(now);
                ws.observe(Logic::One, rise)?;
                emit_logic(registry, data, Logic::One, rise, trace, digest)?;
                now = now.saturating_add(high);
                let fall = SimTime::from_ticks(now);
                ws.observe(Logic::Zero, fall)?;
                emit_logic(registry, data, Logic::Zero, fall, trace, digest)?;
                now = now.saturating_add(low);
            }
        }
    }
    now = now.saturating_add(Ws2812::RESET_TICKS);
    let completed = SimTime::from_ticks(now);
    ws.finish(completed)?;
    Ok(completed)
}

#[allow(clippy::too_many_arguments)]
fn emit_i2c(
    address: u8,
    write: &[u8],
    read: &[u8],
    start: SimTime,
    registry: &mut SignalRegistry,
    data: SignalId,
    clock: SignalId,
    trace: &mut Option<&mut dyn TraceSink>,
    digest: &mut TraceDigest,
) -> Result<SimTime, BoardError> {
    const HALF: u64 = 5_000;
    let mut now = start.ticks();
    emit_logic(
        registry,
        data,
        Logic::Zero,
        SimTime::from_ticks(now),
        trace,
        digest,
    )?;
    now = now.saturating_add(HALF);
    let mut bytes = vec![(address << 1, false)];
    bytes.extend(write.iter().copied().map(|byte| (byte, false)));
    if !read.is_empty() {
        bytes.push(((address << 1) | 1, false));
        bytes.extend(read.iter().copied().map(|byte| (byte, true)));
    }
    for (byte, _from_device) in bytes {
        for bit in (0..8).rev() {
            emit_logic(
                registry,
                clock,
                Logic::Zero,
                SimTime::from_ticks(now),
                trace,
                digest,
            )?;
            emit_logic(
                registry,
                data,
                if byte & (1 << bit) == 0 {
                    Logic::Zero
                } else {
                    Logic::One
                },
                SimTime::from_ticks(now),
                trace,
                digest,
            )?;
            now = now.saturating_add(HALF);
            emit_logic(
                registry,
                clock,
                Logic::One,
                SimTime::from_ticks(now),
                trace,
                digest,
            )?;
            now = now.saturating_add(HALF);
        }
        emit_logic(
            registry,
            clock,
            Logic::Zero,
            SimTime::from_ticks(now),
            trace,
            digest,
        )?;
        emit_logic(
            registry,
            data,
            Logic::Zero,
            SimTime::from_ticks(now),
            trace,
            digest,
        )?;
        now = now.saturating_add(HALF);
        emit_logic(
            registry,
            clock,
            Logic::One,
            SimTime::from_ticks(now),
            trace,
            digest,
        )?;
        now = now.saturating_add(HALF);
    }
    emit_logic(
        registry,
        clock,
        Logic::One,
        SimTime::from_ticks(now),
        trace,
        digest,
    )?;
    emit_logic(
        registry,
        data,
        Logic::Zero,
        SimTime::from_ticks(now),
        trace,
        digest,
    )?;
    now = now.saturating_add(HALF);
    emit_logic(
        registry,
        data,
        Logic::One,
        SimTime::from_ticks(now),
        trace,
        digest,
    )?;
    Ok(SimTime::from_ticks(now))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario() -> BoardScenario {
        BoardScenario {
            name: "nanoc6".to_owned(),
            target: "esp32c6".to_owned(),
            connectors: vec![BoardConnector {
                name: "grove".to_owned(),
                protocol: ConnectorProtocol::I2c,
                data_pin: 2,
                clock_pin: 1,
                voltage_mv: 5_000,
            }],
            mounts: vec![BoardMount {
                component: BoardComponent {
                    name: "rgb".to_owned(),
                    kind: BoardComponentKind::Ws2812 { count: 1 },
                },
                pin: 20,
                enable_pin: Some(19),
            }],
            connections: vec![BoardConnection {
                connector: "grove".to_owned(),
                component: BoardComponent {
                    name: "air".to_owned(),
                    kind: BoardComponentKind::Sgp30 { eco2: 420, tvoc: 8 },
                },
            }],
            actions: vec![
                BoardAction::I2cTransfer {
                    connector: "grove".to_owned(),
                    address: SGP30_ADDRESS,
                    write: vec![0x20, 0x03],
                    read_len: 0,
                    at: 0,
                },
                BoardAction::SetAirQuality {
                    component: "air".to_owned(),
                    eco2: 900,
                    tvoc: 77,
                    at: Sgp30::WARMUP_TICKS,
                },
                BoardAction::I2cTransfer {
                    connector: "grove".to_owned(),
                    address: SGP30_ADDRESS,
                    write: vec![0x20, 0x08],
                    read_len: 6,
                    at: Sgp30::WARMUP_TICKS + 1_000_000,
                },
                BoardAction::Ws2812Frame {
                    component: "rgb".to_owned(),
                    colors: vec![0xff_00_00],
                    at: Sgp30::WARMUP_TICKS + 2_000_000,
                },
            ],
            duration: Sgp30::WARMUP_TICKS + 3_000_000,
        }
    }

    #[test]
    fn executes_connected_sensor_and_ws2812() {
        let result = run_board_scenario(&scenario(), None).unwrap();
        assert_eq!(result.result, "pass");
        assert!(result.events.iter().any(|event| matches!(
            event,
            BoardEvent::I2c { read, .. } if read.len() == 6
        )));
        assert!(result.components.iter().any(|component| matches!(
            component,
            BoardComponentSnapshot::Ws2812 { pixels, .. }
                if pixels.first().is_some_and(|pixel| pixel.red == 255)
        )));
    }
}
