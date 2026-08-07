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
    targets: Arc<Mutex<Esp32s3I2cTargets>>,
    sda: SignalId,
    scl: SignalId,
    hub: SignalHub,
}

#[derive(Clone, Debug)]
enum Esp32s3I2cTarget {
    Sgp30(Sgp30),
    M5Pm1(M5Pm1),
    Bmi270(Bmi270),
    Es8311(Es8311),
}

struct Esp32s3I2cTargets {
    devices: BTreeMap<u8, Esp32s3I2cTarget>,
    m5pm1_signals: Option<M5Pm1Signals>,
    m5pm1_irq_gpio: Option<(GpioHandle, u8)>,
    bmi270_transactions: Option<SignalId>,
    es8311_transactions: Option<SignalId>,
}

#[derive(Clone, Copy, Debug)]
struct M5Pm1Signals {
    transactions: SignalId,
    power_config: SignalId,
    grove_powered: SignalId,
}

/// Host-side attachment and inspection handle for an ESP32-S3 I2C bus.
#[derive(Clone)]
pub struct Esp32s3I2cHandle {
    targets: Arc<Mutex<Esp32s3I2cTargets>>,
    hub: SignalHub,
}

impl Esp32s3I2cHandle {
    /// Attaches the `M5StickS3` power-management companion at address `0x6e`.
    pub fn attach_m5pm1(&self) -> Result<(), SignalError> {
        let mut targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
        targets
            .devices
            .insert(M5PM1_ADDRESS, Esp32s3I2cTarget::M5Pm1(M5Pm1::new()));
        if targets.m5pm1_signals.is_none() {
            targets.m5pm1_signals = Some(M5Pm1Signals {
                transactions: self.hub.declare(
                    "board.m5sticks3.component.m5pm1.transactions",
                    SignalValue::from_u64(0, 64)?,
                    Some("accepted M5PM1 I2C transactions".to_owned()),
                )?,
                power_config: self.hub.declare(
                    "board.m5sticks3.component.m5pm1.power_config",
                    SignalValue::from_u64(0x07, 8)?,
                    Some("M5PM1 power-rail configuration".to_owned()),
                )?,
                grove_powered: self.hub.declare(
                    "board.m5sticks3.component.m5pm1.grove_powered",
                    SignalValue::from_u64(0, 1)?,
                    Some("M5PM1 Grove/BOOST rail state".to_owned()),
                )?,
            });
        }
        Ok(())
    }

    /// Returns the attached M5PM1 state, if present.
    pub fn m5pm1_snapshot(&self) -> Option<M5Pm1Snapshot> {
        let targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
        match targets.devices.get(&M5PM1_ADDRESS) {
            Some(Esp32s3I2cTarget::M5Pm1(device)) => Some(device.snapshot()),
            _ => None,
        }
    }

    /// Connects the M5PM1 active-low IRQ output to one ESP32-S3 GPIO input.
    pub fn bind_m5pm1_irq(
        &self,
        gpio: GpioHandle,
        pin: u8,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        {
            let mut targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
            targets.m5pm1_irq_gpio = Some((gpio, pin));
        }
        self.refresh_m5pm1_signals(at)
    }

    /// Attaches the `M5StickS3` BMI270 inertial sensor at address `0x68`.
    pub fn attach_bmi270(&self) -> Result<(), SignalError> {
        let mut targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
        targets
            .devices
            .insert(BMI270_ADDRESS, Esp32s3I2cTarget::Bmi270(Bmi270::new()));
        if targets.bmi270_transactions.is_none() {
            targets.bmi270_transactions = Some(self.hub.declare(
                "board.m5sticks3.component.bmi270.transactions",
                SignalValue::from_u64(0, 64)?,
                Some("accepted BMI270 I2C transactions".to_owned()),
            )?);
        }
        Ok(())
    }

