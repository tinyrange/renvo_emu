use super::{
    BoardComponentKind, BoardError, BoardScenario, ConnectorProtocol, SGP30_ADDRESS, Sgp30,
    Sgp30Snapshot,
};
use remu_core::SimTime;
use remu_devices::SignalHub;
use remu_signals::{Logic, SignalId, SignalValue};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
struct ConnectorPins {
    data: SignalId,
    clock: SignalId,
    connector_data: SignalId,
    connector_clock: SignalId,
}

/// Result of one deterministic board-level I2C transfer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BoardI2cTransfer {
    /// Connector used for the transfer.
    pub connector: String,
    /// Seven-bit target address.
    pub address: u8,
    /// Bytes written by the host before the optional repeated start.
    pub write: Vec<u8>,
    /// Bytes returned by the target.
    pub read: Vec<u8>,
    /// Transfer start time.
    pub at: SimTime,
    /// Time at which the stop condition was emitted.
    pub completed_at: SimTime,
}

/// Typed I2C bridge for board connectors and command-level external devices.
///
/// The endpoint resolves connector SDA/SCL pins against a machine's shared
/// signal hub, exposes connector-level signals for VCD tracing, and routes
/// supported external devices such as the SGP30. It is deliberately a host
/// transfer endpoint: firmware I2C-controller MMIO observation and device ACK
/// injection remain a separate machine-peripheral slice.
pub struct BoardI2cEndpoint {
    hub: SignalHub,
    connectors: BTreeMap<String, ConnectorPins>,
    protocols: BTreeMap<String, ConnectorProtocol>,
    targets: BTreeMap<(String, u8), String>,
    devices: BTreeMap<String, Sgp30>,
}

impl BoardI2cEndpoint {
    /// Attaches I2C connectors and supported external devices to a machine hub.
    pub fn new(
        scenario: &BoardScenario,
        hub: SignalHub,
        source_prefix: &str,
    ) -> Result<Self, BoardError> {
        super::validate(scenario)?;
        let mut connectors = BTreeMap::new();
        let mut protocols = BTreeMap::new();
        for connector in &scenario.connectors {
            protocols.insert(connector.name.clone(), connector.protocol);
            if connector.protocol != ConnectorProtocol::I2c {
                continue;
            }
            if connector.data_pin == connector.clock_pin {
                return Err(BoardError::I2cPinAlias {
                    connector: connector.name.clone(),
                    pin: connector.data_pin,
                });
            }
            let data_path = format!("{source_prefix}.pin{}", connector.data_pin);
            let clock_path = format!("{source_prefix}.pin{}", connector.clock_pin);
            let data = find_signal(&hub, &data_path)?;
            let clock = find_signal(&hub, &clock_path)?;
            let connector_prefix = format!("board.{}.connector.{}", scenario.name, connector.name);
            let connector_data = hub.declare(
                format!("{connector_prefix}.data"),
                SignalValue::repeat(signal_level(&hub, data), 1)?,
                Some("connector SDA/data line".to_owned()),
            )?;
            let connector_clock = hub.declare(
                format!("{connector_prefix}.clock"),
                SignalValue::repeat(signal_level(&hub, clock), 1)?,
                Some("connector SCL/clock line".to_owned()),
            )?;
            connectors.insert(
                connector.name.clone(),
                ConnectorPins {
                    data,
                    clock,
                    connector_data,
                    connector_clock,
                },
            );
        }

        let mut targets = BTreeMap::new();
        let mut devices = BTreeMap::new();
        for connection in &scenario.connections {
            let connector = protocols
                .get(&connection.connector)
                .copied()
                .expect("validated connection has a connector");
            let (eco2, tvoc) = match &connection.component.kind {
                BoardComponentKind::Sgp30 { eco2, tvoc } => (*eco2, *tvoc),
                BoardComponentKind::PushButton { .. } => {
                    return Err(BoardError::I2cComponent {
                        component: connection.component.name.clone(),
                        kind: "push-button",
                    });
                }
                BoardComponentKind::Led { .. } => {
                    return Err(BoardError::I2cComponent {
                        component: connection.component.name.clone(),
                        kind: "LED",
                    });
                }
                BoardComponentKind::Ws2812 { .. } => {
                    return Err(BoardError::I2cComponent {
                        component: connection.component.name.clone(),
                        kind: "WS2812",
                    });
                }
            };
            if connector != ConnectorProtocol::I2c {
                return Err(BoardError::Protocol {
                    component: connection.component.name.clone(),
                    connector: connection.connector.clone(),
                    required: ConnectorProtocol::I2c,
                    actual: connector,
                });
            }
            let key = (connection.connector.clone(), SGP30_ADDRESS);
            if targets
                .insert(key, connection.component.name.clone())
                .is_some()
            {
                return Err(BoardError::Duplicate {
                    kind: "I2C target",
                    name: connection.component.name.clone(),
                });
            }
            devices.insert(connection.component.name.clone(), Sgp30::new(eco2, tvoc));
        }
        Ok(Self {
            hub,
            connectors,
            protocols,
            targets,
            devices,
        })
    }

