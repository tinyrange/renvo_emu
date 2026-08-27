use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalError, SignalId, SignalValue};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

fn width_bytes(width: AccessWidth) -> usize {
    usize::from(width.bytes())
}

fn read_le(bytes: &[u8], offset: u64, width: AccessWidth) -> Result<u64, DeviceError> {
    let start =
        usize::try_from(offset).map_err(|_| DeviceError::new("register offset overflow"))?;
    let end = start
        .checked_add(width_bytes(width))
        .ok_or_else(|| DeviceError::new("register access overflow"))?;
    let slice = bytes.get(start..end).ok_or_else(|| {
        DeviceError::new(format!("unmodeled register read at offset {offset:#x}"))
    })?;
    Ok(slice
        .iter()
        .enumerate()
        .fold(0_u64, |value, (shift, byte)| {
            value | (u64::from(*byte) << (shift * 8))
        }))
}

fn write_le(
    bytes: &mut [u8],
    offset: u64,
    width: AccessWidth,
    value: u64,
) -> Result<(), DeviceError> {
    let start =
        usize::try_from(offset).map_err(|_| DeviceError::new("register offset overflow"))?;
    let end = start
        .checked_add(width_bytes(width))
        .ok_or_else(|| DeviceError::new("register access overflow"))?;
    let slice = bytes.get_mut(start..end).ok_or_else(|| {
        DeviceError::new(format!("unmodeled register write at offset {offset:#x}"))
    })?;
    for (shift, byte) in slice.iter_mut().enumerate() {
        *byte = (value >> (shift * 8)) as u8;
    }
    Ok(())
}

fn narrow_u32(value: u32, width: AccessWidth) -> u64 {
    match width {
        AccessWidth::Byte => u64::from(value & 0xff),
        AccessWidth::HalfWord => u64::from(value & 0xffff),
        AccessWidth::Word | AccessWidth::DoubleWord => u64::from(value),
    }
}

/// Byte-addressable SAM D21 startup register block with deterministic reset bytes.
pub struct Samd21RegisterBlock {
    name: String,
    reset: Vec<u8>,
    bytes: Vec<u8>,
}

impl Samd21RegisterBlock {
    /// Creates a register block and overlays the supplied reset bytes.
    pub fn new(
        name: impl Into<String>,
        size: usize,
        reset: impl IntoIterator<Item = (usize, u8)>,
    ) -> Self {
        let mut bytes = vec![0; size];
        for (offset, value) in reset {
            if let Some(byte) = bytes.get_mut(offset) {
                *byte = value;
            }
        }
        Self {
            name: name.into(),
            reset: bytes.clone(),
            bytes,
        }
    }
}

impl Device for Samd21RegisterBlock {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        read_le(&self.bytes, offset, width)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        write_le(&mut self.bytes, offset, width, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.bytes.clone_from(&self.reset);
    }
}

/// SAM D21 PORT group A with atomic direction/output aliases and pin configuration bytes.
pub struct Samd21Port {
    name: String,
    pins: u8,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
    control: u32,
    write_config: u32,
    pmux: [u8; 16],
    pin_config: [u8; 32],
}

impl Samd21Port {
    /// Constructs PORT group A and its external pin handle.
    pub fn new(
        name: impl Into<String>,
        pins: u8,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), remu_signals::SignalError> {
        let (state, signals, handle) = vendor_gpio(pins, path, &hub)?;
        Ok((
            Self {
                name: name.into(),
                pins,
                state,
                signals,
                hub,
                control: 0,
                write_config: 0,
                pmux: [0; 16],
                pin_config: [0; 32],
            },
            handle,
        ))
    }

    fn mask(&self) -> u32 {
        if self.pins == 32 {
            u32::MAX
        } else {
            (1_u32 << self.pins) - 1
        }
    }

    fn input(&self) -> u32 {
        self.state
            .lock()
            .expect("GPIO lock poisoned")
            .nets
            .iter()
            .take(usize::from(self.pins))
            .enumerate()
            .fold(0_u32, |value, (pin, net)| {
                value | (u32::from(net.resolved() == Logic::One) << pin)
            })
    }

    fn update_latch(&mut self, offset: u64, value: u32) -> Result<(), DeviceError> {
        let mask = self.mask();
        let mut state = self.state.lock().expect("GPIO lock poisoned");
        match offset {
            0x00 => state.direction = value & mask,
            0x04 => state.direction &= !value,
            0x08 => state.direction |= value & mask,
            0x0c => state.direction ^= value & mask,
            0x10 => state.output = value & mask,
            0x14 => state.output &= !value,
            0x18 => state.output |= value & mask,
            0x1c => state.output ^= value & mask,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled PORT write at {offset:#x}"
                )));
            }
        }
        drop(state);
        Ok(())
    }
}

impl Device for Samd21Port {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if (0x30..0x40).contains(&offset) {
            return read_le(&self.pmux, offset - 0x30, width);
        }
        if (0x40..0x60).contains(&offset) {
            return read_le(&self.pin_config, offset - 0x40, width);
        }
        if width != AccessWidth::Word {
            return Err(DeviceError::new("PORT latch registers require word access"));
        }
        let value = match offset {
            0x00 | 0x04 | 0x08 | 0x0c => self.state.lock().expect("GPIO lock poisoned").direction,
            0x10 | 0x14 | 0x18 | 0x1c => self.state.lock().expect("GPIO lock poisoned").output,
            0x20 => self.input(),
            0x24 => self.control,
            0x28 => self.write_config,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled PORT read at {offset:#x}"
                )));
            }
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if (0x30..0x40).contains(&offset) {
            return write_le(&mut self.pmux, offset - 0x30, width, value);
        }
        if (0x40..0x60).contains(&offset) {
            return write_le(&mut self.pin_config, offset - 0x40, width, value);
        }
        if width != AccessWidth::Word {
            return Err(DeviceError::new("PORT latch registers require word access"));
        }
        match offset {
            0x00..=0x1c => self.update_latch(offset, value as u32)?,
            0x24 => self.control = value as u32,
            0x28 => self.write_config = value as u32,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled PORT write at {offset:#x}"
                )));
            }
        }
        refresh_gpio(&self.state, &self.signals, &self.hub, self.pins, at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        {
            let mut state = self.state.lock().expect("GPIO lock poisoned");
            state.direction = 0;
            state.output = 0;
        }
        self.control = 0;
        self.write_config = 0;
        self.pmux = [0; 16];
        self.pin_config = [0; 32];
    }
}

