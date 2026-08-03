//! Typed host-facing UART endpoint for board models.

use crate::{BoardConnector, ConnectorProtocol};
use remu_core::SimTime;
use remu_devices::{SignalHub, UartHandle};
use remu_signals::{SignalError, SignalId, SignalValue};
use thiserror::Error;

/// Error returned while creating or driving a board UART endpoint.
#[derive(Debug, Error)]
pub enum BoardUartError {
    /// The selected board connector is not a UART connector.
    #[error("connector {connector:?} uses {actual:?}, not UART")]
    Protocol {
        /// Connector name.
        connector: String,
        /// Declared connector protocol.
        actual: ConnectorProtocol,
    },
    /// The connector aliases its TX and RX pins.
    #[error("UART connector {connector:?} aliases TX and RX on GPIO pin {pin}")]
    PinAlias {
        /// Connector name.
        connector: String,
        /// Aliased GPIO number.
        pin: u8,
    },
    /// An endpoint operation moved backwards on the simulation timeline.
    #[error("UART endpoint time regressed from {previous} to {next}")]
    TimeRegression {
        /// Previous endpoint timestamp.
        previous: SimTime,
        /// New endpoint timestamp.
        next: SimTime,
    },
    /// Signal declaration or update failed.
    #[error(transparent)]
    Signal(#[from] SignalError),
}

/// Signal IDs and host transport state for one board UART connector.
///
/// The endpoint keeps the machine's existing UartHandle as the source of TX
/// bytes and queues host-supplied RX bytes into that same handle. This keeps
/// board code independent from any one CPU architecture while preserving the
/// simulator's deterministic signal/VCD path.
pub struct BoardUartEndpoint {
    connector: String,
    uart: UartHandle,
    tx_byte: SignalId,
    tx_strobe: SignalId,
    rx_byte: SignalId,
    rx_strobe: SignalId,
    hub: SignalHub,
    observed_tx: usize,
    tx_strobe_level: bool,
    rx_strobe_level: bool,
    last_at: Option<SimTime>,
}

impl BoardUartEndpoint {
    /// Creates an endpoint for a Starlark board connector and machine UART.
    pub fn new(
        board_name: &str,
        connector: &BoardConnector,
        uart: UartHandle,
        hub: SignalHub,
    ) -> Result<Self, BoardUartError> {
        if connector.protocol != ConnectorProtocol::Uart {
            return Err(BoardUartError::Protocol {
                connector: connector.name.clone(),
                actual: connector.protocol,
            });
        }
        if connector.data_pin == connector.clock_pin {
            return Err(BoardUartError::PinAlias {
                connector: connector.name.clone(),
                pin: connector.data_pin,
            });
        }
        let prefix = format!("board.{board_name}.connector.{}", connector.name);
        let tx_byte = hub.declare(
            format!("{prefix}.tx_byte"),
            SignalValue::from_u64(0, 8)?,
            Some("UART bytes transmitted by the machine".to_owned()),
        )?;
        let tx_strobe = hub.declare(
            format!("{prefix}.tx_strobe"),
            SignalValue::from_u64(0, 1)?,
            Some("UART transmit byte strobe".to_owned()),
        )?;
        let rx_byte = hub.declare(
            format!("{prefix}.rx_byte"),
            SignalValue::from_u64(0, 8)?,
            Some("UART bytes supplied by the host".to_owned()),
        )?;
        let rx_strobe = hub.declare(
            format!("{prefix}.rx_strobe"),
            SignalValue::from_u64(0, 1)?,
            Some("UART receive byte strobe".to_owned()),
        )?;
        Ok(Self {
            connector: connector.name.clone(),
            uart,
            tx_byte,
            tx_strobe,
            rx_byte,
            rx_strobe,
            hub,
            observed_tx: 0,
            tx_strobe_level: false,
            rx_strobe_level: false,
            last_at: None,
        })
    }

    fn check_time(&mut self, at: SimTime) -> Result<(), BoardUartError> {
        if let Some(previous) = self.last_at {
            if at < previous {
                return Err(BoardUartError::TimeRegression { previous, next: at });
            }
        }
        self.last_at = Some(at);
        Ok(())
    }

    /// Connector name used in signal paths and diagnostics.
    pub fn connector(&self) -> &str {
        &self.connector
    }

    /// Queues bytes for the guest's UART receive register and emits RX signals.
    pub fn host_write(&mut self, bytes: &[u8], at: SimTime) -> Result<(), BoardUartError> {
        self.check_time(at)?;
        self.uart.feed_rx(bytes);
        for byte in bytes {
            self.rx_strobe_level = !self.rx_strobe_level;
            self.hub.set(
                self.rx_byte,
                SignalValue::from_u64(u64::from(*byte), 8)?,
                at,
            )?;
            self.hub.set(
                self.rx_strobe,
                SignalValue::from_u64(u64::from(self.rx_strobe_level), 1)?,
                at,
            )?;
        }
        Ok(())
    }

