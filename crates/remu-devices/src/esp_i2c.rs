use super::*;

/// ESP32-C6 I2C controller register block.
///
/// The controller executes the programmed hardware command list as one
/// deterministic functional transaction when `CTR.TRANS_START` is written.
/// This is intentionally a protocol model rather than a clock-accurate I2C
/// state machine: the command/FIFO registers, completion status, and a default
/// SGP30 at address `0x58` are enough for firmware-level sensor-driver tests.
pub struct Esp32c6I2c {
    name: String,
    registers: [u32; 0x100 / 4],
    commands: [u32; 8],
    tx_fifo: VecDeque<u8>,
    rx_fifo: VecDeque<u8>,
    int_raw: u32,
    int_ena: u32,
    response_nack: bool,
    sensor: Sgp30,
    sda: SignalId,
    scl: SignalId,
    hub: SignalHub,
}

/// Command opcodes encoded in the ESP32-C6 I2C command registers.
///
/// The executable ESP-IDF C6 HAL defines restart as `6`, write as `1`, read
/// as `3`, stop as `2`, and end as `4`.  The generated register commentary
/// currently describes a conflicting compact `0..=4` sequence; the HAL values
/// are the compatibility contract used by real C6 firmware.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
enum CommandOpcode {
    Restart = 6,
    Write = 1,
    Read = 3,
    Stop = 2,
    End = 4,
}

impl TryFrom<u32> for CommandOpcode {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Write),
            2 => Ok(Self::Stop),
            3 => Ok(Self::Read),
            4 => Ok(Self::End),
            6 => Ok(Self::Restart),
            _ => Err(()),
        }
    }
}

impl Esp32c6I2c {
    /// Register offset for the low SCL period.
    pub const SCL_LOW_PERIOD: u64 = 0x00;
    /// Register offset for the controller control word.
    pub const CTR: u64 = 0x04;
    /// Register offset for controller status.
    pub const SR: u64 = 0x08;
    /// Register offset for timeout configuration.
    pub const TIMEOUT: u64 = 0x0c;
    /// Register offset for the local slave address.
    pub const SLAVE_ADDR: u64 = 0x10;
    /// Register offset for FIFO pointer status.
    pub const FIFO_STATUS: u64 = 0x14;
    /// Register offset for FIFO configuration.
    pub const FIFO_CONF: u64 = 0x18;
    /// Register offset for the APB FIFO data port.
    pub const DATA: u64 = 0x1c;
    /// Register offset for raw interrupt status.
    pub const INT_RAW: u64 = 0x20;
    /// Register offset for write-to-clear interrupt status.
    pub const INT_CLEAR: u64 = 0x24;
    /// Register offset for interrupt enables.
    pub const INT_ENABLE: u64 = 0x28;
    /// Register offset for masked interrupt status.
    pub const INT_STATUS: u64 = 0x2c;
    /// First hardware command register offset.
    pub const COMMAND0: u64 = 0x58;
    /// Register offset for the controller version word.
    pub const DATE: u64 = 0xf8;

    const TRANS_START: u32 = 1 << 5;
    const FIFO_RX_RESET: u32 = 1 << 12;
    const FIFO_TX_RESET: u32 = 1 << 13;
    const INT_RXFIFO_WM: u32 = 1 << 0;
    const INT_END_DETECT: u32 = 1 << 3;
    const INT_BYTE_TRANS_DONE: u32 = 1 << 4;
    const INT_TRANS_COMPLETE: u32 = 1 << 7;
    const INT_NACK: u32 = 1 << 10;
    const INT_RXFIFO_UDF: u32 = 1 << 12;
    const COMMAND_DONE: u32 = 1 << 31;
    const COMMAND_MASK: u32 = 0x3fff;
    const FIFO_CAPACITY: usize = 32;