#[derive(Default)]
struct TcState {
    enabled: bool,
    interrupt_enabled: bool,
    interrupt_pending: bool,
    matched: bool,
    start: u64,
    compare: u16,
}

/// Machine-facing handle for a SAM D21 COUNT16 TC instance.
#[derive(Clone)]
pub struct Samd21TcHandle(Arc<Mutex<TcState>>);

impl Samd21TcHandle {
    /// Advances the functional counter and returns the match-interrupt level.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("TC lock poisoned");
        if state.enabled
            && !state.matched
            && state.compare != 0
            && now.ticks().saturating_sub(state.start) >= u64::from(state.compare)
        {
            state.interrupt_pending = true;
            state.matched = true;
        }
        state.interrupt_pending && state.interrupt_enabled
    }
}

/// Functional SAM D21 COUNT16 TC register slice.
pub struct Samd21Tc {
    name: String,
    state: Arc<Mutex<TcState>>,
}

impl Samd21Tc {
    /// Constructs a TC instance and its interrupt handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21TcHandle) {
        let state = Arc::new(Mutex::new(TcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Samd21TcHandle(state),
        )
    }
}

impl Device for Samd21Tc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, _width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("TC lock poisoned");
        match offset {
            0x00 => Ok(u64::from(state.enabled) << 1),
            0x0c | 0x0d => Ok(u64::from(state.interrupt_enabled)),
            0x0e => Ok(u64::from(state.interrupt_pending) << 4),
            0x0f => Ok(0),
            0x10 => Ok((at.ticks().saturating_sub(state.start) & 0xffff) as u64),
            0x18 => Ok(u64::from(state.compare)),
            _ => Ok(0),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("TC lock poisoned");
        match offset {
            0x00 => {
                state.enabled = value & 2 != 0;
                state.start = at.ticks();
                state.matched = false;
            }
            0x0c => state.interrupt_enabled &= value & 0x10 == 0,
            0x0d => state.interrupt_enabled |= value & 0x10 != 0,
            0x0e => {
                if value & 0x10 != 0 {
                    state.interrupt_pending = false;
                }
            }
            0x10 => {
                state.start = at.ticks().saturating_sub(value & 0xffff);
                state.matched = false;
            }
            0x18 => {
                state.compare = value as u16;
                state.matched = false;
            }
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("TC lock poisoned") = TcState::default();
    }
}

/// Operating mode selected by the SERCOM `CTRLA.MODE` field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Samd21SercomMode {
    /// USART mode with an external or internal clock (MODE 0 or 1).
    #[default]
    Usart,
    /// SPI host/master mode (MODE 3).
    SpiMaster,
    /// SPI client/slave mode (MODE 2).
    SpiSlave,
    /// I²C host/master mode (MODE 5).
    I2cMaster,
    /// I²C client/slave mode (MODE 4).
    I2cSlave,
    /// A reserved or unsupported SERCOM mode value.
    Other(u8),
}

impl Samd21SercomMode {
    fn from_ctrla(value: u32) -> Self {
        match ((value >> 2) & 0x7) as u8 {
            0 | 1 => Self::Usart,
            2 => Self::SpiSlave,
            3 => Self::SpiMaster,
            4 => Self::I2cSlave,
            5 => Self::I2cMaster,
            mode => Self::Other(mode),
        }
    }

    fn ctrla_mask(self) -> u32 {
        match self {
            Self::Usart => 0x7ff3_e19f,
            Self::SpiMaster | Self::SpiSlave => 0x7f33_019f,
            Self::I2cMaster => 0x7bf1_009f,
            Self::I2cSlave => 0x4bb1_009f,
            Self::Other(_) => 0x0000_001f,
        }
    }

    fn ctrlb_mask(self) -> u32 {
        match self {
            Self::Usart => 0x0003_2747,
            Self::SpiMaster | Self::SpiSlave => 0x0002_e247,
            Self::I2cMaster => 0x0007_0300,
            Self::I2cSlave => 0x0007_c700,
            Self::Other(_) => 0,
        }
    }

    fn baud_mask(self) -> u32 {
        match self {
            Self::Usart => 0x0000_ffff,
            Self::SpiMaster | Self::SpiSlave => 0x0000_00ff,
            Self::I2cMaster => u32::MAX,
            Self::I2cSlave | Self::Other(_) => 0,
        }
    }

    fn command_mask(self) -> u32 {
        if matches!(self, Self::I2cMaster | Self::I2cSlave) {
            0x0003_0000
        } else {
            0
        }
    }

    fn interrupt_mask(self) -> u8 {
        match self {
            Self::Usart => 0xbf,
            Self::SpiMaster | Self::SpiSlave => 0x8f,
            Self::I2cMaster => 0x83,
            Self::I2cSlave => 0x87,
            Self::Other(_) => 0,
        }
    }

    fn status_mask(self) -> u16 {
        match self {
            Self::Usart => 0x003f,
            Self::SpiMaster | Self::SpiSlave => 0x0004,
            Self::I2cMaster => 0x07f7,
            Self::I2cSlave => 0x06df,
            Self::Other(_) => 0,
        }
    }

    fn addr_mask(self) -> u32 {
        match self {
            Self::SpiMaster | Self::SpiSlave => 0x00ff_00ff,
            Self::I2cMaster => 0x00ff_e7ff,
            Self::I2cSlave => 0x07fe_87ff,
            _ => 0,
        }
    }
}