    /// Attaches the `M5StickS3` ES8311 audio codec at address `0x18`.
    pub fn attach_es8311(&self) -> Result<(), SignalError> {
        let mut targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
        targets
            .devices
            .insert(ES8311_ADDRESS, Esp32s3I2cTarget::Es8311(Es8311::new()));
        if targets.es8311_transactions.is_none() {
            targets.es8311_transactions = Some(self.hub.declare(
                "board.m5sticks3.component.es8311.transactions",
                SignalValue::from_u64(0, 64)?,
                Some("accepted ES8311 I2C transactions".to_owned()),
            )?);
        }
        Ok(())
    }

    /// Supplies a deterministic physical IMU sample.
    pub fn set_bmi270_sample(
        &self,
        accel: [i16; 3],
        gyro: [i16; 3],
        temperature: i16,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        {
            let mut targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
            if let Some(Esp32s3I2cTarget::Bmi270(device)) = targets.devices.get_mut(&BMI270_ADDRESS)
            {
                device.set_sample(accel, gyro, temperature);
            }
            if let Some(Esp32s3I2cTarget::M5Pm1(power)) = targets.devices.get_mut(&M5PM1_ADDRESS) {
                power
                    .set_gpio_input(4, false)
                    .expect("M5PM1 GPIO4 is a valid IMU interrupt input");
                power
                    .set_gpio_input(4, true)
                    .expect("M5PM1 GPIO4 is a valid IMU interrupt input");
            }
        }
        self.refresh_m5pm1_signals(at)
    }

    /// Returns the attached BMI270 state, if present.
    pub fn bmi270_snapshot(&self) -> Option<Bmi270Snapshot> {
        let targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
        match targets.devices.get(&BMI270_ADDRESS) {
            Some(Esp32s3I2cTarget::Bmi270(device)) => Some(device.snapshot()),
            _ => None,
        }
    }

    /// Returns the attached ES8311 state, if present.
    pub fn es8311_snapshot(&self) -> Option<Es8311Snapshot> {
        let targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
        match targets.devices.get(&ES8311_ADDRESS) {
            Some(Esp32s3I2cTarget::Es8311(device)) => Some(device.snapshot()),
            _ => None,
        }
    }

    fn refresh_m5pm1_signals(&self, at: SimTime) -> Result<(), DeviceError> {
        let targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
        let Some(signals) = targets.m5pm1_signals else {
            return Ok(());
        };
        let Some(Esp32s3I2cTarget::M5Pm1(device)) = targets.devices.get(&M5PM1_ADDRESS) else {
            return Ok(());
        };
        let snapshot = device.snapshot();
        let irq_gpio = targets.m5pm1_irq_gpio.clone();
        drop(targets);
        for (signal, value, width) in [
            (signals.transactions, snapshot.transactions, 64),
            (signals.power_config, u64::from(snapshot.power_config), 8),
            (
                signals.grove_powered,
                u64::from(snapshot.power_config & 0x08 != 0),
                1,
            ),
        ] {
            self.hub
                .set(
                    signal,
                    SignalValue::from_u64(value, width).expect("fixed-width M5PM1 signal"),
                    at,
                )
                .map_err(|error| DeviceError::new(error.to_string()))?;
        }
        if let Some((gpio, pin)) = irq_gpio {
            gpio.set_input(
                pin,
                if snapshot.irq_asserted {
                    Logic::Zero
                } else {
                    Logic::One
                },
                at,
            )?;
        }
        Ok(())
    }

    fn refresh_component_signals(&self, at: SimTime) -> Result<(), DeviceError> {
        let targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
        let values = [
            (
                targets.bmi270_transactions,
                targets
                    .devices
                    .get(&BMI270_ADDRESS)
                    .and_then(|target| match target {
                        Esp32s3I2cTarget::Bmi270(device) => Some(device.snapshot().transactions),
                        _ => None,
                    }),
            ),
            (
                targets.es8311_transactions,
                targets
                    .devices
                    .get(&ES8311_ADDRESS)
                    .and_then(|target| match target {
                        Esp32s3I2cTarget::Es8311(device) => Some(device.snapshot().transactions),
                        _ => None,
                    }),
            ),
        ];
        for (signal, value) in values {
            if let (Some(signal), Some(value)) = (signal, value) {
                self.hub
                    .set(
                        signal,
                        SignalValue::from_u64(value, 64).expect("64-bit transaction count"),
                        at,
                    )
                    .map_err(|error| DeviceError::new(error.to_string()))?;
            }
        }
        Ok(())
    }
}