    /// Creates an ESP32-C6 controller with a deterministic SGP30 at `0x58`.
    pub fn new(name: impl Into<String>, hub: SignalHub) -> Result<Self, SignalError> {
        let name = name.into();
        let sda = hub.declare(
            format!("board.{name}.sda"),
            SignalValue::repeat(Logic::One, 1)?,
            Some("ESP32-C6 I2C SDA (functional waveform)".to_owned()),
        )?;
        let scl = hub.declare(
            format!("board.{name}.scl"),
            SignalValue::repeat(Logic::One, 1)?,
            Some("ESP32-C6 I2C SCL (functional waveform)".to_owned()),
        )?;
        let mut device = Self {
            name,
            registers: [0; 0x100 / 4],
            commands: [0; 8],
            tx_fifo: VecDeque::with_capacity(Self::FIFO_CAPACITY),
            rx_fifo: VecDeque::with_capacity(Self::FIFO_CAPACITY),
            int_raw: 1 << 1,
            int_ena: 0,
            response_nack: false,
            sensor: Sgp30::new(420, 8),
            sda,
            scl,
            hub,
        };
        device.reset_registers();
        Ok(device)
    }

    fn reset_registers(&mut self) {
        self.registers.fill(0);
        self.commands.fill(0);
        self.tx_fifo.clear();
        self.rx_fifo.clear();
        self.int_raw = 1 << 1;
        self.int_ena = 0;
        self.response_nack = false;
        self.registers[Self::CTR as usize / 4] = (1 << 3) | (1 << 9);
        self.registers[Self::FIFO_CONF as usize / 4] = 11 | (4 << 5) | (1 << 14);
        self.registers[Self::DATE as usize / 4] = 35_656_050;
    }

    fn register_value(&self, offset: u64, _at: SimTime) -> u32 {
        match offset {
            Self::SR => self.status_value(),
            Self::FIFO_STATUS => self.fifo_status_value(),
            Self::DATA => self.rx_fifo.front().copied().unwrap_or(0).into(),
            Self::INT_RAW => self.int_raw,
            Self::INT_ENABLE => self.int_ena,
            Self::INT_STATUS => self.int_raw & self.int_ena,
            offset @ Self::COMMAND0..=0x74 if offset & 3 == 0 => {
                self.commands[((offset - Self::COMMAND0) / 4) as usize]
            }
            _ => self
                .registers
                .get(offset as usize / 4)
                .copied()
                .unwrap_or(0),
        }
    }

    fn status_value(&self) -> u32 {
        let mut status = u32::from(self.response_nack);
        status |= (u32::try_from(self.rx_fifo.len())
            .unwrap_or(u32::MAX)
            .min(0x3f))
            << 8;
        status |= (u32::try_from(self.tx_fifo.len())
            .unwrap_or(u32::MAX)
            .min(0x3f))
            << 18;
        status
    }

    fn fifo_status_value(&self) -> u32 {
        let rx = u32::try_from(self.rx_fifo.len()).unwrap_or(u32::MAX) & 0x1f;
        let tx = u32::try_from(self.tx_fifo.len()).unwrap_or(u32::MAX) & 0x1f;
        rx | (rx << 5) | (tx << 10) | (tx << 15)
    }

