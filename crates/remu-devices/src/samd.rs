use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId};
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
struct RtcState {
    control: u32,
    interrupt_enable: u8,
    interrupt_flags: u8,
    count_base: u32,
    compare0: u32,
    epoch: u64,
}

impl RtcState {
    const CONTROL_MASK: u32 = 0x0f8f;
    const ENABLE: u32 = 1 << 1;
    const MODE_MASK: u32 = 0x0c;
    const MATCHCLR: u32 = 1 << 7;
    const PRESCALER_MASK: u32 = 0x0f00;
    const INT_MASK: u8 = 0xc1;
    const CMP0: u8 = 1;
    const OVF: u8 = 1 << 7;

    fn enabled_count32(&self) -> bool {
        self.control & Self::ENABLE != 0 && self.control & Self::MODE_MASK == 0
    }

    fn count(&self, now: SimTime) -> u32 {
        if !self.enabled_count32() {
            return self.count_base;
        }
        let prescaler = ((self.control & Self::PRESCALER_MASK) >> 8).min(10);
        let elapsed = now.ticks().saturating_sub(self.epoch) >> prescaler;
        self.count_base.wrapping_add(elapsed as u32)
    }

    fn advance(&mut self, now: SimTime) {
        if !self.enabled_count32() {
            return;
        }
        let previous = self.count_base;
        let prescaler = ((self.control & Self::PRESCALER_MASK) >> 8).min(10);
        let elapsed = now.ticks().saturating_sub(self.epoch) >> prescaler;
        let count = previous.wrapping_add(elapsed as u32);
        let wrapped = elapsed > u64::from(u32::MAX - previous) || count < previous;
        let compare_match = if wrapped {
            count >= self.compare0 || previous < self.compare0
        } else {
            previous < self.compare0 && count >= self.compare0
        };
        if compare_match {
            self.interrupt_flags |= Self::CMP0;
        }
        if wrapped {
            self.interrupt_flags |= Self::OVF;
        }
        if compare_match && self.control & Self::MATCHCLR != 0 {
            self.count_base = 0;
        } else {
            self.count_base = count;
        }
        self.epoch = now.ticks();
    }
}

/// Machine-facing handle for the SAM D21 RTC COUNT32/COMP0 interrupt slice.
#[derive(Clone)]
pub struct Samd21RtcHandle(Arc<Mutex<RtcState>>);

impl Samd21RtcHandle {
    /// Advances the abstract RTC clock and returns an enabled interrupt level.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("RTC lock poisoned");
        state.advance(now);
        state.interrupt_flags & state.interrupt_enable & RtcState::INT_MASK != 0
    }
}

/// Functional SAM D21 RTC COUNT32 register slice.
pub struct Samd21Rtc {
    name: String,
    state: Arc<Mutex<RtcState>>,
}

impl Samd21Rtc {
    /// Constructs the RTC and its interrupt handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21RtcHandle) {
        let state = Arc::new(Mutex::new(RtcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Samd21RtcHandle(state),
        )
    }
}

impl Device for Samd21Rtc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let mut state = self.state.lock().expect("RTC lock poisoned");
        state.advance(at);
        let value = match offset {
            0x00 => state.control,
            0x06 | 0x07 => u32::from(state.interrupt_enable),
            0x08 => u32::from(state.interrupt_flags),
            0x0a => 0,
            0x10..=0x13 => state.count(at) >> ((offset - 0x10) * 8),
            0x18..=0x1b => state.compare0 >> ((offset - 0x18) * 8),
            _ => 0,
        };
        let mask = if width.bytes() == 4 {
            u64::from(u32::MAX)
        } else {
            (1_u64 << (u32::from(width.bytes()) * 8)) - 1
        };
        Ok(u64::from(value) & mask)
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("RTC lock poisoned");
        state.advance(at);
        match offset {
            0x00 => {
                if value & 1 != 0 {
                    *state = RtcState::default();
                } else {
                    let value = (value as u32) & RtcState::CONTROL_MASK;
                    let protected =
                        RtcState::MODE_MASK | RtcState::MATCHCLR | RtcState::PRESCALER_MASK;
                    let protected_value = if state.enabled_count32() {
                        state.control & protected
                    } else {
                        value & protected
                    };
                    state.control = protected_value | (value & RtcState::ENABLE);
                    state.epoch = at.ticks();
                }
            }
            0x06 => state.interrupt_enable &= !(value as u8 & RtcState::INT_MASK),
            0x07 => state.interrupt_enable |= value as u8 & RtcState::INT_MASK,
            0x08 => state.interrupt_flags &= !(value as u8 & RtcState::INT_MASK),
            0x10 => {
                state.count_base = value as u32;
                state.epoch = at.ticks();
            }
            0x18 => state.compare0 = value as u32,
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RTC lock poisoned") = RtcState::default();
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

    #[test]
    fn rtc_count32_compare_sets_and_clears_cmp0() {
        let (mut rtc, handle) = Samd21Rtc::new("rtc");
        rtc.write(0x18, AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        rtc.write(0x07, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        rtc.write(0x00, AccessWidth::Word, 2 | (1 << 7), SimTime::ZERO)
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(3)));
        assert!(handle.poll(SimTime::from_ticks(4)));
        assert_eq!(
            rtc.read(0x08, AccessWidth::Byte, SimTime::from_ticks(4))
                .unwrap(),
            1
        );
        rtc.write(0x08, AccessWidth::Byte, 1, SimTime::from_ticks(4))
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(4)));
    }

    #[test]
    fn rtc_count32_overflow_sets_native_ovf_flag() {
        let (mut rtc, handle) = Samd21Rtc::new("rtc");
        rtc.write(0x10, AccessWidth::Word, u64::from(u32::MAX), SimTime::ZERO)
            .unwrap();
        rtc.write(0x18, AccessWidth::Word, 0x7fff_ffff, SimTime::ZERO)
            .unwrap();
        rtc.write(0x07, AccessWidth::Byte, 1 << 7, SimTime::ZERO)
            .unwrap();
        rtc.write(0x00, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(1)));
        assert_eq!(
            rtc.read(0x08, AccessWidth::Byte, SimTime::from_ticks(1))
                .unwrap(),
            1 << 7
        );
        rtc.write(0x08, AccessWidth::Byte, 1 << 7, SimTime::from_ticks(1))
            .unwrap();
        assert_eq!(
            rtc.read(0x08, AccessWidth::Byte, SimTime::from_ticks(1))
                .unwrap(),
            0
        );
    }

    #[test]
    fn rtc_control_and_interrupt_registers_use_native_masks() {
        let (mut rtc, _handle) = Samd21Rtc::new("rtc");
        rtc.write(
            0x00,
            AccessWidth::Word,
            u64::from(u32::MAX & !1),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            rtc.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x0f8e
        );
        rtc.write(0x07, AccessWidth::Byte, u64::MAX, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rtc.read(0x07, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0xc1
        );
        rtc.write(0x06, AccessWidth::Byte, u64::MAX, SimTime::ZERO)
            .unwrap();
        assert_eq!(rtc.read(0x06, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0);
    }
}
