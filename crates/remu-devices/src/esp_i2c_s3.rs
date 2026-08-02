//! Functional ESP32-S3 I2C controller models.

use super::*;

/// ESP32-S3 I2C master register/FIFO slice.
///
/// The model executes the hardware command list synchronously when
/// `CTR.TRANS_START` is written. It preserves the documented FIFO, command,
/// status, interrupt, and waveform surfaces while deliberately leaving out
/// bit-level clock stretching and arbitration timing.
pub struct Esp32s3I2c {
    name: String,
    registers: [u32; 0x100 / 4],
    commands: [u32; 8],
    tx_fifo: VecDeque<u8>,
    rx_fifo: VecDeque<u8>,
    int_raw: u32,
    int_ena: u32,
    nack: bool,
    sensor: Sgp30,
    sda: SignalId,
    scl: SignalId,
    hub: SignalHub,
}

/// ESP32-S3 I2C command-list opcodes defined by the vendor register block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
enum CommandOpcode {
    Restart = 0,
    Write = 1,
    Read = 2,
    Stop = 3,
    End = 4,
}

impl TryFrom<u32> for CommandOpcode {
    type Error = ();

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Restart),
            1 => Ok(Self::Write),
            2 => Ok(Self::Read),
            3 => Ok(Self::Stop),
            4 => Ok(Self::End),
            _ => Err(()),
        }
    }
}

impl Esp32s3I2c {
    /// SCL low-period register offset.
    pub const SCL_LOW_PERIOD: u64 = 0x00;
    /// Controller control register offset.
    pub const CTR: u64 = 0x04;
    /// Controller status register offset.
    pub const SR: u64 = 0x08;
    /// Timeout register offset.
    pub const TIMEOUT: u64 = 0x0c;
    /// Local slave-address register offset.
    pub const SLAVE_ADDR: u64 = 0x10;
    /// FIFO pointer status register offset.
    pub const FIFO_STATUS: u64 = 0x14;
    /// FIFO configuration register offset.
    pub const FIFO_CONF: u64 = 0x18;
    /// FIFO data register offset.
    pub const DATA: u64 = 0x1c;
    /// Raw interrupt status register offset.
    pub const INT_RAW: u64 = 0x20;
    /// Write-to-clear interrupt register offset.
    pub const INT_CLEAR: u64 = 0x24;
    /// Interrupt-enable register offset.
    pub const INT_ENABLE: u64 = 0x28;
    /// Masked interrupt status register offset.
    pub const INT_STATUS: u64 = 0x2c;
    /// First command register offset.
    pub const COMMAND0: u64 = 0x58;
    /// Date/version register offset.
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

