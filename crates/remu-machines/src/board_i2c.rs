use super::{
    BMI270_ADDRESS, Bmi270, Bmi270Snapshot, BoardComponentKind, BoardError, BoardScenario,
    ConnectorProtocol, ES8311_ADDRESS, Es8311, Es8311Snapshot, M5PM1_ADDRESS, M5Pm1, M5Pm1Snapshot,
    SGP30_ADDRESS, Sgp30, Sgp30Snapshot,
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

#[derive(Clone)]
enum BoardI2cDevice {
    Sgp30(Sgp30),
    M5Pm1(M5Pm1),
    Bmi270(Bmi270),
    Es8311(Es8311),
}

impl BoardI2cDevice {
    fn transact(
        &mut self,
        write: &[u8],
        read_len: usize,
        at: SimTime,
    ) -> Result<Vec<u8>, BoardError> {
        match self {
            Self::Sgp30(device) => Ok(device.transact(write, read_len, at)?),
            Self::M5Pm1(device) => Ok(device.transact(write, read_len, at)?),
            Self::Bmi270(device) => Ok(device.transact(write, read_len, at)?),
            Self::Es8311(device) => Ok(device.transact(write, read_len, at)?),
        }
    }
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
    devices: BTreeMap<String, BoardI2cDevice>,
    available_at: SimTime,
}

const I2C_HALF_TICKS: u64 = 5_000;

/// Returns the exclusive end of a byte-level I2C waveform before any signal is
/// changed.  A byte consumes eight data/clock pairs and one ACK pair; the
/// start and stop framing each consume one half-period.
pub(crate) fn i2c_waveform_end(start: SimTime, byte_count: usize) -> Result<SimTime, BoardError> {
    let bytes = u64::try_from(byte_count).map_err(|_| BoardError::I2cTimeOverflow)?;
    let half_periods = bytes
        .checked_mul(18)
        .and_then(|periods| periods.checked_add(2))
        .ok_or(BoardError::I2cTimeOverflow)?;
    let duration = half_periods
        .checked_mul(I2C_HALF_TICKS)
        .ok_or(BoardError::I2cTimeOverflow)?;
    start
        .ticks()
        .checked_add(duration)
        .map(SimTime::from_ticks)
        .ok_or(BoardError::I2cTimeOverflow)
}

fn i2c_byte_count(write_len: usize, read_len: usize) -> Result<usize, BoardError> {
    let with_address = write_len
        .checked_add(1)
        .ok_or(BoardError::I2cTimeOverflow)?;
    if read_len == 0 {
        return Ok(with_address);
    }
    with_address
        .checked_add(read_len)
        .and_then(|count| count.checked_add(1))
        .ok_or(BoardError::I2cTimeOverflow)
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
            let (address, device) = match &connection.component.kind {
                BoardComponentKind::Sgp30 { eco2, tvoc } => (
                    SGP30_ADDRESS,
                    BoardI2cDevice::Sgp30(Sgp30::new(*eco2, *tvoc)),
                ),
                BoardComponentKind::M5Pm1 => (M5PM1_ADDRESS, BoardI2cDevice::M5Pm1(M5Pm1::new())),
                BoardComponentKind::Bmi270 => {
                    (BMI270_ADDRESS, BoardI2cDevice::Bmi270(Bmi270::new()))
                }
                BoardComponentKind::Es8311 => {
                    (ES8311_ADDRESS, BoardI2cDevice::Es8311(Es8311::new()))
                }
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
                BoardComponentKind::St7789 { .. } => {
                    return Err(BoardError::I2cComponent {
                        component: connection.component.name.clone(),
                        kind: "ST7789",
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
            let key = (connection.connector.clone(), address);
            if targets
                .insert(key, connection.component.name.clone())
                .is_some()
            {
                return Err(BoardError::Duplicate {
                    kind: "I2C target",
                    name: connection.component.name.clone(),
                });
            }
            devices.insert(connection.component.name.clone(), device);
        }
        Ok(Self {
            hub,
            connectors,
            protocols,
            targets,
            devices,
            available_at: SimTime::ZERO,
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
        if at < self.available_at {
            return Err(BoardError::TimeRegression {
                previous: self.available_at.ticks(),
                next: at.ticks(),
            });
        }
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
        let mut candidate = self
            .devices
            .get(&target)
            .expect("I2C target has a device model")
            .clone();
        let response = candidate.transact(write, read_len, at)?;
        let completed_at = emit_i2c(&self.hub, pins, address, write, &response, at)?;
        self.devices.insert(target, candidate);
        self.available_at = completed_at;
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
        match self.devices.get(name)? {
            BoardI2cDevice::Sgp30(device) => Some(device.snapshot()),
            _ => None,
        }
    }

    /// Returns the current snapshot for a connected M5PM1 target.
    pub fn m5pm1_snapshot(&self, connector: &str) -> Option<M5Pm1Snapshot> {
        let name = self.targets.get(&(connector.to_owned(), M5PM1_ADDRESS))?;
        match self.devices.get(name)? {
            BoardI2cDevice::M5Pm1(device) => Some(device.snapshot()),
            _ => None,
        }
    }

    /// Returns the current snapshot for a connected BMI270 target.
    pub fn bmi270_snapshot(&self, connector: &str) -> Option<Bmi270Snapshot> {
        let name = self.targets.get(&(connector.to_owned(), BMI270_ADDRESS))?;
        match self.devices.get(name)? {
            BoardI2cDevice::Bmi270(device) => Some(device.snapshot()),
            _ => None,
        }
    }

    /// Returns the current snapshot for a connected ES8311 target.
    pub fn es8311_snapshot(&self, connector: &str) -> Option<Es8311Snapshot> {
        let name = self.targets.get(&(connector.to_owned(), ES8311_ADDRESS))?;
        match self.devices.get(name)? {
            BoardI2cDevice::Es8311(device) => Some(device.snapshot()),
            _ => None,
        }
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
    let byte_count = i2c_byte_count(write.len(), read.len())?;
    let _ = i2c_waveform_end(start, byte_count)?;
    let mut now = start.ticks();
    set_line(
        hub,
        pins.data,
        pins.connector_data,
        Logic::Zero,
        SimTime::from_ticks(now),
    )?;
    advance(&mut now)?;
    let mut bytes = vec![(address << 1, false)];
    bytes.extend(write.iter().copied().map(|byte| (byte, false)));
    if !read.is_empty() {
        bytes.push(((address << 1) | 1, false));
        bytes.extend(read.iter().copied().map(|byte| (byte, true)));
    }
    for (byte, _) in bytes {
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
            advance(&mut now)?;
            set_line(
                hub,
                pins.clock,
                pins.connector_clock,
                Logic::One,
                SimTime::from_ticks(now),
            )?;
            advance(&mut now)?;
        }
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
        advance(&mut now)?;
        set_line(
            hub,
            pins.clock,
            pins.connector_clock,
            Logic::One,
            SimTime::from_ticks(now),
        )?;
        advance(&mut now)?;
    }
    set_line(
        hub,
        pins.clock,
        pins.connector_clock,
        Logic::One,
        SimTime::from_ticks(now),
    )?;
    set_line(
        hub,
        pins.data,
        pins.connector_data,
        Logic::Zero,
        SimTime::from_ticks(now),
    )?;
    advance(&mut now)?;
    set_line(
        hub,
        pins.data,
        pins.connector_data,
        Logic::One,
        SimTime::from_ticks(now),
    )?;
    Ok(SimTime::from_ticks(now))
}

fn advance(now: &mut u64) -> Result<(), BoardError> {
    *now = now
        .checked_add(I2C_HALF_TICKS)
        .ok_or(BoardError::I2cTimeOverflow)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_sgp30_transfer_through_machine_connector_signals() {
        let hub = SignalHub::new();
        hub.declare(
            "board.esp32c6.chip_gpio.pin2",
            SignalValue::repeat(Logic::Z, 1).unwrap(),
            None,
        )
        .unwrap();
        hub.declare(
            "board.esp32c6.chip_gpio.pin1",
            SignalValue::repeat(Logic::Z, 1).unwrap(),
            None,
        )
        .unwrap();
        let scenario = BoardScenario {
            name: "nanoc6".to_owned(),
            target: "esp32c6".to_owned(),
            connectors: vec![super::super::BoardConnector {
                name: "grove".to_owned(),
                protocol: ConnectorProtocol::I2c,
                data_pin: 2,
                clock_pin: 1,
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
        let mut endpoint =
            BoardI2cEndpoint::new(&scenario, hub.clone(), "board.esp32c6.chip_gpio").unwrap();
        let transfer = endpoint
            .transfer("grove", SGP30_ADDRESS, &[0x20, 0x03], 0, SimTime::ZERO)
            .unwrap();
        assert!(transfer.completed_at > SimTime::ZERO);
        assert!(endpoint.sgp30_snapshot("grove").unwrap().initialized);
        let error = endpoint
            .transfer("grove", SGP30_ADDRESS, &[0x20, 0x03], 0, SimTime::ZERO)
            .unwrap_err();
        assert!(matches!(
            error,
            BoardError::TimeRegression { previous, next }
                if previous == transfer.completed_at.ticks() && next == 0
        ));
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
        assert!(!hub.drain_changes().is_empty());
    }

    #[test]
    fn rejects_an_i2c_connector_that_aliases_data_and_clock() {
        let scenario = BoardScenario {
            name: "invalid".to_owned(),
            target: "esp32c6".to_owned(),
            connectors: vec![super::super::BoardConnector {
                name: "grove".to_owned(),
                protocol: ConnectorProtocol::I2c,
                data_pin: 2,
                clock_pin: 2,
                voltage_mv: 5_000,
            }],
            mounts: Vec::new(),
            connections: Vec::new(),
            actions: Vec::new(),
            duration: 1,
        };
        let error =
            match BoardI2cEndpoint::new(&scenario, SignalHub::new(), "board.esp32c6.chip_gpio") {
                Err(error) => error,
                Ok(_) => panic!("aliased I2C pins must fail validation"),
            };
        assert!(matches!(
            error,
            BoardError::I2cPinAlias { connector, pin }
                if connector == "grove" && pin == 2
        ));
    }

    #[test]
    fn routes_all_supported_m5sticks3_i2c_devices_atomically() {
        let hub = SignalHub::new();
        for pin in [1, 2] {
            hub.declare(
                format!("board.esp32s3.chip_gpio.pin{pin}"),
                SignalValue::repeat(Logic::Z, 1).unwrap(),
                None,
            )
            .unwrap();
        }
        let scenario = BoardScenario {
            name: "m5sticks3".to_owned(),
            target: "esp32s3".to_owned(),
            connectors: vec![super::super::BoardConnector {
                name: "internal".to_owned(),
                protocol: ConnectorProtocol::I2c,
                data_pin: 2,
                clock_pin: 1,
                voltage_mv: 3_300,
            }],
            mounts: Vec::new(),
            connections: [
                ("power", BoardComponentKind::M5Pm1),
                ("imu", BoardComponentKind::Bmi270),
                ("codec", BoardComponentKind::Es8311),
            ]
            .into_iter()
            .map(|(name, kind)| super::super::BoardConnection {
                connector: "internal".to_owned(),
                component: super::super::BoardComponent {
                    name: name.to_owned(),
                    kind,
                },
            })
            .collect(),
            actions: Vec::new(),
            duration: 1,
        };
        let mut endpoint =
            BoardI2cEndpoint::new(&scenario, hub, "board.esp32s3.chip_gpio").unwrap();

        let power = endpoint
            .transfer("internal", M5PM1_ADDRESS, &[0x00], 4, SimTime::ZERO)
            .unwrap();
        assert_eq!(power.read, [0x01, 0x01, 0x01, b'A']);
        let imu = endpoint
            .transfer("internal", BMI270_ADDRESS, &[0x00], 1, power.completed_at)
            .unwrap();
        assert_eq!(imu.read, [0x24]);
        endpoint
            .transfer(
                "internal",
                ES8311_ADDRESS,
                &[0x00, 0x80],
                0,
                imu.completed_at,
            )
            .unwrap();

        assert_eq!(endpoint.m5pm1_snapshot("internal").unwrap().transactions, 1);
        assert_eq!(
            endpoint.bmi270_snapshot("internal").unwrap().transactions,
            1
        );
        let codec = endpoint.es8311_snapshot("internal").unwrap();
        assert!(codec.powered);
        assert_eq!(codec.transactions, 1);
    }

    #[test]
    fn rejects_i2c_waveform_time_overflow() {
        let hub = SignalHub::new();
        hub.declare(
            "board.esp32c6.chip_gpio.pin2",
            SignalValue::repeat(Logic::Z, 1).unwrap(),
            None,
        )
        .unwrap();
        hub.declare(
            "board.esp32c6.chip_gpio.pin1",
            SignalValue::repeat(Logic::Z, 1).unwrap(),
            None,
        )
        .unwrap();
        let scenario = BoardScenario {
            name: "nanoc6".to_owned(),
            target: "esp32c6".to_owned(),
            connectors: vec![super::super::BoardConnector {
                name: "grove".to_owned(),
                protocol: ConnectorProtocol::I2c,
                data_pin: 2,
                clock_pin: 1,
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
        let mut endpoint =
            BoardI2cEndpoint::new(&scenario, hub.clone(), "board.esp32c6.chip_gpio").unwrap();
        let error = endpoint
            .transfer(
                "grove",
                SGP30_ADDRESS,
                &[0x20, 0x03],
                0,
                SimTime::from_ticks(u64::MAX),
            )
            .unwrap_err();
        assert!(matches!(error, BoardError::I2cTimeOverflow));
        assert!(!endpoint.sgp30_snapshot("grove").unwrap().initialized);
        assert!(hub.drain_changes().is_empty());
        endpoint
            .transfer("grove", SGP30_ADDRESS, &[0x20, 0x03], 0, SimTime::ZERO)
            .unwrap();
        assert!(endpoint.sgp30_snapshot("grove").unwrap().initialized);
    }
}