/// Native ATSAMD21 SERCOM register identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Samd21SercomRegister {
    /// Control A.
    Ctrla,
    /// Control B.
    Ctrlb,
    /// Baud rate.
    Baud,
    /// USART receive pulse length.
    RxPulse,
    /// Interrupt enable clear alias.
    Intenclr,
    /// Interrupt enable set alias.
    Intenset,
    /// Interrupt flag status and clear.
    Intflag,
    /// Status.
    Status,
    /// Synchronization busy.
    Syncbusy,
    /// Address.
    Addr,
    /// Data.
    Data,
    /// Debug control.
    Dbgctrl,
}

impl Samd21SercomRegister {
    /// Converts a native SERCOM register offset to its named ID.
    pub const fn from_offset(offset: usize) -> Option<Self> {
        match offset {
            0x00 => Some(Self::Ctrla),
            0x04 => Some(Self::Ctrlb),
            0x0c => Some(Self::Baud),
            0x0e => Some(Self::RxPulse),
            0x14 => Some(Self::Intenclr),
            0x16 => Some(Self::Intenset),
            0x18 => Some(Self::Intflag),
            0x1a => Some(Self::Status),
            0x1c => Some(Self::Syncbusy),
            0x24 => Some(Self::Addr),
            0x28 => Some(Self::Data),
            0x30 => Some(Self::Dbgctrl),
            _ => None,
        }
    }

    /// Returns the native byte offset of this register.
    pub const fn offset(self) -> usize {
        match self {
            Self::Ctrla => 0x00,
            Self::Ctrlb => 0x04,
            Self::Baud => 0x0c,
            Self::RxPulse => 0x0e,
            Self::Intenclr => 0x14,
            Self::Intenset => 0x16,
            Self::Intflag => 0x18,
            Self::Status => 0x1a,
            Self::Syncbusy => 0x1c,
            Self::Addr => 0x24,
            Self::Data => 0x28,
            Self::Dbgctrl => 0x30,
        }
    }
}

const SERCOM_CTRLA_SWRST: u32 = 1;
const SERCOM_CTRLA_ENABLE: u32 = 1 << 1;
const SERCOM_CTRLB_ACKACT: u32 = 1 << 18;
const SERCOM_SPI_CTRLB_RXEN: u32 = 1 << 17;
const SERCOM_I2C_INTFLAG_MB: u8 = 1;
const SERCOM_I2C_INTFLAG_SB: u8 = 1 << 1;
const SERCOM_SPI_INTFLAG_DRE: u8 = 1;
const SERCOM_SPI_INTFLAG_TXC: u8 = 1 << 1;
const SERCOM_SPI_INTFLAG_RXC: u8 = 1 << 2;
const SERCOM_I2C_STATUS_BUSERR: u16 = 1;
const SERCOM_I2C_STATUS_ARBLOST: u16 = 1 << 1;
const SERCOM_I2C_STATUS_BUSSTATE_MASK: u16 = 0x30;
const SERCOM_I2C_STATUS_CLKHOLD: u16 = 1 << 7;
const SERCOM_I2C_BUSSTATE_UNKNOWN: u8 = 0;
const SERCOM_I2C_BUSSTATE_IDLE: u8 = 1;
const SERCOM_I2C_BUSSTATE_OWNER: u8 = 2;

#[derive(Default)]
struct UsartState {
    enabled: bool,
    interrupt_enable: u8,
    interrupt_flags: u8,
    bytes: Vec<u8>,
    mode: Samd21SercomMode,
    spi_rx: VecDeque<u8>,
    spi_injected: VecDeque<u8>,
    spi_tx: Vec<u8>,
    i2c_rx: VecDeque<u8>,
    i2c_tx: Vec<u8>,
    i2c_address: Option<u16>,
    ctrla: u32,
    ctrlb: u32,
    baud: u32,
    rx_pulse: u8,
    status: u16,
    addr: u32,
    dbgctrl: u8,
}

impl UsartState {
    fn flags(&self) -> u8 {
        let mut flags = self.interrupt_flags & self.mode.interrupt_mask();
        match self.mode {
            // The original USART acceptance slice exposes DRE continuously so existing
            // firmware can use its bounded polling loop without a clock model.
            Samd21SercomMode::Usart => flags |= 1,
            Samd21SercomMode::SpiMaster => {
                if self.enabled {
                    flags |= SERCOM_SPI_INTFLAG_DRE;
                }
                if self.ctrlb & SERCOM_SPI_CTRLB_RXEN != 0 && !self.spi_rx.is_empty() {
                    flags |= SERCOM_SPI_INTFLAG_RXC;
                }
            }
            Samd21SercomMode::SpiSlave
            | Samd21SercomMode::I2cMaster
            | Samd21SercomMode::I2cSlave => {}
            Samd21SercomMode::Other(_) => {}
        }
        flags & self.mode.interrupt_mask()
    }

    fn status_value(&self) -> u16 {
        self.status & self.mode.status_mask()
    }

    fn apply_ctrla(&mut self, value: u32) {
        let mode = Samd21SercomMode::from_ctrla(value);
        self.mode = mode;
        self.ctrla = value & mode.ctrla_mask() & !SERCOM_CTRLA_SWRST;
        self.enabled = self.ctrla & SERCOM_CTRLA_ENABLE != 0;
        self.ctrlb &= mode.ctrlb_mask() & !mode.command_mask();
        self.baud &= mode.baud_mask();
        self.addr &= mode.addr_mask();
        self.interrupt_enable &= mode.interrupt_mask();
        self.interrupt_flags = 0;
        self.spi_rx.clear();
        self.spi_injected.clear();
        self.i2c_rx.clear();
        self.i2c_address = None;
        self.status = if matches!(mode, Samd21SercomMode::I2cMaster) && self.enabled {
            SERCOM_I2C_BUSSTATE_UNKNOWN as u16
        } else {
            0
        };
    }