    /// Performs one host-side transfer and emits deterministic SDA/SCL edges.
    pub fn transfer(
        &mut self,
        connector: &str,
        address: u8,
        write: &[u8],
        read_len: usize,
        at: SimTime,
    ) -> Result<BoardI2cTransfer, BoardError> {
        let protocol =
            self.protocols
                .get(connector)
                .copied()
                .ok_or_else(|| BoardError::Unknown {
                    kind: "connector",
                    name: connector.to_owned(),
                })?;
        if protocol != ConnectorProtocol::I2c {
            return Err(BoardError::Protocol {
                component: "I2C transfer".to_owned(),
                connector: connector.to_owned(),
                required: ConnectorProtocol::I2c,
                actual: protocol,
            });
        }
        if address > 0x7f {
            return Err(BoardError::I2cAddressWidth { address });
        }
        let pins = *self
            .connectors
            .get(connector)
            .expect("I2C protocol has resolved connector pins");
        let target = self
            .targets
            .get(&(connector.to_owned(), address))
            .cloned()
            .ok_or_else(|| BoardError::I2cAddress {
                connector: connector.to_owned(),
                address,
            })?;
        let response = self
            .devices
            .get_mut(&target)
            .expect("I2C target has a device model")
            .transact(write, read_len, at)?;
        let completed_at = emit_i2c(&self.hub, pins, address, write, &response, at)?;
        Ok(BoardI2cTransfer {
            connector: connector.to_owned(),
            address,
            write: write.to_vec(),
            read: response,
            at,
            completed_at,
        })
    }

    /// Returns the current snapshot for a connected SGP30 target.
    pub fn sgp30_snapshot(&self, connector: &str) -> Option<Sgp30Snapshot> {
        let name = self.targets.get(&(connector.to_owned(), SGP30_ADDRESS))?;
        self.devices.get(name).map(Sgp30::snapshot)
    }
}

fn find_signal(hub: &SignalHub, path: &str) -> Result<SignalId, BoardError> {
    hub.with_registry(|registry| registry.find(path))
        .ok_or_else(|| remu_signals::SignalError::UnknownPath(path.to_owned()).into())
}

fn signal_level(hub: &SignalHub, signal: SignalId) -> Logic {
    hub.with_registry(|registry| {
        registry
            .value(signal)
            .and_then(|value| value.bit(0))
            .unwrap_or(Logic::X)
    })
}

fn set_line(
    hub: &SignalHub,
    source: SignalId,
    connector: SignalId,
    value: Logic,
    at: SimTime,
) -> Result<(), BoardError> {
    let signal = SignalValue::repeat(value, 1)?;
    hub.set(source, signal.clone(), at)?;
    hub.set(connector, signal, at)?;
    Ok(())
}