/// Named ESP32-S3 I2C register offsets exposed by the functional model.
///
/// The enum keeps register identities explicit for bus observers and tests;
/// [`Self::offset`] converts one identity to the byte offset used by the
/// memory-mapped [`Device`] contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum Esp32s3I2cRegister {
    /// SCL low-period configuration.
    SclLowPeriod = 0x00,
    /// Controller control and transaction-start register.
    Ctr = 0x04,
    /// Read-only controller status.
    Sr = 0x08,
    /// Receive timeout configuration.
    Timeout = 0x0c,
    /// Local slave address configuration.
    SlaveAddr = 0x10,
    /// FIFO pointer status.
    FifoStatus = 0x14,
    /// FIFO configuration.
    FifoConf = 0x18,
    /// FIFO data register.
    Data = 0x1c,
    /// Raw interrupt status.
    IntRaw = 0x20,
    /// Write-one-to-clear interrupt register.
    IntClear = 0x24,
    /// Interrupt enable mask.
    IntEnable = 0x28,
    /// Masked interrupt status.
    IntStatus = 0x2c,
    /// Command slot zero.
    Command0 = 0x58,
    /// Command slot one.
    Command1 = 0x5c,
    /// Command slot two.
    Command2 = 0x60,
    /// Command slot three.
    Command3 = 0x64,
    /// Command slot four.
    Command4 = 0x68,
    /// Command slot five.
    Command5 = 0x6c,
    /// Command slot six.
    Command6 = 0x70,
    /// Command slot seven.
    Command7 = 0x74,
    /// Peripheral date/version register.
    Date = 0xf8,
}

impl Esp32s3I2cRegister {
    /// Returns the byte offset for this register identity.
    pub const fn offset(self) -> u64 {
        self as u64
    }
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
    pub const SCL_LOW_PERIOD: u64 = Esp32s3I2cRegister::SclLowPeriod.offset();
    /// Controller control register offset.
    pub const CTR: u64 = Esp32s3I2cRegister::Ctr.offset();
    /// Controller status register offset.
    pub const SR: u64 = Esp32s3I2cRegister::Sr.offset();
    /// Timeout register offset.
    pub const TIMEOUT: u64 = Esp32s3I2cRegister::Timeout.offset();
    /// Local slave-address register offset.
    pub const SLAVE_ADDR: u64 = Esp32s3I2cRegister::SlaveAddr.offset();
    /// FIFO pointer status register offset.
    pub const FIFO_STATUS: u64 = Esp32s3I2cRegister::FifoStatus.offset();
    /// FIFO configuration register offset.
    pub const FIFO_CONF: u64 = Esp32s3I2cRegister::FifoConf.offset();
    /// FIFO data register offset.
    pub const DATA: u64 = Esp32s3I2cRegister::Data.offset();
    /// Raw interrupt status register offset.
    pub const INT_RAW: u64 = Esp32s3I2cRegister::IntRaw.offset();
    /// Write-to-clear interrupt register offset.
    pub const INT_CLEAR: u64 = Esp32s3I2cRegister::IntClear.offset();
    /// Interrupt-enable register offset.
    pub const INT_ENABLE: u64 = Esp32s3I2cRegister::IntEnable.offset();
    /// Masked interrupt status register offset.
    pub const INT_STATUS: u64 = Esp32s3I2cRegister::IntStatus.offset();
    /// First command register offset.
    pub const COMMAND0: u64 = Esp32s3I2cRegister::Command0.offset();
    /// Date/version register offset.
    pub const DATE: u64 = Esp32s3I2cRegister::Date.offset();

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
    const BYTE_NUM_MASK: u32 = 0xff;
    const ACK_CHECK_EN: u32 = 1 << 8;
    const ACK_EXPECTED: u32 = 1 << 9;
    const ACK_VALUE: u32 = 1 << 10;
    const FIFO_CAPACITY: usize = 32;
    const WAVEFORM_HALF_TICKS: u64 = 1;