    fn reset_protocol(&mut self) -> u8 {
        let dbgctrl = self.dbgctrl;
        *self = Self::default();
        self.dbgctrl = dbgctrl;
        dbgctrl
    }
}

/// Machine-facing SAM D21 SERCOM USART output and interrupt state.
#[derive(Clone)]
pub struct Samd21UsartHandle(Arc<Mutex<UsartState>>);

impl Samd21UsartHandle {
    /// Bytes transmitted through the USART DATA register.
    pub fn bytes(&self) -> Vec<u8> {
        self.0.lock().expect("USART lock poisoned").bytes.clone()
    }

    /// Returns the currently selected SERCOM protocol mode.
    pub fn mode(&self) -> Samd21SercomMode {
        self.0.lock().expect("USART lock poisoned").mode
    }

    /// Returns bytes transmitted in SPI mode.
    pub fn spi_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("USART lock poisoned").spi_tx.clone()
    }

    /// Queues deterministic bytes for the next SPI master receive operations.
    pub fn queue_spi_rx(&self, bytes: impl IntoIterator<Item = u8>) {
        self.0
            .lock()
            .expect("USART lock poisoned")
            .spi_injected
            .extend(bytes);
    }

    /// Returns bytes transmitted in I²C host mode.
    pub fn i2c_bytes(&self) -> Vec<u8> {
        self.0.lock().expect("USART lock poisoned").i2c_tx.clone()
    }

    /// Queues deterministic bytes for the next I²C master read operations.
    pub fn queue_i2c_rx(&self, bytes: impl IntoIterator<Item = u8>) {
        self.0
            .lock()
            .expect("USART lock poisoned")
            .i2c_rx
            .extend(bytes);
    }

    /// Returns the most recently addressed I²C host address, including its R/W bit.
    pub fn i2c_address(&self) -> Option<u16> {
        self.0.lock().expect("USART lock poisoned").i2c_address
    }

    /// Whether an enabled SERCOM interrupt is pending.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.0.lock().expect("USART lock poisoned");
        state.interrupt_enable & state.flags() != 0
    }
}

/// Functional SAM D21 SERCOM register slice.
pub struct Samd21Usart {
    name: String,
    state: Arc<Mutex<UsartState>>,
    registers: [u8; 0x34],
}

/// Native ATSAMD21 DAC register identifiers.
///
/// Keeping the register map typed makes it harder to accidentally confuse the
/// adjacent interrupt-enable, interrupt-flag, and status offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Samd21DacRegister {
    /// Control A: SWRST, ENABLE, and RUNSTDBY.
    Ctrla = 0x00,
    /// Control B: reference and output selection.
    Ctrlb = 0x01,
    /// Event input/output selection.
    Evctrl = 0x02,
    /// Interrupt-enable clear (write one to clear).
    Intenclr = 0x04,
    /// Interrupt-enable set (write one to set).
    Intenset = 0x05,
    /// Interrupt flags (write one to clear).
    Intflag = 0x06,
    /// Synchronization status.
    Status = 0x07,
    /// Direct 10-bit conversion data.
    Data = 0x08,
    /// Buffered 10-bit conversion data.
    Databuf = 0x0c,
}

impl Samd21DacRegister {
    fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x00 => Some(Self::Ctrla),
            0x01 => Some(Self::Ctrlb),
            0x02 => Some(Self::Evctrl),
            0x04 => Some(Self::Intenclr),
            0x05 => Some(Self::Intenset),
            0x06 => Some(Self::Intflag),
            0x07 => Some(Self::Status),
            // DATA and DATABUF are 16-bit registers whose byte lanes are
            // individually addressable on the SAM D21.
            0x08 | 0x09 => Some(Self::Data),
            0x0c | 0x0d => Some(Self::Databuf),
            _ => None,
        }
    }
}

const DAC_CTRLA_MASK: u8 = 0x07;
const DAC_CTRLA_SWRST: u8 = 1 << 0;
const DAC_CTRLA_ENABLE: u8 = 1 << 1;
const DAC_CTRLB_MASK: u8 = 0xdf;
const DAC_CTRLB_LEFTADJ: u8 = 1 << 2;
const DAC_EVENT_MASK: u8 = 0x03;
const DAC_EVENT_STARTEI: u8 = 1 << 0;
const DAC_INTERRUPT_MASK: u8 = 0x07;
const DAC_INTERRUPT_EMPTY: u8 = 1 << 1;
const DAC_INTERRUPT_UNDERRUN: u8 = 1 << 0;

#[derive(Default)]
struct DacState {
    ctrla: u8,
    ctrlb: u8,
    evctrl: u8,
    interrupt_enable: u8,
    interrupt_flags: u8,
    data: u16,
    databuf: u16,
    databuf_full: bool,
}

/// Host-facing SAM D21 DAC output state.
#[derive(Clone)]
pub struct Samd21DacHandle {
    state: Arc<Mutex<DacState>>,
    output: Option<(SignalHub, SignalId)>,
}

impl Samd21DacHandle {
    /// Returns whether the DAC channel is enabled.
    pub fn enabled(&self) -> bool {
        self.state.lock().expect("DAC lock poisoned").ctrla & DAC_CTRLA_ENABLE != 0
    }

    /// Returns the normalized 10-bit digital output code currently held by DATA.
    pub fn data(&self) -> u16 {
        self.state.lock().expect("DAC lock poisoned").data
    }

    /// Returns the normalized 10-bit code waiting in DATABUF.
    pub fn data_buffer(&self) -> u16 {
        self.state.lock().expect("DAC lock poisoned").databuf
    }

    /// Returns whether DATABUF contains a value waiting for a conversion event.
    pub fn data_buffer_full(&self) -> bool {
        self.state.lock().expect("DAC lock poisoned").databuf_full
    }

    /// Returns the selected reference and output mode bits.
    pub fn control_b(&self) -> u8 {
        self.state.lock().expect("DAC lock poisoned").ctrlb
    }