    /// Returns newly transmitted guest bytes and emits TX signals for them.
    pub fn poll_tx(&mut self, at: SimTime) -> Result<Vec<u8>, BoardUartError> {
        self.check_time(at)?;
        let bytes = self.uart.bytes();
        if self.observed_tx > bytes.len() {
            self.observed_tx = 0;
        }
        let new_bytes = bytes[self.observed_tx..].to_vec();
        for byte in &new_bytes {
            self.tx_strobe_level = !self.tx_strobe_level;
            self.hub.set(
                self.tx_byte,
                SignalValue::from_u64(u64::from(*byte), 8)?,
                at,
            )?;
            self.hub.set(
                self.tx_strobe,
                SignalValue::from_u64(u64::from(self.tx_strobe_level), 1)?,
                at,
            )?;
        }
        self.observed_tx = bytes.len();
        Ok(new_bytes)
    }

    /// Number of bytes currently queued for guest RX.
    pub fn pending_rx(&self) -> usize {
        self.uart.rx_len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::Device;
    use remu_devices::FunctionalUart;
    use remu_signals::Logic;

    fn connector(protocol: ConnectorProtocol) -> BoardConnector {
        BoardConnector {
            name: "console".to_owned(),
            protocol,
            data_pin: 1,
            clock_pin: 2,
            voltage_mv: 3300,
        }
    }

    #[test]
    fn bridges_host_rx_and_machine_tx_with_typed_signals() {
        let hub = SignalHub::new();
        let (mut uart, handle) = FunctionalUart::new("uart", 0, 4, 1);
        let mut endpoint = BoardUartEndpoint::new(
            "teaching",
            &connector(ConnectorProtocol::Uart),
            handle.clone(),
            hub.clone(),
        )
        .unwrap();
        endpoint.host_write(b"R", SimTime::from_ticks(2)).unwrap();
        assert_eq!(endpoint.pending_rx(), 1);
        assert_eq!(
            uart.read(0, remu_core::AccessWidth::Word, SimTime::from_ticks(3))
                .unwrap(),
            u64::from(82_u8)
        );
        uart.write(
            0,
            remu_core::AccessWidth::Word,
            u64::from(b"OK"[0]),
            SimTime::from_ticks(4),
        )
        .unwrap();
        uart.write(
            0,
            remu_core::AccessWidth::Word,
            u64::from(b"OK"[1]),
            SimTime::from_ticks(4),
        )
        .unwrap();
        assert_eq!(endpoint.poll_tx(SimTime::from_ticks(4)).unwrap(), b"OK");
        assert_eq!(endpoint.poll_tx(SimTime::from_ticks(5)).unwrap(), b"");
        assert_eq!(
            hub.with_registry(|registry| registry
                .find("board.teaching.connector.console.tx_byte")
                .and_then(|signal| registry.value(signal))
                .and_then(|value| value.bit(0))),
            Some(Logic::One)
        );
    }

    #[test]
    fn rejects_non_uart_connectors() {
        let result = BoardUartEndpoint::new(
            "teaching",
            &connector(ConnectorProtocol::I2c),
            UartHandle::default(),
            SignalHub::new(),
        );
        assert!(matches!(result, Err(BoardUartError::Protocol { .. })));
    }

    #[test]
    fn rejects_endpoint_time_regression() {
        let (_, handle) = FunctionalUart::new("uart", 0, 4, 1);
        let mut endpoint = BoardUartEndpoint::new(
            "teaching",
            &connector(ConnectorProtocol::Uart),
            handle,
            SignalHub::new(),
        )
        .unwrap();
        endpoint.host_write(b"R", SimTime::from_ticks(5)).unwrap();
        assert!(matches!(
            endpoint.poll_tx(SimTime::from_ticks(4)),
            Err(BoardUartError::TimeRegression { .. })
        ));
    }

    #[test]
    fn rejects_uart_connector_pin_alias() {
        let mut aliased = connector(ConnectorProtocol::Uart);
        aliased.clock_pin = aliased.data_pin;
        let (_, handle) = FunctionalUart::new("uart", 0, 4, 1);
        let result = BoardUartEndpoint::new("teaching", &aliased, handle, SignalHub::new());
        assert!(matches!(
            result,
            Err(BoardUartError::PinAlias { connector, pin })
                if connector == "console" && pin == 1
        ));
    }
}