fn emit_i2c(
    hub: &SignalHub,
    pins: ConnectorPins,
    address: u8,
    write: &[u8],
    read: &[u8],
    start: SimTime,
) -> Result<SimTime, BoardError> {
    const HALF: u64 = 5_000;
    let mut now = start.ticks();
    let start_at = SimTime::from_ticks(now);
    // An I2C transaction begins from the released (high/high) bus state.
    set_line(hub, pins.clock, pins.connector_clock, Logic::One, start_at)?;
    set_line(hub, pins.data, pins.connector_data, Logic::One, start_at)?;
    set_line(hub, pins.data, pins.connector_data, Logic::Zero, start_at)?;
    now = now.saturating_add(HALF);
    set_line(
        hub,
        pins.clock,
        pins.connector_clock,
        Logic::Zero,
        SimTime::from_ticks(now),
    )?;

    now = emit_byte(hub, pins, address << 1, true, now)?;
    for byte in write {
        now = emit_byte(hub, pins, *byte, true, now)?;
    }
    if !read.is_empty() {
        // Release SDA while SCL is high, then pull it low for a repeated start.
        set_line(
            hub,
            pins.data,
            pins.connector_data,
            Logic::One,
            SimTime::from_ticks(now),
        )?;
        now = now.saturating_add(HALF);
        set_line(
            hub,
            pins.data,
            pins.connector_data,
            Logic::Zero,
            SimTime::from_ticks(now),
        )?;
        now = now.saturating_add(HALF);
        set_line(
            hub,
            pins.clock,
            pins.connector_clock,
            Logic::Zero,
            SimTime::from_ticks(now),
        )?;
        now = emit_byte(hub, pins, (address << 1) | 1, true, now)?;
        for (index, byte) in read.iter().copied().enumerate() {
            let ack = index + 1 < read.len();
            now = emit_byte(hub, pins, byte, ack, now)?;
        }
    }

    // A stop condition is SDA rising while SCL is high.
    set_line(
        hub,
        pins.clock,
        pins.connector_clock,
        Logic::Zero,
        SimTime::from_ticks(now),
    )?;
    set_line(
        hub,
        pins.data,
        pins.connector_data,
        Logic::Zero,
        SimTime::from_ticks(now),
    )?;
    now = now.saturating_add(HALF);
    set_line(
        hub,
        pins.clock,
        pins.connector_clock,
        Logic::One,
        SimTime::from_ticks(now),
    )?;
    now = now.saturating_add(HALF);
    set_line(
        hub,
        pins.data,
        pins.connector_data,
        Logic::One,
        SimTime::from_ticks(now),
    )?;
    Ok(SimTime::from_ticks(now))
}