    /// Returns the interrupt request level derived from enabled flags.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("DAC lock poisoned");
        state.interrupt_flags & state.interrupt_enable != 0
    }

    /// Starts one deterministic conversion from DATABUF, modelling STARTEI.
    ///
    /// A full buffer transfers to DATA and raises EMPTY. An empty buffer raises
    /// UNDERRUN. Analog settling and clock-dependent conversion time are outside
    /// this functional model.
    pub fn start_conversion(&self, at: SimTime) -> Result<(), SignalError> {
        let code = {
            let mut state = self.state.lock().expect("DAC lock poisoned");
            // A start-conversion event is only observed while the controller is
            // enabled and STARTEI is selected. In direct-data mode firmware
            // writes DATA instead of relying on this event path.
            if state.ctrla & DAC_CTRLA_ENABLE == 0 || state.evctrl & DAC_EVENT_STARTEI == 0 {
                return Ok(());
            }
            if state.databuf_full {
                state.data = state.databuf;
                state.databuf_full = false;
                state.interrupt_flags |= DAC_INTERRUPT_EMPTY;
                Some(state.data)
            } else {
                state.interrupt_flags |= DAC_INTERRUPT_UNDERRUN;
                None
            }
        };
        if let (Some(code), Some((hub, signal))) = (code, &self.output) {
            hub.set(*signal, SignalValue::from_u64(u64::from(code), 10)?, at)?;
        }
        Ok(())
    }
}

/// Functional SAM D21 single-channel 10-bit DAC.
///
/// This follows the native register offsets and bit meanings from Microchip
/// DS40001882H §35. It deliberately reports a deterministic digital code rather
/// than attempting analog voltage, settling, or reference-electrical fidelity.
pub struct Samd21Dac {
    name: String,
    state: Arc<Mutex<DacState>>,
    output: Option<(SignalHub, SignalId)>,
}

impl Samd21Dac {
    /// Constructs a DAC without a waveform output signal.
    pub fn new(name: impl Into<String>) -> (Self, Samd21DacHandle) {
        Self::new_inner(name.into(), None)
    }

    /// Constructs a DAC and declares its 10-bit digital output signal.
    pub fn new_with_signals(
        name: impl Into<String>,
        hub: SignalHub,
        path: impl Into<String>,
    ) -> Result<(Self, Samd21DacHandle), SignalError> {
        let signal = hub.declare(
            path,
            SignalValue::from_u64(0, 10)?,
            Some("deterministic DAC conversion code".to_owned()),
        )?;
        Ok(Self::new_inner(name.into(), Some((hub, signal))))
    }

    fn new_inner(name: String, output: Option<(SignalHub, SignalId)>) -> (Self, Samd21DacHandle) {
        let state = Arc::new(Mutex::new(DacState::default()));
        (
            Self {
                name,
                state: state.clone(),
                output: output.clone(),
            },
            Samd21DacHandle { state, output },
        )
    }

    fn reset_state(&mut self, at: SimTime) -> Result<(), DeviceError> {
        *self.state.lock().expect("DAC lock poisoned") = DacState::default();
        self.emit_output(0, at)
    }

    fn emit_output(&self, code: u16, at: SimTime) -> Result<(), DeviceError> {
        if let Some((hub, signal)) = &self.output {
            hub.set(
                *signal,
                SignalValue::from_u64(u64::from(code), 10)
                    .map_err(|error| DeviceError::new(error.to_string()))?,
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))?;
        }
        Ok(())
    }

    fn decode(value: u16, ctrlb: u8) -> u16 {
        if ctrlb & DAC_CTRLB_LEFTADJ != 0 {
            (value >> 6) & 0x03ff
        } else {
            value & 0x03ff
        }
    }

    fn encode(code: u16, ctrlb: u8) -> u16 {
        if ctrlb & DAC_CTRLB_LEFTADJ != 0 {
            (code & 0x03ff) << 6
        } else {
            code & 0x03ff
        }
    }

    fn merge_data_register(
        current: u16,
        offset: u64,
        width: AccessWidth,
        value: u64,
        base: u64,
    ) -> u16 {
        let lane = offset.saturating_sub(base).min(1);
        let shift = lane * 8;
        let mask = (width.value_mask() as u16) << shift;
        (current & !mask) | (((value as u16) << shift) & mask)
    }
}