    const fn command_shape_is_valid(word: u32, opcode: CommandOpcode) -> bool {
        match opcode {
            CommandOpcode::Restart | CommandOpcode::Stop | CommandOpcode::End => {
                word & (Self::BYTE_NUM_MASK
                    | Self::ACK_CHECK_EN
                    | Self::ACK_EXPECTED
                    | Self::ACK_VALUE)
                    == 0
            }
            CommandOpcode::Write | CommandOpcode::Read => true,
        }
    }

    /// Creates an I2C controller with a deterministic SGP30 at address `0x58`.
    pub fn new(name: impl Into<String>, hub: SignalHub) -> Result<Self, SignalError> {
        Self::new_with_handle(name, hub).map(|(device, _)| device)
    }

    /// Creates an I2C controller and a handle for attaching board devices.
    pub fn new_with_handle(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Esp32s3I2cHandle), SignalError> {
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
        let targets = Arc::new(Mutex::new(Esp32s3I2cTargets {
            devices: BTreeMap::from([(SGP30_ADDRESS, Esp32s3I2cTarget::Sgp30(Sgp30::new(420, 8)))]),
            m5pm1_signals: None,
            m5pm1_irq_gpio: None,
            bmi270_transactions: None,
            es8311_transactions: None,
        }));
        let handle = Esp32s3I2cHandle {
            targets: targets.clone(),
            hub: hub.clone(),
        };
        let mut device = Self {
            name,
            registers: [0; 0x100 / 4],
            commands: [0; 8],
            tx_fifo: VecDeque::with_capacity(Self::FIFO_CAPACITY),
            rx_fifo: VecDeque::with_capacity(Self::FIFO_CAPACITY),
            int_raw: 1 << 1,
            int_ena: 0,
            nack: false,
            targets,
            sda,
            scl,
            hub,
        };
        device.reset_registers();
        Ok((device, handle))
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
            let byte_count = (word & Self::BYTE_NUM_MASK) as usize;
            let opcode = CommandOpcode::try_from((word >> 11) & 0x7);
            if let Ok(opcode) = opcode
                && !Self::command_shape_is_valid(word, opcode)
            {
                self.nack = true;
                break;
            }
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
                    // A modeled SGP30 acknowledges accepted bytes with zero.
                    // If firmware explicitly asks the controller to check for
                    // a NACK, the command therefore fails deterministically.
                    if word & Self::ACK_CHECK_EN != 0 && word & Self::ACK_EXPECTED != 0 {
                        self.nack = true;
                        break;
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
            let response = address
                .and_then(|address| {
                    self.targets
                        .lock()
                        .expect("ESP32-S3 I2C lock poisoned")
                        .devices
                        .get_mut(&address)
                        .map(|target| match target {
                            Esp32s3I2cTarget::Sgp30(device) => device
                                .transact(&write_payload, read_len, at)
                                .map_err(|error| DeviceError::new(error.to_string())),
                            Esp32s3I2cTarget::M5Pm1(device) => device
                                .transact(&write_payload, read_len, at)
                                .map_err(|error| DeviceError::new(error.to_string())),
                            Esp32s3I2cTarget::Bmi270(device) => device
                                .transact(&write_payload, read_len, at)
                                .map_err(|error| DeviceError::new(error.to_string())),
                            Esp32s3I2cTarget::Es8311(device) => device
                                .transact(&write_payload, read_len, at)
                                .map_err(|error| DeviceError::new(error.to_string())),
                        })
                })
                .transpose()?;
            match response {
                Some(response) if response.len() <= Self::FIFO_CAPACITY => {
                    self.rx_fifo.extend(response);
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
        let response = self.rx_fifo.iter().copied().collect::<Vec<_>>();
        self.emit_waveform(&tx, &response, at)?;
        Esp32s3I2cHandle {
            targets: self.targets.clone(),
            hub: self.hub.clone(),
        }
        .refresh_m5pm1_signals(at)?;
        Esp32s3I2cHandle {
            targets: self.targets.clone(),
            hub: self.hub.clone(),
        }
        .refresh_component_signals(at)?;
        Ok(())
    }

    fn emit_waveform(&self, tx: &[u8], rx: &[u8], at: SimTime) -> Result<(), DeviceError> {
        let mut now = at.ticks();
        let mut tx_index = 0;
        let mut rx_index = 0;
        let mut started = false;
        for command in self.commands {
            let word = command & Self::COMMAND_MASK;
            let byte_count = (word & Self::BYTE_NUM_MASK) as usize;
            let opcode = CommandOpcode::try_from((word >> 11) & 0x7);
            if let Ok(opcode) = opcode
                && !Self::command_shape_is_valid(word, opcode)
            {
                break;
            }
            match opcode {
                Ok(CommandOpcode::Restart) => {
                    self.emit_start(&mut now)?;
                    started = true;
                }
                Ok(CommandOpcode::Write) if started => {
                    for _ in 0..byte_count {
                        let byte = tx.get(tx_index).copied().unwrap_or(0xff);
                        tx_index = tx_index.saturating_add(1);
                        self.emit_write_byte(byte, &mut now)?;
                    }
                }
                Ok(CommandOpcode::Read) if started => {
                    let explicit_nack = word & Self::ACK_VALUE != 0;
                    for index in 0..byte_count {
                        let byte = rx.get(rx_index).copied().unwrap_or(0xff);
                        rx_index = rx_index.saturating_add(1);
                        let last = rx_index >= rx.len();
                        let last_in_command = index + 1 == byte_count;
                        let ack = if last_in_command && explicit_nack {
                            false
                        } else {
                            !last
                        };
                        self.emit_read_byte(byte, ack, &mut now)?;
                    }
                }
                Ok(CommandOpcode::Stop) if started => {
                    self.emit_stop(&mut now)?;
                    started = false;
                }
                Ok(CommandOpcode::End) => break,
                Ok(CommandOpcode::Write | CommandOpcode::Read | CommandOpcode::Stop) => {}
                Err(()) => break,
            }
        }
        if started {
            self.emit_stop(&mut now)?;
        }
        Ok(())
    }

    fn set_sda(&self, value: Logic, at: u64) -> Result<(), DeviceError> {
        self.hub
            .set(
                self.sda,
                SignalValue::repeat(value, 1).expect("one-bit signal"),
                SimTime::from_ticks(at),
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn set_scl(&self, value: Logic, at: u64) -> Result<(), DeviceError> {
        self.hub
            .set(
                self.scl,
                SignalValue::repeat(value, 1).expect("one-bit signal"),
                SimTime::from_ticks(at),
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn emit_start(&self, now: &mut u64) -> Result<(), DeviceError> {
        self.set_scl(Logic::One, *now)?;
        self.set_sda(Logic::One, *now)?;
        self.set_sda(Logic::Zero, *now)?;
        *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
        self.set_scl(Logic::Zero, *now)
    }

    fn emit_stop(&self, now: &mut u64) -> Result<(), DeviceError> {
        self.set_scl(Logic::Zero, *now)?;
        self.set_sda(Logic::Zero, *now)?;
        *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
        self.set_scl(Logic::One, *now)?;
        *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
        self.set_sda(Logic::One, *now)?;
        *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
        Ok(())
    }

    fn emit_write_byte(&self, byte: u8, now: &mut u64) -> Result<(), DeviceError> {
        for bit in (0..8).rev() {
            self.set_scl(Logic::Zero, *now)?;
            self.set_sda(
                if byte & (1 << bit) == 0 {
                    Logic::Zero
                } else {
                    Logic::One
                },
                *now,
            )?;
            *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
            self.set_scl(Logic::One, *now)?;
            *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
        }
        // The modeled SGP30 acknowledges every accepted write byte.
        self.set_scl(Logic::Zero, *now)?;
        self.set_sda(Logic::Zero, *now)?;
        *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
        self.set_scl(Logic::One, *now)?;
        *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
        Ok(())
    }

    fn emit_read_byte(&self, byte: u8, ack: bool, now: &mut u64) -> Result<(), DeviceError> {
        for bit in (0..8).rev() {
            self.set_scl(Logic::Zero, *now)?;
            self.set_sda(
                if byte & (1 << bit) == 0 {
                    Logic::Zero
                } else {
                    Logic::One
                },
                *now,
            )?;
            *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
            self.set_scl(Logic::One, *now)?;
            *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
        }
        self.set_scl(Logic::Zero, *now)?;
        self.set_sda(if ack { Logic::Zero } else { Logic::One }, *now)?;
        *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
        self.set_scl(Logic::One, *now)?;
        *now = now.saturating_add(Self::WAVEFORM_HALF_TICKS);
        Ok(())
    }

    /// Returns the deterministic sensor state for host-side qualification.
    pub fn sensor_snapshot(&self) -> Sgp30Snapshot {
        let targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
        match targets.devices.get(&SGP30_ADDRESS) {
            Some(Esp32s3I2cTarget::Sgp30(sensor)) => sensor.snapshot(),
            _ => Sgp30::new(420, 8).snapshot(),
        }
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
        let mut targets = self.targets.lock().expect("ESP32-S3 I2C lock poisoned");
        for target in targets.devices.values_mut() {
            *target = match target {
                Esp32s3I2cTarget::Sgp30(_) => Esp32s3I2cTarget::Sgp30(Sgp30::new(420, 8)),
                Esp32s3I2cTarget::M5Pm1(_) => Esp32s3I2cTarget::M5Pm1(M5Pm1::new()),
                Esp32s3I2cTarget::Bmi270(_) => Esp32s3I2cTarget::Bmi270(Bmi270::new()),
                Esp32s3I2cTarget::Es8311(_) => Esp32s3I2cTarget::Es8311(Es8311::new()),
            };
        }
        drop(targets);
        // Reset releases both open-drain lines to the idle high state. Signal
        // identifiers are owned by this device, so a reset cannot fail unless
        // the hub itself has been corrupted.
        let _ = self.set_sda(Logic::One, SimTime::ZERO.ticks());
        let _ = self.set_scl(Logic::One, SimTime::ZERO.ticks());
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
        let hub = SignalHub::new();
        let mut device = Esp32s3I2c::new("esp32s3.i2c0", hub.clone()).unwrap();
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
        let data = hub
            .with_registry(|registry| registry.find("board.esp32s3.i2c0.sda"))
            .unwrap();
        let clock = hub
            .with_registry(|registry| registry.find("board.esp32s3.i2c0.scl"))
            .unwrap();
        let mut clock_level = Logic::One;
        let mut starts = 0;
        let changes = hub.drain_changes();
        for pair in changes.windows(2) {
            assert!(pair[0].at <= pair[1].at);
        }
        for change in changes {
            if change.signal == clock {
                clock_level = change.value.bit(0).unwrap();
            } else if change.signal == data
                && change.value.bit(0) == Some(Logic::Zero)
                && clock_level == Logic::One
            {
                starts += 1;
            }
        }
        // The write-only initialization has one start; the read transaction
        // has a start and a repeated start before its read address.
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
    fn routes_i2c1_transactions_to_an_attached_m5pm1() {
        let hub = SignalHub::new();
        let (mut device, handle) =
            Esp32s3I2c::new_with_handle("esp32s3.i2c1", hub.clone()).unwrap();
        let (_gpio_device, gpio) =
            FunctionalGpio::new("gpio", 16, "board.gpio", hub, 0, 4, 8).unwrap();
        handle.attach_m5pm1().unwrap();
        handle
            .bind_m5pm1_irq(gpio.clone(), 13, SimTime::ZERO)
            .unwrap();
        program(
            &mut device,
            &[M5PM1_ADDRESS << 1, 0x06, 0x0f],
            &[
                command(0, CommandOpcode::Restart),
                command(3, CommandOpcode::Write),
                command(0, CommandOpcode::Stop),
                command(0, CommandOpcode::End),
            ],
            SimTime::from_ticks(10),
        );
        let snapshot = handle.m5pm1_snapshot().unwrap();
        assert_eq!(snapshot.transactions, 1);
        assert_eq!(snapshot.power_config, 0x0f);
        assert_eq!(gpio.resolved(13).unwrap(), Logic::One);

        for bytes in [
            [M5PM1_ADDRESS << 1, 0x17, 0x01], // GPIO4 interrupt function.
            [M5PM1_ADDRESS << 1, 0x19, 0x10], // GPIO4 rising edge.
            [M5PM1_ADDRESS << 1, 0x43, 0x0f], // Unmask GPIO4.
        ] {
            program(
                &mut device,
                &bytes,
                &[
                    command(0, CommandOpcode::Restart),
                    command(3, CommandOpcode::Write),
                    command(0, CommandOpcode::Stop),
                    command(0, CommandOpcode::End),
                ],
                SimTime::from_ticks(20),
            );
        }
        handle.attach_bmi270().unwrap();
        handle
            .set_bmi270_sample([1, 2, 3], [4, 5, 6], 7, SimTime::from_ticks(30))
            .unwrap();
        assert!(handle.m5pm1_snapshot().unwrap().irq_asserted);
        assert_eq!(gpio.resolved(13).unwrap(), Logic::Zero);
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

    #[test]
    fn rejects_reserved_opcode_and_unexpected_ack() {
        let mut reserved = Esp32s3I2c::new("esp32s3.i2c0", SignalHub::new()).unwrap();
        program(
            &mut reserved,
            &[0xb0],
            &[1 | (5 << 11), command(0, CommandOpcode::End)],
            SimTime::ZERO,
        );
        assert_ne!(
            read_word(&mut reserved, Esp32s3I2c::INT_RAW, SimTime::ZERO) & (1 << 10),
            0
        );

        let mut malformed_end = Esp32s3I2c::new("esp32s3.i2c2", SignalHub::new()).unwrap();
        program(
            &mut malformed_end,
            &[],
            &[command(1, CommandOpcode::End)],
            SimTime::ZERO,
        );
        assert_ne!(
            read_word(&mut malformed_end, Esp32s3I2c::INT_RAW, SimTime::ZERO) & (1 << 10),
            0
        );

        let mut unexpected_ack = Esp32s3I2c::new("esp32s3.i2c1", SignalHub::new()).unwrap();
        program(
            &mut unexpected_ack,
            &[0xb0],
            &[
                command(1, CommandOpcode::Write)
                    | Esp32s3I2c::ACK_CHECK_EN
                    | Esp32s3I2c::ACK_EXPECTED,
                command(0, CommandOpcode::End),
            ],
            SimTime::ZERO,
        );
        assert_ne!(
            read_word(&mut unexpected_ack, Esp32s3I2c::INT_RAW, SimTime::ZERO) & (1 << 10),
            0
        );
    }

    #[test]
    fn register_enum_keeps_offsets_named() {
        assert_eq!(Esp32s3I2cRegister::Data.offset(), Esp32s3I2c::DATA);
        assert_eq!(Esp32s3I2cRegister::Command7.offset(), 0x74);
        assert_eq!(Esp32s3I2cRegister::Date.offset(), Esp32s3I2c::DATE);
    }
}