    fn write_register(&mut self, offset: u64, value: u32, at: SimTime) -> Result<(), DeviceError> {
        match offset {
            Self::DATA => {
                if self.tx_fifo.len() >= Self::FIFO_CAPACITY {
                    self.int_raw |= 1 << 11;
                } else {
                    self.tx_fifo.push_back((value & 0xff) as u8);
                }
            }
            Self::CTR => {
                self.registers[Self::CTR as usize / 4] = value & !(Self::TRANS_START);
                if value & Self::TRANS_START != 0 {
                    self.execute(at)?;
                }
            }
            Self::FIFO_CONF => {
                self.registers[Self::FIFO_CONF as usize / 4] = value & 0x7fff;
                if value & Self::FIFO_RX_RESET != 0 {
                    self.rx_fifo.clear();
                }
                if value & Self::FIFO_TX_RESET != 0 {
                    self.tx_fifo.clear();
                }
            }
            Self::INT_CLEAR => self.int_raw &= !value,
            Self::INT_ENABLE => self.int_ena = value & 0x7ffff,
            offset @ Self::COMMAND0..=0x74 if offset & 3 == 0 => {
                self.commands[((offset - Self::COMMAND0) / 4) as usize] =
                    value & Self::COMMAND_MASK;
            }
            Self::SR | Self::FIFO_STATUS | Self::INT_RAW | Self::INT_STATUS | Self::DATE => {
                return Err(DeviceError::new(format!(
                    "ESP32-C6 I2C register at offset {offset:#x} is read-only"
                )));
            }
            offset if offset < 0x100 && offset & 3 == 0 => {
                self.registers[offset as usize / 4] = value;
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled ESP32-C6 I2C write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn execute(&mut self, at: SimTime) -> Result<(), DeviceError> {
        self.rx_fifo.clear();
        self.int_raw &= !(Self::INT_END_DETECT
            | Self::INT_BYTE_TRANS_DONE
            | Self::INT_TRANS_COMPLETE
            | Self::INT_NACK
            | Self::INT_RXFIFO_WM
            | Self::INT_RXFIFO_UDF);
        self.response_nack = false;
        let tx = self.tx_fifo.drain(..).collect::<Vec<_>>();
        let mut tx_index = 0_usize;
        let mut address = None;
        let mut write_payload = Vec::new();
        let mut read_len = 0_usize;
        let mut awaiting_address = true;
        let mut complete = false;
        let command_count = self.commands.len();
        for (index, command) in self.commands.iter_mut().enumerate() {
            let word = *command;
            let byte_count = usize::from((word & 0xff) as u8);
            let opcode = CommandOpcode::try_from((word >> 11) & 0x7);
            match opcode {
                Ok(CommandOpcode::Restart) => awaiting_address = true,
                Ok(CommandOpcode::Write) => {
                    let end = tx_index.saturating_add(byte_count);
                    if end > tx.len() {
                        self.response_nack = true;
                        break;
                    }
                    let bytes = &tx[tx_index..end];
                    tx_index = end;
                    if awaiting_address {
                        let Some(first) = bytes.first() else {
                            self.response_nack = true;
                            break;
                        };
                        address = Some(first >> 1);
                        awaiting_address = false;
                        if first & 1 != 0 {
                            // A write opcode carrying a read address is used
                            // by ESP-IDF before a following READ command.
                            read_len = 0;
                        } else {
                            write_payload.extend_from_slice(&bytes[1..]);
                        }
                    } else {
                        write_payload.extend_from_slice(bytes);
                    }
                }
                Ok(CommandOpcode::Read) => read_len = read_len.saturating_add(byte_count),
                Ok(CommandOpcode::Stop) => complete = true,
                Ok(CommandOpcode::End) => {
                    complete = true;
                    break;
                }
                Err(()) => {
                    self.response_nack = true;
                    break;
                }
            }
            *command = (word & Self::COMMAND_MASK) | Self::COMMAND_DONE;
            if index == command_count - 1 {
                complete = true;
            }
        }
        if !complete {
            self.response_nack = true;
        }

        if !self.response_nack {
            match address {
                Some(SGP30_ADDRESS) => {
                    let response = self
                        .sensor
                        .transact(&write_payload, read_len, at)
                        .map_err(|error| DeviceError::new(error.to_string()))?;
                    if response.len() > Self::FIFO_CAPACITY {
                        self.response_nack = true;
                    } else {
                        self.rx_fifo.extend(response);
                    }
                }
                Some(_) | None => self.response_nack = true,
            }
        }
        if self.response_nack {
            self.int_raw |= Self::INT_NACK;
        } else {
            self.int_raw |=
                Self::INT_END_DETECT | Self::INT_BYTE_TRANS_DONE | Self::INT_TRANS_COMPLETE;
            let threshold = (self.registers[Self::FIFO_CONF as usize / 4] & 0x1f) as usize;
            if threshold != 0 && self.rx_fifo.len() >= threshold {
                self.int_raw |= Self::INT_RXFIFO_WM;
            }
        }
        self.emit_waveform(&tx, at)?;
        Ok(())
    }

    fn emit_waveform(&self, bytes: &[u8], at: SimTime) -> Result<(), DeviceError> {
        let high = SignalValue::repeat(Logic::One, 1).expect("one-bit signal");
        let low = SignalValue::repeat(Logic::Zero, 1).expect("one-bit signal");
        self.hub
            .set(self.sda, high.clone(), at)
            .map_err(|error| DeviceError::new(error.to_string()))?;
        self.hub
            .set(self.scl, high.clone(), at)
            .map_err(|error| DeviceError::new(error.to_string()))?;
        for byte in bytes {
            for bit in (0..8).rev() {
                let value = if byte & (1 << bit) == 0 {
                    low.clone()
                } else {
                    high.clone()
                };
                self.hub
                    .set(self.sda, value, at)
                    .map_err(|error| DeviceError::new(error.to_string()))?;
                self.hub
                    .set(self.scl, low.clone(), at)
                    .map_err(|error| DeviceError::new(error.to_string()))?;
                self.hub
                    .set(self.scl, high.clone(), at)
                    .map_err(|error| DeviceError::new(error.to_string()))?;
            }
        }
        self.hub
            .set(self.sda, high, at)
            .map_err(|error| DeviceError::new(error.to_string()))?;
        Ok(())
    }

    /// Returns the deterministic sensor state for host-side qualification.
    pub fn sensor_snapshot(&self) -> Sgp30Snapshot {
        self.sensor.snapshot()
    }
}

impl Device for Esp32c6I2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-C6 I2C requires aligned word access",
            ));
        }
        if offset == Self::DATA {
            let value = self.rx_fifo.pop_front().unwrap_or_else(|| {
                self.int_raw |= Self::INT_RXFIFO_UDF;
                0
            });
            return Ok(u64::from(value));
        }
        Ok(u64::from(self.register_value(offset, at)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-C6 I2C requires aligned word access",
            ));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("word access is masked");
        self.write_register(offset, value, at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset_registers();
        self.sensor = Sgp30::new(420, 8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn command(bytes: u32, opcode: CommandOpcode) -> u32 {
        bytes | ((opcode as u32) << 11)
    }

    fn write_word(device: &mut Esp32c6I2c, offset: u64, value: u32, at: SimTime) {
        device
            .write(offset, AccessWidth::Word, u64::from(value), at)
            .unwrap();
    }

    fn read_word(device: &mut Esp32c6I2c, offset: u64, at: SimTime) -> u32 {
        device.read(offset, AccessWidth::Word, at).unwrap() as u32
    }

    #[test]
    fn executes_sgp30_init_and_measure_commands() {
        let mut device = Esp32c6I2c::new("esp32c6.i2c0", SignalHub::new()).unwrap();
        let now = SimTime::from_ticks(Sgp30::WARMUP_TICKS);
        for byte in [0xb0, 0x20, 0x03] {
            write_word(&mut device, Esp32c6I2c::DATA, byte, now);
        }
        for (index, value) in [
            command(0, CommandOpcode::Restart),
            command(3, CommandOpcode::Write),
            command(0, CommandOpcode::Stop),
            command(0, CommandOpcode::End),
        ]
        .into_iter()
        .enumerate()
        {
            write_word(
                &mut device,
                Esp32c6I2c::COMMAND0 + (index as u64 * 4),
                value,
                now,
            );
        }
        write_word(&mut device, Esp32c6I2c::CTR, 0x30, now);
        assert_eq!(device.sensor_snapshot().commands, 1);
        assert_eq!(
            read_word(&mut device, Esp32c6I2c::INT_RAW, now) & (1 << 10),
            0
        );

        for byte in [0xb0, 0x20, 0x08, 0xb1] {
            write_word(&mut device, Esp32c6I2c::DATA, byte, now);
        }
        for (index, value) in [
            command(0, CommandOpcode::Restart),
            command(3, CommandOpcode::Write),
            command(0, CommandOpcode::Restart),
            command(1, CommandOpcode::Write),
            command(6, CommandOpcode::Read),
            command(0, CommandOpcode::Stop),
            command(0, CommandOpcode::End),
        ]
        .into_iter()
        .enumerate()
        {
            write_word(
                &mut device,
                Esp32c6I2c::COMMAND0 + (index as u64 * 4),
                value,
                now,
            );
        }
        write_word(&mut device, Esp32c6I2c::CTR, 0x30, now);
        let measurement = (0..6)
            .map(|_| read_word(&mut device, Esp32c6I2c::DATA, now) as u8)
            .collect::<Vec<_>>();
        assert_eq!(measurement.len(), 6);
        assert_eq!(device.sensor_snapshot().commands, 2);
        assert_eq!(read_word(&mut device, Esp32c6I2c::INT_STATUS, now), 0);
    }

    #[test]
    fn reports_nack_for_unknown_address_and_clears_interrupts() {
        let mut device = Esp32c6I2c::new("esp32c6.i2c0", SignalHub::new()).unwrap();
        let now = SimTime::ZERO;
        write_word(&mut device, Esp32c6I2c::DATA, 0x80, now);
        for (index, value) in [
            command(0, CommandOpcode::Restart),
            command(1, CommandOpcode::Write),
            command(0, CommandOpcode::Stop),
        ]
        .into_iter()
        .enumerate()
        {
            write_word(
                &mut device,
                Esp32c6I2c::COMMAND0 + (index as u64 * 4),
                value,
                now,
            );
        }
        write_word(&mut device, Esp32c6I2c::CTR, 0x30, now);
        assert_ne!(
            read_word(&mut device, Esp32c6I2c::INT_RAW, now) & (1 << 10),
            0
        );
        write_word(&mut device, Esp32c6I2c::INT_CLEAR, u32::MAX, now);
        assert_eq!(
            read_word(&mut device, Esp32c6I2c::INT_RAW, now) & (1 << 10),
            0
        );
    }

    #[test]
    fn uses_the_esp_idf_c6_command_opcode_encoding() {
        assert_eq!(CommandOpcode::Restart as u32, 6);
        assert_eq!(CommandOpcode::Write as u32, 1);
        assert_eq!(CommandOpcode::Read as u32, 3);
        assert_eq!(CommandOpcode::Stop as u32, 2);
        assert_eq!(CommandOpcode::End as u32, 4);
        assert!(CommandOpcode::try_from(0).is_err());
        assert!(CommandOpcode::try_from(5).is_err());
        assert!(CommandOpcode::try_from(7).is_err());
    }

    #[test]
    fn rejects_reserved_command_opcode_before_sensor_transaction() {
        let mut device = Esp32c6I2c::new("esp32c6.i2c0", SignalHub::new()).unwrap();
        let now = SimTime::ZERO;
        write_word(&mut device, Esp32c6I2c::DATA, 0xb0, now);
        write_word(&mut device, Esp32c6I2c::COMMAND0, 0 << 11, now);
        write_word(&mut device, Esp32c6I2c::CTR, Esp32c6I2c::TRANS_START, now);
        assert_ne!(
            read_word(&mut device, Esp32c6I2c::INT_RAW, now) & Esp32c6I2c::INT_NACK,
            0
        );
        assert_eq!(device.sensor_snapshot().commands, 0);
    }
}