impl Device for Samd21Dac {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("DAC lock poisoned");
        let value = match Samd21DacRegister::from_offset(offset) {
            Some(Samd21DacRegister::Ctrla) => u64::from(state.ctrla),
            Some(Samd21DacRegister::Ctrlb) => u64::from(state.ctrlb),
            Some(Samd21DacRegister::Evctrl) => u64::from(state.evctrl),
            Some(Samd21DacRegister::Intenclr | Samd21DacRegister::Intenset) => {
                u64::from(state.interrupt_enable)
            }
            Some(Samd21DacRegister::Intflag) => u64::from(state.interrupt_flags),
            Some(Samd21DacRegister::Status) => 0,
            // DATA and DATABUF are write-only in the native register map.
            // Host-side inspection uses Samd21DacHandle instead.
            Some(Samd21DacRegister::Data | Samd21DacRegister::Databuf) => 0,
            None => 0,
        };
        Ok(value & width.value_mask())
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let value = value & width.value_mask();
        if Samd21DacRegister::from_offset(offset) == Some(Samd21DacRegister::Ctrla)
            && value as u8 & DAC_CTRLA_SWRST != 0
        {
            self.reset_state(at)?;
            return Ok(());
        }
        let mut output = None;
        {
            let mut state = self.state.lock().expect("DAC lock poisoned");
            match Samd21DacRegister::from_offset(offset) {
                Some(Samd21DacRegister::Ctrla) => state.ctrla = value as u8 & DAC_CTRLA_MASK,
                Some(Samd21DacRegister::Ctrlb) if state.ctrla & DAC_CTRLA_ENABLE == 0 => {
                    state.ctrlb = value as u8 & DAC_CTRLB_MASK
                }
                Some(Samd21DacRegister::Ctrlb) => {}
                Some(Samd21DacRegister::Evctrl) => state.evctrl = value as u8 & DAC_EVENT_MASK,
                Some(Samd21DacRegister::Intenclr) => {
                    state.interrupt_enable &= !(value as u8 & DAC_INTERRUPT_MASK)
                }
                Some(Samd21DacRegister::Intenset) => {
                    state.interrupt_enable |= value as u8 & DAC_INTERRUPT_MASK
                }
                Some(Samd21DacRegister::Intflag) => {
                    state.interrupt_flags &= !(value as u8 & DAC_INTERRUPT_MASK)
                }
                Some(Samd21DacRegister::Data) => {
                    let raw = Self::merge_data_register(
                        Self::encode(state.data, state.ctrlb),
                        offset,
                        width,
                        value,
                        0x08,
                    );
                    state.data = Self::decode(raw, state.ctrlb);
                    output = Some(state.data);
                }
                Some(Samd21DacRegister::Databuf) => {
                    let raw = Self::merge_data_register(
                        Self::encode(state.databuf, state.ctrlb),
                        offset,
                        width,
                        value,
                        0x0c,
                    );
                    state.databuf = Self::decode(raw, state.ctrlb);
                    state.databuf_full = true;
                    state.interrupt_flags &= !DAC_INTERRUPT_EMPTY;
                }
                Some(Samd21DacRegister::Status) | None => {}
            }
        }
        if let Some(code) = output {
            self.emit_output(code, at)?;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        // Reset cannot report a signal error through the bus trait; a reset
        // transition is still deterministic and the next write remains visible.
        *self.state.lock().expect("DAC lock poisoned") = DacState::default();
        if let Some((hub, signal)) = &self.output {
            let _ = hub.set(
                *signal,
                SignalValue::from_u64(0, 10).expect("fixed DAC signal width is valid"),
                SimTime::ZERO,
            );
        }
    }
}

