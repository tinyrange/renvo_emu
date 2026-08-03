use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalError, SignalId, SignalValue};
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

/// Machine-facing handle for the selected SAM D21 TC3 slice.
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

/// Functional SAM D21 TC3 COUNT16 register slice.
pub struct Samd21Tc {
    name: String,
    state: Arc<Mutex<TcState>>,
}

impl Samd21Tc {
    /// Constructs TC3 and its interrupt handle.
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

#[derive(Default)]
struct UsartState {
    enabled: bool,
    interrupt_enable: u8,
    bytes: Vec<u8>,
}

/// Machine-facing SAM D21 SERCOM USART output and interrupt state.
#[derive(Clone)]
pub struct Samd21UsartHandle(Arc<Mutex<UsartState>>);

impl Samd21UsartHandle {
    /// Bytes transmitted through the DATA register.
    pub fn bytes(&self) -> Vec<u8> {
        self.0.lock().expect("USART lock poisoned").bytes.clone()
    }

    /// Whether the data-register-empty interrupt is enabled.
    pub fn interrupt_pending(&self) -> bool {
        self.0.lock().expect("USART lock poisoned").interrupt_enable & 1 != 0
    }
}

/// Functional SAM D21 SERCOM USART startup/transmit slice.
pub struct Samd21Usart {
    name: String,
    state: Arc<Mutex<UsartState>>,
    registers: [u8; 0x30],
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
    /// Constructs SERCOM USART and its observation handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21UsartHandle) {
        let state = Arc::new(Mutex::new(UsartState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 0x30],
            },
            Samd21UsartHandle(state),
        )
    }
}