fn emit_byte(
    hub: &SignalHub,
    pins: ConnectorPins,
    byte: u8,
    ack: bool,
    mut now: u64,
) -> Result<u64, BoardError> {
    const HALF: u64 = 5_000;
    for bit in (0..8).rev() {
        let at = SimTime::from_ticks(now);
        set_line(hub, pins.clock, pins.connector_clock, Logic::Zero, at)?;
        set_line(
            hub,
            pins.data,
            pins.connector_data,
            if byte & (1 << bit) == 0 {
                Logic::Zero
            } else {
                Logic::One
            },
            at,
        )?;
        now = now.saturating_add(HALF);
        set_line(
            hub,
            pins.clock,
            pins.connector_clock,
            Logic::One,
            SimTime::from_ticks(now),
        )?;
        now = now.saturating_add(HALF);
    }
    let at = SimTime::from_ticks(now);
    set_line(hub, pins.clock, pins.connector_clock, Logic::Zero, at)?;
    set_line(
        hub,
        pins.data,
        pins.connector_data,
        if ack { Logic::Zero } else { Logic::One },
        at,
    )?;
    now = now.saturating_add(HALF);
    set_line(
        hub,
        pins.clock,
        pins.connector_clock,
        Logic::One,
        SimTime::from_ticks(now),
    )?;
    Ok(now.saturating_add(HALF))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(data_pin: u8, clock_pin: u8) -> (SignalHub, BoardScenario) {
        let hub = SignalHub::new();
        hub.declare(
            format!("board.esp32c6.chip_gpio.pin{data_pin}"),
            SignalValue::repeat(Logic::Z, 1).unwrap(),
            None,
        )
        .unwrap();
        if data_pin != clock_pin {
            hub.declare(
                format!("board.esp32c6.chip_gpio.pin{clock_pin}"),
                SignalValue::repeat(Logic::Z, 1).unwrap(),
                None,
            )
            .unwrap();
        }
        let scenario = BoardScenario {
            name: "nanoc6".to_owned(),
            target: "esp32c6".to_owned(),
            connectors: vec![super::super::BoardConnector {
                name: "grove".to_owned(),
                protocol: ConnectorProtocol::I2c,
                data_pin,
                clock_pin,
                voltage_mv: 5_000,
            }],
            mounts: Vec::new(),
            connections: vec![super::super::BoardConnection {
                connector: "grove".to_owned(),
                component: super::super::BoardComponent {
                    name: "air".to_owned(),
                    kind: BoardComponentKind::Sgp30 { eco2: 420, tvoc: 8 },
                },
            }],
            actions: Vec::new(),
            duration: 1,
        };
        (hub, scenario)
    }

    #[test]
    fn routes_sgp30_transfer_through_machine_connector_signals() {
        let (hub, scenario) = fixture(2, 1);
        let mut endpoint =
            BoardI2cEndpoint::new(&scenario, hub.clone(), "board.esp32c6.chip_gpio").unwrap();
        let transfer = endpoint
            .transfer("grove", SGP30_ADDRESS, &[0x20, 0x03], 0, SimTime::ZERO)
            .unwrap();
        assert!(transfer.completed_at > SimTime::ZERO);
        assert!(endpoint.sgp30_snapshot("grove").unwrap().initialized);
        let measurement = endpoint
            .transfer(
                "grove",
                SGP30_ADDRESS,
                &[0x20, 0x08],
                6,
                SimTime::from_ticks(Sgp30::WARMUP_TICKS),
            )
            .unwrap();
        assert_eq!(measurement.read.len(), 6);
        assert!(
            hub.with_registry(|registry| registry.find("board.nanoc6.connector.grove.data"))
                .is_some()
        );
        let data = hub
            .with_registry(|registry| registry.find("board.nanoc6.connector.grove.data"))
            .unwrap();
        let clock = hub
            .with_registry(|registry| registry.find("board.nanoc6.connector.grove.clock"))
            .unwrap();
        let mut clock_level = Logic::Z;
        let mut starts = 0;
        for change in hub.drain_changes() {
            if change.signal == clock {
                clock_level = change.value.bit(0).unwrap();
            } else if change.signal == data
                && change.value.bit(0) == Some(Logic::Zero)
                && clock_level == Logic::One
            {
                starts += 1;
            }
        }
        // The initialization transfer has one start; the read transfer has a
        // start plus a repeated start between its write and read phases.
        assert_eq!(starts, 3);
        assert_eq!(
            hub.with_registry(|registry| registry.value(data).unwrap().bit(0)),
            Some(Logic::One)
        );
        assert_eq!(
            hub.with_registry(|registry| registry.value(clock).unwrap().bit(0)),
            Some(Logic::One)
        );
    }

    #[test]
    fn rejects_non_seven_bit_address_before_target_lookup() {
        let (hub, scenario) = fixture(2, 1);
        let mut endpoint =
            BoardI2cEndpoint::new(&scenario, hub, "board.esp32c6.chip_gpio").unwrap();
        let error = endpoint
            .transfer("grove", 0x80, &[], 0, SimTime::ZERO)
            .unwrap_err();
        assert!(matches!(
            error,
            BoardError::I2cAddressWidth { address: 0x80 }
        ));
    }

    #[test]
    fn rejects_sda_scl_pin_aliases() {
        let (hub, mut scenario) = fixture(2, 1);
        scenario.connectors[0].clock_pin = 2;
        let error = BoardI2cEndpoint::new(&scenario, hub, "board.esp32c6.chip_gpio")
            .err()
            .unwrap();
        assert!(matches!(
            error,
            BoardError::I2cPinAlias {
                connector,
                pin: 2
            } if connector == "grove"
        ));
    }
}