impl Samd21Usart {
    /// Constructs SERCOM and its observation handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21UsartHandle) {
        let state = Arc::new(Mutex::new(UsartState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 0x34],
            },
            Samd21UsartHandle(state),
        )
    }

    fn raw_register(&self, register: Samd21SercomRegister) -> u32 {
        let state = self.state.lock().expect("USART lock poisoned");
        match register {
            Samd21SercomRegister::Ctrla => state.ctrla,
            Samd21SercomRegister::Ctrlb => state.ctrlb,
            Samd21SercomRegister::Baud => state.baud,
            Samd21SercomRegister::RxPulse => u32::from(state.rx_pulse),
            Samd21SercomRegister::Intenclr | Samd21SercomRegister::Intenset => {
                u32::from(state.interrupt_enable)
            }
            Samd21SercomRegister::Intflag => u32::from(state.flags()),
            Samd21SercomRegister::Status => u32::from(state.status_value()),
            Samd21SercomRegister::Syncbusy => 0,
            Samd21SercomRegister::Addr => state.addr,
            Samd21SercomRegister::Data => 0,
            Samd21SercomRegister::Dbgctrl => u32::from(state.dbgctrl),
        }
    }

    fn store_register(&mut self, register: Samd21SercomRegister, value: u32) {
        let offset = register.offset();
        let bytes = value.to_le_bytes();
        let end = (offset + 4).min(self.registers.len());
        self.registers[offset..end].copy_from_slice(&bytes[..end - offset]);
    }

    fn merged_value(
        &self,
        register: Samd21SercomRegister,
        width: AccessWidth,
        value: u64,
    ) -> Result<u32, DeviceError> {
        let mut bytes = self.raw_register(register).to_le_bytes();
        write_le(&mut bytes, 0, width, value)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn write_data(&mut self, value: u8) {
        let mut state = self.state.lock().expect("USART lock poisoned");
        match state.mode {
            Samd21SercomMode::Usart | Samd21SercomMode::Other(_) => state.bytes.push(value),
            Samd21SercomMode::SpiMaster => {
                state.spi_tx.push(value);
                state.interrupt_flags &= !SERCOM_SPI_INTFLAG_TXC;
                state.interrupt_flags |= SERCOM_SPI_INTFLAG_TXC;
                if state.ctrlb & SERCOM_SPI_CTRLB_RXEN != 0 {
                    let response = state.spi_injected.pop_front().unwrap_or(value);
                    state.spi_rx.push_back(response);
                }
            }
            // No pin-level SPI client clock is available yet; retain the transmit byte but do
            // not fabricate a receive completion for client mode.
            Samd21SercomMode::SpiSlave => state.spi_tx.push(value),
            Samd21SercomMode::I2cMaster => {
                if state.enabled && state.addr & 1 == 0 {
                    state.i2c_tx.push(value);
                    state.interrupt_flags &= !SERCOM_I2C_INTFLAG_MB;
                    state.status &= !SERCOM_I2C_STATUS_CLKHOLD;
                    state.interrupt_flags |= SERCOM_I2C_INTFLAG_MB;
                    state.status |= SERCOM_I2C_STATUS_CLKHOLD;
                }
            }
            Samd21SercomMode::I2cSlave => {}
        }
    }

    fn read_data(&mut self) -> u8 {
        let mut state = self.state.lock().expect("USART lock poisoned");
        match state.mode {
            Samd21SercomMode::SpiMaster => {
                let value = state.spi_rx.pop_front().unwrap_or(0);
                if state.spi_rx.is_empty() {
                    state.interrupt_flags &= !SERCOM_SPI_INTFLAG_RXC;
                }
                value
            }
            Samd21SercomMode::I2cMaster => {
                let value = state.i2c_rx.pop_front().unwrap_or(0);
                state.interrupt_flags &= !SERCOM_I2C_INTFLAG_SB;
                state.status &= !SERCOM_I2C_STATUS_CLKHOLD;
                value
            }
            _ => 0,
        }
    }

    fn issue_i2c_command(&mut self, command: u32) {
        let mut state = self.state.lock().expect("USART lock poisoned");
        if !matches!(
            state.mode,
            Samd21SercomMode::I2cMaster | Samd21SercomMode::I2cSlave
        ) {
            return;
        }
        match command & 0x3 {
            1 => {
                if state.mode == Samd21SercomMode::I2cMaster {
                    state.status = (state.status & !SERCOM_I2C_STATUS_BUSSTATE_MASK)
                        | (u16::from(SERCOM_I2C_BUSSTATE_OWNER) << 4);
                }
                state.interrupt_flags &= !(SERCOM_I2C_INTFLAG_MB | SERCOM_I2C_INTFLAG_SB);
                state.status &= !SERCOM_I2C_STATUS_CLKHOLD;
            }
            2 => {
                state.status = (state.status & !SERCOM_I2C_STATUS_BUSSTATE_MASK)
                    | (u16::from(SERCOM_I2C_BUSSTATE_IDLE) << 4);
                state.interrupt_flags &= !(SERCOM_I2C_INTFLAG_MB | SERCOM_I2C_INTFLAG_SB);
                state.status &= !SERCOM_I2C_STATUS_CLKHOLD;
            }
            3 => {
                state.interrupt_flags &= !(SERCOM_I2C_INTFLAG_MB | SERCOM_I2C_INTFLAG_SB);
                state.status &= !SERCOM_I2C_STATUS_CLKHOLD;
            }
            _ => {}
        }
    }

    fn write_ctrla(&mut self, width: AccessWidth, value: u64) -> Result<(), DeviceError> {
        let raw = self.merged_value(Samd21SercomRegister::Ctrla, width, value)?;
        let mut state = self.state.lock().expect("USART lock poisoned");
        if raw & SERCOM_CTRLA_SWRST != 0 {
            let dbgctrl = state.reset_protocol();
            drop(state);
            self.registers = [0; 0x34];
            self.state.lock().expect("USART lock poisoned").dbgctrl = dbgctrl;
            self.store_register(Samd21SercomRegister::Dbgctrl, u32::from(dbgctrl));
            return Ok(());
        }
        if state.enabled && raw & SERCOM_CTRLA_ENABLE != 0 {
            // CTRLA is enable-protected except ENABLE and SWRST.
            return Ok(());
        }
        state.apply_ctrla(raw);
        let ctrla = state.ctrla;
        drop(state);
        self.store_register(Samd21SercomRegister::Ctrla, ctrla);
        Ok(())
    }

    fn write_ctrlb(&mut self, width: AccessWidth, value: u64) -> Result<(), DeviceError> {
        let raw = self.merged_value(Samd21SercomRegister::Ctrlb, width, value)?;
        let (mode, enabled, current) = {
            let state = self.state.lock().expect("USART lock poisoned");
            (state.mode, state.enabled, state.ctrlb)
        };
        let mask = mode.ctrlb_mask();
        let command = (raw & mode.command_mask()) >> 16;
        let mut state = self.state.lock().expect("USART lock poisoned");
        if enabled {
            // While enabled, only I²C ACKACT/CMD are writable.
            if matches!(
                mode,
                Samd21SercomMode::I2cMaster | Samd21SercomMode::I2cSlave
            ) {
                state.ctrlb = (current & !SERCOM_CTRLB_ACKACT)
                    | (raw & SERCOM_CTRLB_ACKACT)
                    | (current & mask & !mode.command_mask());
            }
        } else {
            state.ctrlb = raw & mask & !mode.command_mask();
        }
        drop(state);
        if command != 0 {
            self.issue_i2c_command(command);
        }
        let ctrlb = self.raw_register(Samd21SercomRegister::Ctrlb);
        self.store_register(Samd21SercomRegister::Ctrlb, ctrlb);
        Ok(())
    }

    fn write_addr(&mut self, width: AccessWidth, value: u64) -> Result<(), DeviceError> {
        let raw = self.merged_value(Samd21SercomRegister::Addr, width, value)?;
        let mut state = self.state.lock().expect("USART lock poisoned");
        let mode = state.mode;
        let addr = raw & mode.addr_mask();
        state.addr = addr;
        if matches!(mode, Samd21SercomMode::I2cMaster) && state.enabled {
            state.i2c_address = Some((addr & 0x7ff) as u16);
            state.status &= !(SERCOM_I2C_STATUS_BUSERR | SERCOM_I2C_STATUS_ARBLOST);
            state.interrupt_flags &= !(SERCOM_I2C_INTFLAG_MB | SERCOM_I2C_INTFLAG_SB);
            state.status = (state.status & !SERCOM_I2C_STATUS_BUSSTATE_MASK)
                | (u16::from(SERCOM_I2C_BUSSTATE_OWNER) << 4);
            if addr & 1 != 0 {
                if state.i2c_rx.is_empty() {
                    state.i2c_rx.push_back(0);
                }
                state.interrupt_flags |= SERCOM_I2C_INTFLAG_SB;
            } else {
                state.interrupt_flags |= SERCOM_I2C_INTFLAG_MB;
            }
            state.status |= SERCOM_I2C_STATUS_CLKHOLD;
        }
        drop(state);
        self.store_register(Samd21SercomRegister::Addr, addr);
        Ok(())
    }

    fn write_status(&mut self, value: u64) {
        let mut state = self.state.lock().expect("USART lock poisoned");
        let raw = value as u16;
        if state.mode == Samd21SercomMode::I2cMaster {
            if raw & SERCOM_I2C_STATUS_BUSSTATE_MASK == 0x10
                && state.status & SERCOM_I2C_STATUS_BUSSTATE_MASK == 0
            {
                state.status = (state.status & !SERCOM_I2C_STATUS_BUSSTATE_MASK) | 0x10;
            }
            state.status &= !(raw
                & (SERCOM_I2C_STATUS_BUSERR
                    | SERCOM_I2C_STATUS_ARBLOST
                    | (1 << 6)
                    | (1 << 8)
                    | (1 << 9)
                    | (1 << 10)));
        } else if state.mode == Samd21SercomMode::I2cSlave {
            state.status &= !(raw & state.mode.status_mask());
        }
    }
}