impl Device for Samd21Usart {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if offset == 0x18 {
            return Ok(1);
        }
        if offset == 0x1c {
            return Ok(0);
        }
        read_le(&self.registers, offset, width)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        match offset {
            0x00 => self.state.lock().expect("USART lock poisoned").enabled = value & 2 != 0,
            0x14 => {
                self.state
                    .lock()
                    .expect("USART lock poisoned")
                    .interrupt_enable &= !(value as u8)
            }
            0x16 => {
                self.state
                    .lock()
                    .expect("USART lock poisoned")
                    .interrupt_enable |= value as u8
            }
            0x28 => self
                .state
                .lock()
                .expect("USART lock poisoned")
                .bytes
                .push(value as u8),
            _ => write_le(&mut self.registers, offset, width, value)?,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("USART lock poisoned") = UsartState::default();
        self.registers = [0; 0x30];
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

#[derive(Default)]
struct WdtState {
    enabled: bool,
    period_code: u8,
    started: u64,
    expired: bool,
}

/// Machine-facing SAM D21 watchdog timeout state.
#[derive(Clone)]
pub struct Samd21WdtHandle(Arc<Mutex<WdtState>>);

impl Samd21WdtHandle {
    /// Advances the functional watchdog and consumes one reset request.
    pub fn take_reset(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("WDT lock poisoned");
        let period = 8_u64 << state.period_code.min(11);
        if state.enabled && now.ticks().saturating_sub(state.started) >= period {
            state.expired = true;
            state.enabled = false;
        }
        std::mem::take(&mut state.expired)
    }
}

/// Functional SAM D21 watchdog enable/configuration/clear slice.
pub struct Samd21Wdt {
    name: String,
    state: Arc<Mutex<WdtState>>,
    interrupt_enable: bool,
    interrupt_flag: bool,
}

impl Samd21Wdt {
    /// Constructs the watchdog and reset-request handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21WdtHandle) {
        let state = Arc::new(Mutex::new(WdtState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                interrupt_enable: false,
                interrupt_flag: false,
            },
            Samd21WdtHandle(state),
        )
    }
}

impl Device for Samd21Wdt {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, _width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("WDT lock poisoned");
        match offset {
            0x00 => Ok(u64::from(state.enabled) << 1),
            0x01 => Ok(u64::from(state.period_code)),
            0x04 | 0x05 => Ok(u64::from(self.interrupt_enable)),
            0x06 => Ok(u64::from(self.interrupt_flag)),
            0x07 => Ok(0),
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
        let mut state = self.state.lock().expect("WDT lock poisoned");
        match offset {
            0x00 => {
                state.enabled = value & 2 != 0;
                state.started = at.ticks();
                state.expired = false;
            }
            0x01 => state.period_code = (value as u8) & 0xf,
            0x04 => self.interrupt_enable &= value & 1 == 0,
            0x05 => self.interrupt_enable |= value & 1 != 0,
            0x06 => self.interrupt_flag &= value & 1 == 0,
            0x08 if value as u8 == 0xa5 => {
                state.started = at.ticks();
                state.expired = false;
            }
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("WDT lock poisoned") = WdtState::default();
        self.interrupt_enable = false;
        self.interrupt_flag = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_aliases_drive_vcd_backed_pins() {
        let hub = SignalHub::new();
        let (mut port, handle) =
            Samd21Port::new("port", 26, "board.atsamd21e18.gpio", hub).unwrap();
        port.write(0x08, AccessWidth::Word, 1 << 7, SimTime::ZERO)
            .unwrap();
        port.write(0x18, AccessWidth::Word, 1 << 7, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.direction(), 1 << 7);
        assert_eq!(handle.output(), 1 << 7);
        assert_eq!(handle.resolved(7).unwrap(), Logic::One);
    }

    #[test]
    fn tc_match_sets_and_clears_mc0_interrupt() {
        let (mut tc, handle) = Samd21Tc::new("tc3");
        tc.write(0x18, AccessWidth::HalfWord, 4, SimTime::ZERO)
            .unwrap();
        tc.write(0x0d, AccessWidth::Byte, 0x10, SimTime::ZERO)
            .unwrap();
        tc.write(0x00, AccessWidth::HalfWord, 2, SimTime::ZERO)
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(3)));
        assert!(handle.poll(SimTime::from_ticks(4)));
        tc.write(0x0e, AccessWidth::Byte, 0x10, SimTime::from_ticks(4))
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(4)));
    }

    #[test]
    fn sercom_data_collects_transmit_bytes() {
        let (mut usart, handle) = Samd21Usart::new("sercom0");
        usart
            .write(0x28, AccessWidth::HalfWord, u64::from(b'A'), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.bytes(), b"A");
    }

    #[test]
    fn dac_models_native_control_data_buffer_and_interrupts() {
        let (mut dac, handle) = Samd21Dac::new("dac");
        assert!(!handle.enabled());
        dac.write(0x01, AccessWidth::Byte, 0x41, SimTime::ZERO)
            .unwrap();
        dac.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        dac.write(0x08, AccessWidth::HalfWord, 0xffff, SimTime::ZERO)
            .unwrap();
        dac.write(0x0c, AccessWidth::HalfWord, 0x0555, SimTime::ZERO)
            .unwrap();
        assert!(handle.enabled());
        assert_eq!(handle.control_b(), 0x41);
        assert_eq!(handle.data(), 0x03ff);
        assert_eq!(handle.data_buffer(), 0x0155);
        assert!(handle.data_buffer_full());
        dac.write(0x02, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        dac.write(0x05, AccessWidth::Byte, 0x02, SimTime::ZERO)
            .unwrap();
        handle.start_conversion(SimTime::from_ticks(3)).unwrap();
        assert_eq!(handle.data(), 0x0155);
        assert!(!handle.data_buffer_full());
        assert!(handle.interrupt_pending());
        assert_eq!(
            dac.read(0x06, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x02
        );
        dac.write(0x06, AccessWidth::Byte, 0x02, SimTime::from_ticks(3))
            .unwrap();
        assert!(!handle.interrupt_pending());
        dac.write(0x00, AccessWidth::Byte, 0x01, SimTime::from_ticks(3))
            .unwrap();
        assert!(!handle.enabled());
        assert_eq!(handle.data(), 0);
    }

    #[test]
    fn dac_decodes_left_adjusted_data_and_reports_underrun() {
        let (mut dac, handle) = Samd21Dac::new("dac");
        dac.write(0x01, AccessWidth::Byte, 0x04, SimTime::ZERO)
            .unwrap();
        dac.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        dac.write(0x02, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        dac.write(0x08, AccessWidth::HalfWord, 0x03fc, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.data(), 0x000f);
        handle.start_conversion(SimTime::from_ticks(1)).unwrap();
        assert_eq!(dac.read(0x06, AccessWidth::Byte, SimTime::ZERO).unwrap(), 1);
    }

    #[test]
    fn dac_honors_native_byte_lanes_and_write_only_data_registers() {
        let (mut dac, handle) = Samd21Dac::new("dac");
        dac.write(0x08, AccessWidth::Byte, 0x02, SimTime::ZERO)
            .unwrap();
        dac.write(0x09, AccessWidth::Byte, 0x03, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.data(), 0x0302);
        assert_eq!(
            dac.read(0x08, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0
        );
        dac.write(0x0c, AccessWidth::Byte, 0x05, SimTime::ZERO)
            .unwrap();
        dac.write(0x0d, AccessWidth::Byte, 0x02, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.data_buffer(), 0x0205);
        assert_eq!(
            dac.read(0x0c, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0
        );
    }

    #[test]
    fn dac_control_b_is_enable_protected() {
        let (mut dac, handle) = Samd21Dac::new("dac");
        dac.write(0x01, AccessWidth::Byte, 0x04, SimTime::ZERO)
            .unwrap();
        dac.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        dac.write(0x01, AccessWidth::Byte, 0, SimTime::from_ticks(1))
            .unwrap();
        assert_eq!(handle.control_b(), 0x04);
    }

    #[test]
    fn eic_latches_a_configured_rising_edge_until_write_one_to_clear() {
        let (mut eic, handle) = Samd21Eic::new("eic");
        eic.write(0x18, AccessWidth::Word, 1 << (3 * 4), SimTime::ZERO)
            .unwrap();
        eic.write(0x0c, AccessWidth::Word, 1 << 3, SimTime::ZERO)
            .unwrap();
        eic.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        assert!(!handle.poll(0));
        assert!(handle.poll(1 << 3));
        assert_eq!(handle.flags(), 1 << 3);
        eic.write(0x10, AccessWidth::Word, 1 << 3, SimTime::ZERO)
            .unwrap();
        assert!(!handle.poll(1 << 3));
    }

    #[test]
    fn watchdog_clear_restarts_the_functional_timeout() {
        let (mut wdt, handle) = Samd21Wdt::new("wdt");
        wdt.write(0x01, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        wdt.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        assert!(!handle.take_reset(SimTime::from_ticks(7)));
        wdt.write(0x08, AccessWidth::Byte, 0xa5, SimTime::from_ticks(7))
            .unwrap();
        assert!(!handle.take_reset(SimTime::from_ticks(14)));
        assert!(handle.take_reset(SimTime::from_ticks(15)));
    }
}