    /// Creates an I2C controller with a deterministic SGP30 at address `0x58`.
    pub fn new(name: impl Into<String>, hub: SignalHub) -> Result<Self, SignalError> {
        let name = name.into();
        let sda = hub.declare(
            format!("board.{name}.sda"),
            SignalValue::repeat(Logic::One, 1)?,
            Some("ESP32-S3 I2C SDA functional waveform".to_owned()),
        )?;
        let scl = hub.declare(
            format!("board.{name}.scl"),
            SignalValue::repeat(Logic::One, 1)?,
            Some("ESP32-S3 I2C SCL functional waveform".to_owned()),
        )?;
        let mut device = Self {
            name,
            registers: [0; 0x100 / 4],
            commands: [0; 8],
            tx_fifo: VecDeque::with_capacity(Self::FIFO_CAPACITY),
            rx_fifo: VecDeque::with_capacity(Self::FIFO_CAPACITY),
            int_raw: 1 << 1,
            int_ena: 0,
            nack: false,
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
        self.nack = false;
        self.registers[Self::CTR as usize / 4] = (1 << 3) | (1 << 9);
        self.registers[Self::FIFO_CONF as usize / 4] = 11 | (4 << 5) | (1 << 14);
        self.registers[Self::DATE as usize / 4] = 35_656_050;
    }

    fn status(&self) -> u32 {
        u32::from(self.nack)
            | (u32::try_from(self.rx_fifo.len())
                .unwrap_or(u32::MAX)
                .min(0x3f)
                << 8)
            | (u32::try_from(self.tx_fifo.len())
                .unwrap_or(u32::MAX)
                .min(0x3f)
                << 18)
    }

    fn fifo_status(&self) -> u32 {
        let rx = u32::try_from(self.rx_fifo.len()).unwrap_or(u32::MAX) & 0x1f;
        let tx = u32::try_from(self.tx_fifo.len()).unwrap_or(u32::MAX) & 0x1f;
        rx | (rx << 5) | (tx << 10) | (tx << 15)
    }

    fn read_register(&self, offset: u64) -> u32 {
        match offset {
            Self::SR => self.status(),
            Self::FIFO_STATUS => self.fifo_status(),
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

    fn write_register(&mut self, offset: u64, value: u32, at: SimTime) -> Result<(), DeviceError> {
        match offset {
            Self::DATA => {
                if self.tx_fifo.len() >= Self::FIFO_CAPACITY {
                    self.int_raw |= 1 << 11;
                } else {
                    self.tx_fifo.push_back(value as u8);
                }
            }
            Self::CTR => {
                self.registers[Self::CTR as usize / 4] = value & !Self::TRANS_START;
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
                    "ESP32-S3 I2C register at offset {offset:#x} is read-only"
                )));
            }
            offset if offset < 0x100 && offset & 3 == 0 => {
                self.registers[offset as usize / 4] = value;
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled ESP32-S3 I2C write at offset {offset:#x}"
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
        self.nack = false;
        let tx = self.tx_fifo.drain(..).collect::<Vec<_>>();
        let mut tx_index: usize = 0;
        let mut address = None;
        let mut write_payload = Vec::new();
        let mut read_len: usize = 0;
        let mut awaiting_address = true;
        let mut complete = false;
        let command_count = self.commands.len();
        for (index, command) in self.commands.iter_mut().enumerate() {
            let word = *command;
            let byte_count = (word & 0xff) as usize;
            let opcode = CommandOpcode::try_from((word >> 11) & 0x7);
            match opcode {
                Ok(CommandOpcode::Restart) => awaiting_address = true,
                Ok(CommandOpcode::Write) => {
                    let end = tx_index.saturating_add(byte_count);
                    if end > tx.len() {
                        self.nack = true;
                        break;
                    }
                    let bytes = &tx[tx_index..end];
                    tx_index = end;
                    if awaiting_address {
                        let Some(first) = bytes.first() else {
                            self.nack = true;
                            break;
                        };
                        address = Some(first >> 1);
                        awaiting_address = false;
                        if first & 1 == 0 {
                            write_payload.extend_from_slice(&bytes[1..]);
                        }
                    } else {
                        write_payload.extend_from_slice(bytes);
                    }
                }
                Ok(CommandOpcode::Read) => read_len = read_len.saturating_add(byte_count),
                Ok(CommandOpcode::Stop) => {
                    complete = true;
                    awaiting_address = true;
                }
                Ok(CommandOpcode::End) => {
                    complete = true;
                    break;
                }
                Err(()) => {
                    self.nack = true;
                    break;
                }
            }
            *command = (word & Self::COMMAND_MASK) | Self::COMMAND_DONE;
            if index == command_count - 1 {
                complete = true;
            }
        }
        if !complete {
            self.nack = true;
        }
        if !self.nack {
            match address {
                Some(SGP30_ADDRESS) => {
                    let response = self
                        .sensor
                        .transact(&write_payload, read_len, at)
                        .map_err(|error| DeviceError::new(error.to_string()))?;
                    if response.len() > Self::FIFO_CAPACITY {
                        self.nack = true;
                    } else {
                        self.rx_fifo.extend(response);
                    }
                }
                Some(_) | None => self.nack = true,
            }
        }
        if self.nack {
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

impl Device for Esp32s3I2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 I2C requires aligned word access",
            ));
        }
        if offset == Self::DATA {
            let value = self.rx_fifo.pop_front().unwrap_or_else(|| {
                self.int_raw |= Self::INT_RXFIFO_UDF;
                0
            });
            return Ok(u64::from(value));
        }
        Ok(u64::from(self.read_register(offset)))
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
                "ESP32-S3 I2C requires aligned word access",
            ));
        }
        self.write_register(offset, value as u32, at)
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

    fn write_word(device: &mut Esp32s3I2c, offset: u64, value: u32, at: SimTime) {
        device
            .write(offset, AccessWidth::Word, u64::from(value), at)
            .unwrap();
    }

    fn read_word(device: &mut Esp32s3I2c, offset: u64, at: SimTime) -> u32 {
        device.read(offset, AccessWidth::Word, at).unwrap() as u32
    }

    fn program(device: &mut Esp32s3I2c, bytes: &[u8], commands: &[u32], at: SimTime) {
        for byte in bytes {
            write_word(device, Esp32s3I2c::DATA, u32::from(*byte), at);
        }
        for (index, command) in commands.iter().copied().enumerate() {
            write_word(
                device,
                Esp32s3I2c::COMMAND0 + (index as u64 * 4),
                command,
                at,
            );
        }
        write_word(device, Esp32s3I2c::CTR, 0x30, at);
    }

    #[test]
    fn executes_sgp30_init_and_measurement() {
        let mut device = Esp32s3I2c::new("esp32s3.i2c0", SignalHub::new()).unwrap();
        let at = SimTime::from_ticks(Sgp30::WARMUP_TICKS);
        program(
            &mut device,
            &[0xb0, 0x20, 0x03],
            &[
                command(0, CommandOpcode::Restart),
                command(3, CommandOpcode::Write),
                command(0, CommandOpcode::Stop),
                command(0, CommandOpcode::End),
            ],
            SimTime::ZERO,
        );
        assert_eq!(device.sensor_snapshot().commands, 1);
        program(
            &mut device,
            &[0xb0, 0x20, 0x08, 0xb1],
            &[
                command(0, CommandOpcode::Restart),
                command(3, CommandOpcode::Write),
                command(0, CommandOpcode::Restart),
                command(1, CommandOpcode::Write),
                command(6, CommandOpcode::Read),
                command(0, CommandOpcode::Stop),
                command(0, CommandOpcode::End),
            ],
            at,
        );
        let measurement = (0..6)
            .map(|_| read_word(&mut device, Esp32s3I2c::DATA, at) as u8)
            .collect::<Vec<_>>();
        assert_eq!(measurement[0..2], [1, 164]);
        assert_eq!(device.sensor_snapshot().commands, 2);
    }

    #[test]
    fn reports_nack_for_unknown_address() {
        let mut device = Esp32s3I2c::new("esp32s3.i2c1", SignalHub::new()).unwrap();
        program(
            &mut device,
            &[0x80],
            &[
                command(0, CommandOpcode::Restart),
                command(1, CommandOpcode::Write),
                command(0, CommandOpcode::Stop),
            ],
            SimTime::ZERO,
        );
        assert_ne!(
            read_word(&mut device, Esp32s3I2c::INT_RAW, SimTime::ZERO) & (1 << 10),
            0
        );
        write_word(&mut device, Esp32s3I2c::INT_CLEAR, u32::MAX, SimTime::ZERO);
        assert_eq!(
            read_word(&mut device, Esp32s3I2c::INT_RAW, SimTime::ZERO) & (1 << 10),
            0
        );
    }
}