impl Device for Samd21Usart {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let Some(register) = Samd21SercomRegister::from_offset(
            usize::try_from(offset)
                .map_err(|_| DeviceError::new("SERCOM register offset overflow"))?,
        ) else {
            return read_le(&self.registers, offset, width);
        };
        if register == Samd21SercomRegister::Data {
            return Ok(u64::from(self.read_data()));
        }
        Ok(narrow_u32(self.raw_register(register), width))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let Some(register) = Samd21SercomRegister::from_offset(
            usize::try_from(offset)
                .map_err(|_| DeviceError::new("SERCOM register offset overflow"))?,
        ) else {
            return write_le(&mut self.registers, offset, width, value);
        };
        match register {
            Samd21SercomRegister::Ctrla => self.write_ctrla(width, value)?,
            Samd21SercomRegister::Ctrlb => self.write_ctrlb(width, value)?,
            Samd21SercomRegister::Baud => {
                let raw = self.merged_value(register, width, value)?;
                let mut state = self.state.lock().expect("USART lock poisoned");
                if !state.enabled {
                    state.baud = raw & state.mode.baud_mask();
                }
                let baud = state.baud;
                drop(state);
                self.store_register(register, baud);
            }
            Samd21SercomRegister::RxPulse => {
                let mut state = self.state.lock().expect("USART lock poisoned");
                state.rx_pulse = value as u8;
                let rx_pulse = state.rx_pulse;
                drop(state);
                self.store_register(register, u32::from(rx_pulse));
            }
            Samd21SercomRegister::Intenclr => {
                let mut state = self.state.lock().expect("USART lock poisoned");
                state.interrupt_enable &= !(value as u8 & state.mode.interrupt_mask());
            }
            Samd21SercomRegister::Intenset => {
                let mut state = self.state.lock().expect("USART lock poisoned");
                state.interrupt_enable |= value as u8 & state.mode.interrupt_mask();
            }
            Samd21SercomRegister::Intflag => {
                let mut state = self.state.lock().expect("USART lock poisoned");
                state.interrupt_flags &= !(value as u8 & state.mode.interrupt_mask());
            }
            Samd21SercomRegister::Status => self.write_status(value),
            Samd21SercomRegister::Syncbusy => {}
            Samd21SercomRegister::Addr => self.write_addr(width, value)?,
            Samd21SercomRegister::Data => self.write_data(value as u8),
            Samd21SercomRegister::Dbgctrl => {
                let mut state = self.state.lock().expect("USART lock poisoned");
                state.dbgctrl = value as u8 & 1;
                let dbgctrl = state.dbgctrl;
                drop(state);
                self.store_register(register, u32::from(dbgctrl));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("USART lock poisoned") = UsartState::default();
        self.registers = [0; 0x34];
    }
}

#[derive(Default)]
struct EicState {
    enabled: bool,
    interrupt_enable: u32,
    interrupt_flags: u32,
    config: [u32; 2],
    previous: u32,
    initialized: bool,
}

/// Machine-facing SAM D21 external-interrupt controller state.
#[derive(Clone)]
pub struct Samd21EicHandle(Arc<Mutex<EicState>>);

impl Samd21EicHandle {
    /// Samples the 16 EIC channels and returns the aggregate interrupt level.
    pub fn poll(&self, inputs: u32) -> bool {
        let mut state = self.0.lock().expect("EIC lock poisoned");
        let inputs = inputs & 0xffff;
        if !state.initialized {
            state.previous = inputs;
            state.initialized = true;
        }
        if state.enabled {
            for channel in 0..16_u32 {
                let config = state.config[usize::try_from(channel / 8).expect("small index")];
                let sense = (config >> ((channel % 8) * 4)) & 7;
                let before = state.previous & (1 << channel) != 0;
                let after = inputs & (1 << channel) != 0;
                let triggered = match sense {
                    1 => !before && after,
                    2 => before && !after,
                    3 => before != after,
                    4 => after,
                    5 => !after,
                    _ => false,
                };
                if triggered {
                    state.interrupt_flags |= 1 << channel;
                }
            }
        }
        state.previous = inputs;
        state.interrupt_flags & state.interrupt_enable != 0
    }

    /// Current latched interrupt flags.
    pub fn flags(&self) -> u32 {
        self.0.lock().expect("EIC lock poisoned").interrupt_flags
    }
}

/// Functional SAM D21 EIC enable, sense, flag and interrupt-enable slice.
pub struct Samd21Eic {
    name: String,
    state: Arc<Mutex<EicState>>,
}

impl Samd21Eic {
    /// Constructs EIC and its package-input sampling handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21EicHandle) {
        let state = Arc::new(Mutex::new(EicState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Samd21EicHandle(state),
        )
    }
}

impl Device for Samd21Eic {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, _width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("EIC lock poisoned");
        match offset {
            0x00 => Ok(u64::from(state.enabled) << 1),
            0x08 | 0x0c => Ok(u64::from(state.interrupt_enable)),
            0x10 => Ok(u64::from(state.interrupt_flags)),
            0x18 => Ok(u64::from(state.config[0])),
            0x1c => Ok(u64::from(state.config[1])),
            _ => Ok(0),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let value = value as u32;
        let mut state = self.state.lock().expect("EIC lock poisoned");
        match offset {
            0x00 => state.enabled = value & 2 != 0,
            0x08 => state.interrupt_enable &= !value,
            0x0c => state.interrupt_enable |= value,
            0x10 => state.interrupt_flags &= !value,
            0x18 => state.config[0] = value,
            0x1c => state.config[1] = value,
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("EIC lock poisoned") = EicState::default();
    }
}

#[cfg(test)]
#[path = "samd_tests.rs"]
mod tests;
