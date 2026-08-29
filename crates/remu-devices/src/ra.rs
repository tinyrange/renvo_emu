use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// RA4M1 ELC event number for GPT0 counter overflow.
pub const RA4M1_EVENT_GPT0_OVERFLOW: u16 = 0x05d;
/// RA4M1 ELC event number for GPT1 counter overflow.
pub const RA4M1_EVENT_GPT1_OVERFLOW: u16 = 0x065;
/// RA4M1 ELC event number for GPT2 counter overflow.
pub const RA4M1_EVENT_GPT2_OVERFLOW: u16 = 0x06d;
/// RA4M1 ELC event number for GPT3 counter overflow.
pub const RA4M1_EVENT_GPT3_OVERFLOW: u16 = 0x075;
/// RA4M1 ELC event number for GPT4 counter overflow.
pub const RA4M1_EVENT_GPT4_OVERFLOW: u16 = 0x07d;
/// RA4M1 ELC event number for GPT5 counter overflow.
pub const RA4M1_EVENT_GPT5_OVERFLOW: u16 = 0x085;
/// RA4M1 ELC event number for GPT6 counter overflow.
pub const RA4M1_EVENT_GPT6_OVERFLOW: u16 = 0x08d;
/// RA4M1 ELC event number for GPT7 counter overflow.
pub const RA4M1_EVENT_GPT7_OVERFLOW: u16 = 0x095;
/// RA4M1 ELC event number for the key interrupt function.
pub const RA4M1_EVENT_KINT: u16 = 0x045;
/// RA4M1 ELC software event 0.
pub const RA4M1_EVENT_ELC_SOFTWARE0: u16 = 0x053;
/// RA4M1 ELC software event 1.
pub const RA4M1_EVENT_ELC_SOFTWARE1: u16 = 0x054;
/// RA4M1 ELC event number for SCI9 transmit-data-empty.
pub const RA4M1_EVENT_SCI9_TXI: u16 = 0x0a9;
/// RA4M1 ELC event number for AGT0 underflow/interrupt.
pub const RA4M1_EVENT_AGT0_INT: u16 = 0x01e;
/// RA4M1 ELC event number for AGT1 underflow/interrupt.
pub const RA4M1_EVENT_AGT1_INT: u16 = 0x021;

/// Named RA4M1 GPT register identifier for the modeled counter/overflow surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u16)]
pub enum RaGptRegister {
    /// Counter start/stop and clock divider control (GTCR).
    Gtcr = 0x2c,
    /// Counter/compare interrupt enables (GTINTAD).
    Gtintad = 0x38,
    /// Counter status flags (GTST).
    Gtst = 0x3c,
    /// Current counter value (GTCNT).
    Gtcnt = 0x48,
    /// Counter period/reload value (GTPR).
    Gtpr = 0x64,
}

impl RaGptRegister {
    /// Stable list of modeled GPT register IDs.
    pub const ALL: [Self; 5] = [
        Self::Gtcr,
        Self::Gtintad,
        Self::Gtst,
        Self::Gtcnt,
        Self::Gtpr,
    ];

    /// Returns the native GPT byte offset.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    /// Returns the vendor register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gtcr => "gtcr",
            Self::Gtintad => "gtintad",
            Self::Gtst => "gtst",
            Self::Gtcnt => "gtcnt",
            Self::Gtpr => "gtpr",
        }
    }

    /// Resolves a native GPT byte offset to a named register.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x2c => Some(Self::Gtcr),
            0x38 => Some(Self::Gtintad),
            0x3c => Some(Self::Gtst),
            0x48 => Some(Self::Gtcnt),
            0x64 => Some(Self::Gtpr),
            _ => None,
        }
    }
}

/// Named RA4M1 KINT register identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum RaKintRegister {
    /// Key-return edge/flag mode control (KRCTL).
    Krctl = 0x00,
    /// Key-return interrupt flags (KRF).
    Krf = 0x04,
    /// Per-channel key-return enable mask (KRM).
    Krm = 0x08,
}

impl RaKintRegister {
    /// Stable list of modeled KINT register IDs.
    pub const ALL: [Self; 3] = [Self::Krctl, Self::Krf, Self::Krm];

    /// Returns the native KINT byte offset.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    /// Returns the vendor register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Krctl => "krctl",
            Self::Krf => "krf",
            Self::Krm => "krm",
        }
    }

    /// Resolves a native KINT offset to a named register.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x00 => Some(Self::Krctl),
            0x04 => Some(Self::Krf),
            0x08 => Some(Self::Krm),
            _ => None,
        }
    }
}

/// Named RA4M1 Event Link Controller register identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RaElcRegister {
    /// Event Link Controller enable register (ELCR).
    Elcr,
    /// Software event generation register 0 (ELSEGR0).
    Elsegr0,
    /// Software event generation register 1 (ELSEGR1).
    Elsegr1,
    /// Event-link setting register with its destination index (ELSRn).
    Elsr(u8),
}

impl RaElcRegister {
    /// Stable list of modeled ELC register IDs.
    pub const ALL: [Self; 26] = [
        Self::Elcr,
        Self::Elsegr0,
        Self::Elsegr1,
        Self::Elsr(0),
        Self::Elsr(1),
        Self::Elsr(2),
        Self::Elsr(3),
        Self::Elsr(4),
        Self::Elsr(5),
        Self::Elsr(6),
        Self::Elsr(7),
        Self::Elsr(8),
        Self::Elsr(9),
        Self::Elsr(10),
        Self::Elsr(11),
        Self::Elsr(12),
        Self::Elsr(13),
        Self::Elsr(14),
        Self::Elsr(15),
        Self::Elsr(16),
        Self::Elsr(17),
        Self::Elsr(18),
        Self::Elsr(19),
        Self::Elsr(20),
        Self::Elsr(21),
        Self::Elsr(22),
    ];

    /// Returns the native ELC byte offset.
    pub const fn offset(self) -> u64 {
        match self {
            Self::Elcr => 0x00,
            Self::Elsegr0 => 0x02,
            Self::Elsegr1 => 0x04,
            Self::Elsr(index) => 0x10 + (index as u64) * 4,
        }
    }

    /// Returns the vendor register name family.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Elcr => "elcr",
            Self::Elsegr0 => "elsegr0",
            Self::Elsegr1 => "elsegr1",
            Self::Elsr(_) => "elsr",
        }
    }

    /// Returns the destination index for an ELSR register.
    pub const fn link_index(self) -> Option<u8> {
        match self {
            Self::Elsr(index) => Some(index),
            Self::Elcr | Self::Elsegr0 | Self::Elsegr1 => None,
        }
    }

    /// Resolves a native ELC byte offset to a named register.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x00 => Some(Self::Elcr),
            0x02 => Some(Self::Elsegr0),
            0x04 => Some(Self::Elsegr1),
            0x10..=0x68 if (offset - 0x10) % 4 == 0 => {
                Some(Self::Elsr(((offset - 0x10) / 4) as u8))
            }
            _ => None,
        }
    }
}

fn input_bits(state: &Arc<Mutex<GpioState>>) -> u16 {
    state
        .lock()
        .expect("RA IOPORT lock poisoned")
        .nets
        .iter()
        .enumerate()
        .fold(0_u16, |value, (pin, net)| {
            value | (u16::from(net.resolved() == Logic::One) << pin)
        })
}

fn lane(value: u32, offset: u64, width: AccessWidth) -> Result<u64, DeviceError> {
    let shift = usize::try_from(offset & 3).expect("register lane fits usize") * 8;
    let bits = usize::from(width.bytes()) * 8;
    if shift + bits > 32 {
        return Err(DeviceError::new(
            "RA register access crosses a word boundary",
        ));
    }
    let mask = if bits == 32 {
        u32::MAX
    } else {
        (1_u32 << bits) - 1
    };
    Ok(u64::from((value >> shift) & mask))
}

fn merge_lane(
    current: u32,
    offset: u64,
    width: AccessWidth,
    value: u64,
) -> Result<u32, DeviceError> {
    let shift = usize::try_from(offset & 3).expect("register lane fits usize") * 8;
    let bits = usize::from(width.bytes()) * 8;
    if shift + bits > 32 {
        return Err(DeviceError::new(
            "RA register access crosses a word boundary",
        ));
    }
    let lane_mask = if bits == 32 {
        u32::MAX
    } else {
        (1_u32 << bits) - 1
    };
    let mask = lane_mask << shift;
    Ok((current & !mask) | (((value as u32) & lane_mask) << shift))
}

/// One RA4M1 16-pin IOPORT register bank.
pub struct RaIoPort {
    name: String,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
    event_output: u16,
}

impl RaIoPort {
    /// Creates one package-visible port and its host pin handle.
    pub fn new(
        name: impl Into<String>,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), remu_signals::SignalError> {
        let (state, signals, handle) = vendor_gpio(16, path, &hub)?;
        Ok((
            Self {
                name: name.into(),
                state,
                signals,
                hub,
                event_output: 0,
            },
            handle,
        ))
    }

    /// Shared state used by the PFS view of the same package pins.
    fn state(&self) -> Arc<Mutex<GpioState>> {
        self.state.clone()
    }

    fn refresh(&self, at: SimTime) -> Result<(), DeviceError> {
        refresh_gpio(&self.state, &self.signals, &self.hub, 16, at)
    }
}

impl Device for RaIoPort {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let aligned = offset & !3;
        let value = match aligned {
            0x00 => {
                let state = self.state.lock().expect("RA IOPORT lock poisoned");
                (state.direction & 0xffff) | ((state.output & 0xffff) << 16)
            }
            0x04 => {
                let input = u32::from(input_bits(&self.state));
                input | (input << 16)
            }
            0x0c => u32::from(self.event_output) | (u32::from(self.event_output) << 16),
            _ => 0,
        };
        lane(value, offset, width)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let aligned = offset & !3;
        match aligned {
            0x00 => {
                let current = {
                    let state = self.state.lock().expect("RA IOPORT lock poisoned");
                    (state.direction & 0xffff) | ((state.output & 0xffff) << 16)
                };
                let updated = merge_lane(current, offset, width, value)?;
                let mut state = self.state.lock().expect("RA IOPORT lock poisoned");
                state.direction = updated & 0xffff;
                state.output = (updated >> 16) & 0xffff;
            }
            0x08 => {
                let command = merge_lane(0, offset, width, value)?;
                let set = command & 0xffff;
                let clear = command >> 16;
                let mut state = self.state.lock().expect("RA IOPORT lock poisoned");
                state.output = (state.output | set) & !clear;
            }
            0x0c => {
                let current = u32::from(self.event_output) | (u32::from(self.event_output) << 16);
                let updated = merge_lane(current, offset, width, value)?;
                self.event_output = ((updated & 0xffff) | (updated >> 16)) as u16;
            }
            _ => {}
        }
        self.refresh(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.event_output = 0;
        let mut state = self.state.lock().expect("RA IOPORT lock poisoned");
        state.direction = 0;
        state.output = 0;
    }
}

/// RA4M1 pin-function-select array sharing the IOPORT pin latches.
pub struct RaPfs {
    name: String,
    states: Vec<Arc<Mutex<GpioState>>>,
    registers: Vec<u32>,
}

impl RaPfs {
    /// Creates the 15-port by 16-pin PFS window.
    pub fn new(name: impl Into<String>, ports: &[RaIoPort]) -> Self {
        Self {
            name: name.into(),
            states: ports.iter().map(RaIoPort::state).collect(),
            registers: vec![0; 15 * 16],
        }
    }

    fn pin_index(offset: u64) -> Option<(usize, usize)> {
        let index = usize::try_from(offset / 4).ok()?;
        let port = index / 16;
        let pin = index % 16;
        (port < 15).then_some((port, pin))
    }
}

impl Device for RaPfs {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let (port, pin) = Self::pin_index(offset)
            .ok_or_else(|| DeviceError::new(format!("unmodeled RA PFS read at {offset:#x}")))?;
        let mut value = self.registers[port * 16 + pin];
        let input = self
            .states
            .get(port)
            .is_some_and(|state| input_bits(state) & (1 << pin) != 0);
        value = (value & !(1 << 1)) | (u32::from(input) << 1);
        lane(value, offset, width)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let (port, pin) = Self::pin_index(offset)
            .ok_or_else(|| DeviceError::new(format!("unmodeled RA PFS write at {offset:#x}")))?;
        let index = port * 16 + pin;
        self.registers[index] =
            merge_lane(self.registers[index], offset, width, value)? & !(1 << 1);
        if let Some(state) = self.states.get(port) {
            let register = self.registers[index];
            let mut state = state.lock().expect("RA PFS lock poisoned");
            let bit = 1_u32 << pin;
            if register & 1 != 0 {
                state.output |= bit;
            } else {
                state.output &= !bit;
            }
            if register & (1 << 2) != 0 {
                state.direction |= bit;
            } else {
                state.direction &= !bit;
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
    }
}

#[derive(Default)]
struct GptState {
    running: bool,
    overflow_interrupt: bool,
    pending: bool,
    started: u64,
    counter: u32,
    period: u32,
    divider: u8,
    counter_mask: u32,
}

/// Host-facing RA4M1 GPT state.
#[derive(Clone)]
pub struct RaGptHandle(Arc<Mutex<GptState>>);

impl RaGptHandle {
    /// Advances the GPT channel and reports an overflow event pulse/level.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("RA GPT lock poisoned");
        let divider = 1_u64 << state.divider.min(7);
        let period = u64::from(state.period & state.counter_mask)
            .saturating_add(1)
            .saturating_mul(divider);
        if state.running && period != 0 && now.ticks().saturating_sub(state.started) >= period {
            state.pending = true;
            state.started = now.ticks();
        }
        state.pending && state.overflow_interrupt
    }
}

/// Functional RA4M1 GPT counter/overflow slice.
pub struct RaGpt {
    name: String,
    state: Arc<Mutex<GptState>>,
    registers: [u32; 64],
}

impl RaGpt {
    /// Creates a 32-bit GPT and its event handle.
    pub fn new(name: impl Into<String>) -> (Self, RaGptHandle) {
        Self::new_with_mask(name, u32::MAX)
    }

    /// Creates a 16-bit GPT and its event handle.
    pub fn new_16(name: impl Into<String>) -> (Self, RaGptHandle) {
        Self::new_with_mask(name, u16::MAX.into())
    }

    fn new_with_mask(name: impl Into<String>, counter_mask: u32) -> (Self, RaGptHandle) {
        let state = Arc::new(Mutex::new(GptState {
            period: counter_mask,
            counter_mask,
            ..GptState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 64],
            },
            RaGptHandle(state),
        )
    }
}

impl Device for RaGpt {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RA GPT requires word accesses"));
        }
        let state = self.state.lock().expect("RA GPT lock poisoned");
        let value = match RaGptRegister::from_offset(offset) {
            Some(RaGptRegister::Gtcr) => {
                u32::from(state.running) | (u32::from(state.divider) << 24)
            }
            Some(RaGptRegister::Gtst) => u32::from(state.pending) << 6,
            Some(RaGptRegister::Gtcnt) => {
                state.counter.wrapping_add(
                    (at.ticks().saturating_sub(state.started) >> state.divider.min(7)) as u32,
                ) & state.counter_mask
            }
            Some(RaGptRegister::Gtpr) => state.period,
            Some(RaGptRegister::Gtintad) | None => {
                self.registers[usize::try_from(offset / 4).unwrap_or(0).min(63)]
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RA GPT requires word accesses"));
        }
        let value = value as u32;
        let mut state = self.state.lock().expect("RA GPT lock poisoned");
        match RaGptRegister::from_offset(offset) {
            Some(RaGptRegister::Gtcr) => {
                state.running = value & 1 != 0;
                state.divider = ((value >> 24) & 7) as u8;
                state.started = at.ticks();
            }
            Some(RaGptRegister::Gtintad) => state.overflow_interrupt = value & (3 << 6) != 0,
            Some(RaGptRegister::Gtst) => {
                if value & (1 << 6) == 0 {
                    state.pending = false;
                }
            }
            Some(RaGptRegister::Gtcnt) => {
                state.counter = value & state.counter_mask;
                state.started = at.ticks();
            }
            Some(RaGptRegister::Gtpr) => state.period = value & state.counter_mask,
            None => self.registers[usize::try_from(offset / 4).unwrap_or(0).min(63)] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let counter_mask = self
            .state
            .lock()
            .expect("RA GPT lock poisoned")
            .counter_mask;
        *self.state.lock().expect("RA GPT lock poisoned") = GptState {
            period: counter_mask,
            counter_mask,
            ..GptState::default()
        };
        self.registers.fill(0);
    }
}

#[derive(Default)]
struct AgtState {
    running: bool,
    pending: bool,
    underflow: bool,
    started: u64,
    counter: u16,
    reload: u16,
    compare_a: u16,
    compare_b: u16,
    mode1: u8,
    mode2: u8,
    ioc: u8,
    isr: u8,
    cmsr: u8,
    iosel: u8,
}

/// Machine-facing RA4M1 AGT event handle.
#[derive(Clone)]
pub struct RaAgtHandle(Arc<Mutex<AgtState>>);

impl RaAgtHandle {
    /// Advances the timer and consumes one underflow event.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("RA AGT lock poisoned");
        if state.running {
            let period = u64::from(state.reload).saturating_add(1);
            let elapsed = now.ticks().saturating_sub(state.started);
            if elapsed >= period {
                state.started = now.ticks();
                state.counter = state.reload;
                state.pending = true;
                state.underflow = true;
            } else {
                state.counter = state
                    .reload
                    .wrapping_sub(u16::try_from(elapsed).unwrap_or(u16::MAX));
            }
        }
        std::mem::take(&mut state.pending)
    }
}

/// Functional RA4M1 AGT0/AGT1 down-counter and underflow-event slice.
pub struct RaAgt {
    name: String,
    state: Arc<Mutex<AgtState>>,
    registers: [u8; 0x20],
}

impl RaAgt {
    /// Constructs an AGT channel and event handle.
    pub fn new(name: impl Into<String>) -> (Self, RaAgtHandle) {
        let state = Arc::new(Mutex::new(AgtState {
            compare_a: 0,
            counter: 0,
            reload: 0,
            ..AgtState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 0x20],
            },
            RaAgtHandle(state),
        )
    }
}

impl Device for RaAgt {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("RA AGT lock poisoned");
        let value = match offset {
            0x00 => u32::from(state.counter),
            0x02 => u32::from(state.compare_a),
            0x04 => u32::from(state.compare_b),
            0x08 => {
                u32::from(state.running)
                    | (u32::from(state.running) << 1)
                    | (u32::from(state.underflow) << 5)
            }
            0x09 => u32::from(state.mode1),
            0x0a => u32::from(state.mode2),
            0x0c => u32::from(state.ioc),
            0x0d => u32::from(state.isr),
            0x0e => u32::from(state.cmsr),
            0x0f => u32::from(state.iosel),
            _ => u32::from(self.registers[usize::try_from(offset).unwrap_or(0).min(0x1f)]),
        };
        match width {
            AccessWidth::Byte => Ok(u64::from(value & 0xff)),
            AccessWidth::HalfWord if offset & 1 == 0 => Ok(u64::from(value & 0xffff)),
            AccessWidth::Word if offset & 3 == 0 => Ok(u64::from(value)),
            _ => Err(DeviceError::new("RA AGT access is not aligned")),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if !matches!(
            width,
            AccessWidth::Byte | AccessWidth::HalfWord | AccessWidth::Word
        ) {
            return Err(DeviceError::new("RA AGT access width is unsupported"));
        }
        let mut state = self.state.lock().expect("RA AGT lock poisoned");
        match offset {
            0x00 => {
                state.counter = value as u16;
                state.reload = value as u16;
                state.started = at.ticks();
            }
            0x02 => state.compare_a = value as u16,
            0x04 => state.compare_b = value as u16,
            0x08 => {
                state.running = value & 1 != 0;
                if value & 4 != 0 {
                    state.running = false;
                    state.counter = u16::MAX;
                    state.pending = false;
                }
                if value & (1 << 5) == 0 {
                    state.underflow = false;
                }
                state.started = at.ticks();
            }
            0x09 => state.mode1 = value as u8,
            0x0a => state.mode2 = value as u8,
            0x0c => state.ioc = value as u8,
            0x0d => state.isr = value as u8,
            0x0e => state.cmsr = value as u8,
            0x0f => state.iosel = value as u8,
            _ => {
                if let Some(register) = self.registers.get_mut(usize::try_from(offset).unwrap_or(0))
                {
                    *register = value as u8;
                }
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA AGT lock poisoned") = AgtState {
            compare_a: 0,
            counter: 0,
            reload: 0,
            ..AgtState::default()
        };
        self.registers = [0; 0x20];
    }
}

#[derive(Default)]
struct SciState {
    scr: u8,
    bytes: Vec<u8>,
}

/// Host-facing RA4M1 SCI9 output and event state.
#[derive(Clone)]
pub struct RaSciHandle(Arc<Mutex<SciState>>);

impl RaSciHandle {
    /// Bytes written to SCI9.TDR.
    pub fn bytes(&self) -> Vec<u8> {
        self.0.lock().expect("RA SCI lock poisoned").bytes.clone()
    }
    /// Whether the TXI event is enabled and asserted.
    pub fn txi_pending(&self) -> bool {
        self.0.lock().expect("RA SCI lock poisoned").scr & 0x80 != 0
    }
}

/// Functional RA4M1 SCI9 asynchronous transmit slice.
pub struct RaSci {
    name: String,
    state: Arc<Mutex<SciState>>,
    registers: [u8; 32],
}

#[derive(Default)]
struct IicState {
    iccr1: u8,
    iccr2: u8,
    registers: [u8; 0x12],
    status1: u8,
    status2: u8,
    interrupt_enable: u8,
    transmit_data: u8,
    receive_data: u8,
    transmitted: Vec<u8>,
    received: VecDeque<u8>,
}

impl IicState {
    const ICE: u8 = 1 << 7;
    const IICRST: u8 = 1 << 6;
    const SOWP: u8 = 1 << 4;
    const BBSY: u8 = 1 << 7;
    const MST: u8 = 1 << 6;
    const TRS: u8 = 1 << 5;
    const STOP_REQUEST: u8 = 1 << 3;
    const RESTART: u8 = 1 << 2;
    const START_REQUEST: u8 = 1 << 1;
    const TDRE: u8 = 1 << 7;
    const TEND: u8 = 1 << 6;
    const RDRF: u8 = 1 << 5;
    const NACKF: u8 = 1 << 4;
    const STOP_DETECTED: u8 = 1 << 3;
    const START_DETECTED: u8 = 1 << 2;
    const ICSR2_WRITABLE: u8 = 0x7f;

    fn reset() -> Self {
        let mut registers = [0; 0x12];
        // Native reset values for ICMR1/2, ICFER, ICSER, ICBRL, and ICBRH.
        registers[0x02] = 0x08;
        registers[0x03] = 0x06;
        registers[0x05] = 0x72;
        registers[0x06] = 0x09;
        registers[0x10] = 0xff;
        registers[0x11] = 0xff;
        Self {
            iccr1: 0x1f,
            registers,
            transmit_data: 0xff,
            ..Self::default()
        }
    }

    fn interrupt_pending(&self) -> bool {
        self.status2 & self.interrupt_enable != 0
    }

    fn read_register(&self, offset: u64) -> u8 {
        match offset {
            0x00 => self.iccr1 | Self::SOWP,
            0x01 => self.iccr2,
            0x02 => self.registers[2] | 1 << 3,
            0x03 => self.registers[3] & 0xf7,
            0x04 => self.registers[4],
            0x05 => self.registers[5] & 0x7f,
            0x06 => self.registers[6] & 0xaf,
            0x07 => self.interrupt_enable,
            0x08 => self.status1,
            0x09 => self.status2,
            0x0a..=0x0f => self.registers[offset as usize],
            0x10 | 0x11 => self.registers[offset as usize] | 0xe0,
            0x12 => self.transmit_data,
            _ => 0,
        }
    }
}

/// Host-facing RA4M1 IIC transfer and status state.
#[derive(Clone)]
pub struct RaIicHandle(Arc<Mutex<IicState>>);

impl RaIicHandle {
    /// Returns all bytes written to ICDRT since reset.
    pub fn transmitted(&self) -> Vec<u8> {
        self.0
            .lock()
            .expect("RA IIC lock poisoned")
            .transmitted
            .clone()
    }

    /// Queues one deterministic byte for a guest ICDRR read.
    pub fn enqueue_receive(&self, byte: u8) {
        let mut state = self.0.lock().expect("RA IIC lock poisoned");
        state.received.push_back(byte);
        state.status2 |= IicState::RDRF;
    }

    /// Injects a deterministic missing-acknowledgement condition.
    pub fn set_nack(&self) {
        self.0.lock().expect("RA IIC lock poisoned").status2 |= IicState::NACKF;
    }

    /// Whether an enabled IIC status bit requests service.
    pub fn interrupt_pending(&self) -> bool {
        self.0
            .lock()
            .expect("RA IIC lock poisoned")
            .interrupt_pending()
    }

    /// Whether the controller currently owns a started bus.
    pub fn bus_busy(&self) -> bool {
        self.0.lock().expect("RA IIC lock poisoned").iccr2 & IicState::BBSY != 0
    }
}

/// Functional RA4M1 IIC master register and byte-transfer slice.
///
/// Start/stop control, transmit and receive data paths, status flags, and
/// status-interrupt enables are modeled deterministically. Bit-level bus
/// arbitration, clock stretching, and electrical open-drain resolution remain
/// outside this functional boundary.
pub struct RaIic {
    name: String,
    state: Arc<Mutex<IicState>>,
}

impl RaIic {
    /// Constructs an IIC instance and its host transfer handle.
    pub fn new(name: impl Into<String>) -> (Self, RaIicHandle) {
        let state = Arc::new(Mutex::new(IicState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaIicHandle(state),
        )
    }
}

impl Device for RaIic {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("RA IIC requires byte accesses"));
        }
        let mut state = self.state.lock().expect("RA IIC lock poisoned");
        let value = match offset {
            0x00..=0x12 => state.read_register(offset),
            0x13 => {
                if let Some(value) = state.received.pop_front() {
                    state.receive_data = value;
                }
                if state.received.is_empty() {
                    state.status2 &= !IicState::RDRF;
                }
                state.receive_data
            }
            _ => 0,
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("RA IIC requires byte accesses"));
        }
        let value = value as u8;
        let mut state = self.state.lock().expect("RA IIC lock poisoned");
        match offset {
            0x00 => {
                if value & IicState::IICRST != 0 {
                    let ice = value & IicState::ICE;
                    *state = IicState::reset();
                    state.iccr1 = ice | IicState::IICRST | IicState::SOWP;
                } else {
                    state.iccr1 = value & 0xef;
                    if value & IicState::ICE == 0 {
                        state.iccr2 &= !IicState::BBSY;
                    }
                }
            }
            0x01 => {
                let busy = state.iccr2 & IicState::BBSY != 0;
                let old_mode = state.iccr2 & (IicState::MST | IicState::TRS);
                let requested_mode = value & (IicState::MST | IicState::TRS);
                let mode = if requested_mode != 0 {
                    requested_mode
                } else {
                    old_mode
                };
                state.iccr2 = (state.iccr2 & IicState::BBSY)
                    | mode
                    | (value
                        & (IicState::STOP_REQUEST | IicState::RESTART | IicState::START_REQUEST));
                if value & IicState::START_REQUEST != 0 && !busy && state.iccr1 & IicState::ICE != 0
                {
                    state.iccr2 |= IicState::BBSY | IicState::MST;
                    state.iccr2 &= !IicState::START_REQUEST;
                    state.status2 |= IicState::START_DETECTED;
                }
                if value & IicState::RESTART != 0 && busy && state.iccr2 & IicState::MST != 0 {
                    state.iccr2 &= !IicState::RESTART;
                    state.status2 |= IicState::START_DETECTED;
                }
                if value & IicState::STOP_REQUEST != 0 && busy && state.iccr2 & IicState::MST != 0 {
                    state.iccr2 &=
                        !(IicState::BBSY | IicState::MST | IicState::TRS | IicState::STOP_REQUEST);
                    state.status2 |= IicState::STOP_DETECTED;
                }
            }
            0x02 => state.registers[2] = value,
            0x03 => state.registers[3] = value & 0xf7,
            0x04 => state.registers[4] = value,
            0x05 => state.registers[5] = value & 0x7f,
            0x06 => state.registers[6] = value & 0xaf,
            0x07 => state.interrupt_enable = value,
            0x08 => state.status1 &= value,
            0x09 => {
                state.status2 =
                    (state.status2 & !IicState::ICSR2_WRITABLE) | (state.status2 & value)
            }
            0x0a..=0x0f => state.registers[offset as usize] = value,
            0x10 | 0x11 => state.registers[offset as usize] = value & 0x1f,
            0x12 => {
                state.transmit_data = value;
                state.transmitted.push(value);
                state.status2 |= IicState::TDRE | IicState::TEND;
            }
            0x13 => {}
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA IIC lock poisoned") = IicState::reset();
    }
}

impl RaSci {
    /// Creates SCI9 and its machine handle.
    pub fn new(name: impl Into<String>) -> (Self, RaSciHandle) {
        let state = Arc::new(Mutex::new(SciState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 32],
            },
            RaSciHandle(state),
        )
    }
}

impl Device for RaSci {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("RA SCI requires byte accesses"));
        }
        let value = match offset {
            0x02 => self.state.lock().expect("RA SCI lock poisoned").scr,
            0x04 => 0x84,
            _ => *self.registers.get(offset as usize).unwrap_or(&0),
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("RA SCI requires byte accesses"));
        }
        match offset {
            0x02 => self.state.lock().expect("RA SCI lock poisoned").scr = value as u8,
            0x03 => self
                .state
                .lock()
                .expect("RA SCI lock poisoned")
                .bytes
                .push(value as u8),
            _ => {
                if let Some(register) = self.registers.get_mut(offset as usize) {
                    *register = value as u8;
                }
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA SCI lock poisoned") = SciState::default();
        self.registers.fill(0);
    }
}

const SPI_SPCR_SPRIE: u8 = 1 << 7;
const SPI_SPCR_SPE: u8 = 1 << 6;
const SPI_SPCR_SPTIE: u8 = 1 << 5;
const SPI_SPCR_SPEIE: u8 = 1 << 4;
const SPI_SPCR_MSTR: u8 = 1 << 3;
const SPI_SPCR_MODFEN: u8 = 1 << 2;
const SPI_SPCR_TXMD: u8 = 1 << 1;
const SPI_SPCR_SPMS: u8 = 1;
const SPI_SPCR_MASK: u8 = SPI_SPCR_SPRIE
    | SPI_SPCR_SPE
    | SPI_SPCR_SPTIE
    | SPI_SPCR_SPEIE
    | SPI_SPCR_MSTR
    | SPI_SPCR_MODFEN
    | SPI_SPCR_TXMD
    | SPI_SPCR_SPMS;
const SPI_SPSR_SPRF: u8 = 1 << 7;
const SPI_SPSR_SPTEF: u8 = 1 << 5;
const SPI_SPSR_UDRF: u8 = 1 << 4;
const SPI_SPSR_PERF: u8 = 1 << 3;
const SPI_SPSR_MODF: u8 = 1 << 2;
const SPI_SPSR_IDLNF: u8 = 1 << 1;
const SPI_SPSR_OVRF: u8 = 1;
const SPI_SPSR_READ_MASK: u8 = SPI_SPSR_SPRF
    | SPI_SPSR_SPTEF
    | SPI_SPSR_UDRF
    | SPI_SPSR_PERF
    | SPI_SPSR_MODF
    | SPI_SPSR_IDLNF
    | SPI_SPSR_OVRF;
const SPI_SPSR_CLEAR_ZERO_MASK: u8 = SPI_SPSR_UDRF | SPI_SPSR_PERF | SPI_SPSR_MODF | SPI_SPSR_OVRF;
const SPI_SPDCR_SPRDTD: u8 = 1 << 4;
const SPI_SPDCR_SPLW: u8 = 1 << 5;
const SPI_SPDCR_SPBYT: u8 = 1 << 6;
const SPI_SPDCR_MASK: u8 = SPI_SPDCR_SPRDTD | SPI_SPDCR_SPLW | SPI_SPDCR_SPBYT;
const SPI_SPCR2_MASK: u8 = 0x1f;
const SPI_SPCMD0_MASK: u16 = 0xff7f;
const SPI_SPCMD0_RESET: u16 = 0x0401;

#[derive(Default)]
struct SpiState {
    control: u8,
    status: u8,
    command: u16,
    tx_data: u32,
    rx_data: u32,
    tx: Vec<u32>,
    rx: VecDeque<u32>,
}

/// Host-facing RA4M1 RSPI transfer state.
#[derive(Clone)]
pub struct RaSpiHandle(Arc<Mutex<SpiState>>);

impl RaSpiHandle {
    /// Queues one word returned by the next enabled transfer.
    pub fn queue_rx(&self, value: u32) {
        self.0
            .lock()
            .expect("RA SPI lock poisoned")
            .rx
            .push_back(value);
    }

    /// Consumes words written through SPDR.
    pub fn take_tx(&self) -> Vec<u32> {
        std::mem::take(&mut self.0.lock().expect("RA SPI lock poisoned").tx)
    }
}

/// Functional RA4M1 RSPI0/RSPI1 master transfer slice.
pub struct RaSpi {
    name: String,
    state: Arc<Mutex<SpiState>>,
    registers: [u8; 0x11],
}

impl RaSpi {
    /// Creates an RSPI instance and its host transfer handle.
    pub fn new(name: impl Into<String>) -> (Self, RaSpiHandle) {
        let state = Arc::new(Mutex::new(SpiState {
            status: SPI_SPSR_SPTEF,
            command: SPI_SPCMD0_RESET,
            ..SpiState::default()
        }));
        let mut registers = [0; 0x11];
        registers[0x0a] = u8::MAX;
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers,
            },
            RaSpiHandle(state),
        )
    }

    fn data_access_is_valid(control: u8, width: AccessWidth) -> bool {
        match (control & (SPI_SPDCR_SPBYT | SPI_SPDCR_SPLW), width) {
            (SPI_SPDCR_SPBYT, AccessWidth::Byte)
            | (SPI_SPDCR_SPLW, AccessWidth::Word)
            | (0, AccessWidth::HalfWord) => true,
            _ => false,
        }
    }

    fn data_value(state: &SpiState, control: u8, width: AccessWidth) -> Result<u64, DeviceError> {
        if !Self::data_access_is_valid(control, width) {
            return Err(DeviceError::new(
                "RA SPI SPDR access width does not match SPDCR",
            ));
        }
        let value = if control & SPI_SPDCR_SPRDTD != 0 {
            state.tx_data
        } else {
            state.rx_data
        };
        match width {
            AccessWidth::Byte => Ok(u64::from(value & 0xff)),
            AccessWidth::HalfWord => Ok(u64::from(value & 0xffff)),
            AccessWidth::Word => Ok(u64::from(value)),
            AccessWidth::DoubleWord => Err(DeviceError::new("RA SPI data is at most 32 bits")),
        }
    }

    fn reset_state(&mut self) {
        let mut state = self.state.lock().expect("RA SPI lock poisoned");
        *state = SpiState {
            status: SPI_SPSR_SPTEF,
            command: SPI_SPCMD0_RESET,
            ..SpiState::default()
        };
        self.registers.fill(0);
        self.registers[0x0a] = u8::MAX;
    }
}

impl Device for RaSpi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let mut state = self.state.lock().expect("RA SPI lock poisoned");
        if offset == 0x04 {
            let control = self.registers[0x0b];
            let value = Self::data_value(&state, control, width)?;
            if control & SPI_SPDCR_SPRDTD == 0 {
                state.status &= !SPI_SPSR_SPRF;
            }
            return Ok(value);
        }
        if offset == 0x10 {
            return match width {
                AccessWidth::Byte => Ok(u64::from(state.command as u8)),
                AccessWidth::HalfWord => Ok(u64::from(state.command)),
                _ => Err(DeviceError::new(
                    "RA SPI SPCMD0 requires byte or halfword access",
                )),
            };
        }
        if offset == 0x11 && width == AccessWidth::Byte {
            return Ok(u64::from((state.command >> 8) as u8));
        }
        if width != AccessWidth::Byte {
            return Err(DeviceError::new(
                "RA SPI control registers require byte accesses",
            ));
        }
        let value = match offset {
            0x00 => state.control,
            0x03 => state.status & SPI_SPSR_READ_MASK,
            _ => self.registers.get(offset as usize).copied().unwrap_or(0),
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("RA SPI lock poisoned");
        if offset == 0x04 {
            let control = self.registers[0x0b];
            if !Self::data_access_is_valid(control, width) {
                return Err(DeviceError::new(
                    "RA SPI SPDR access width does not match SPDCR",
                ));
            }
            let value = match width {
                AccessWidth::Byte => value & 0xff,
                AccessWidth::HalfWord => value & 0xffff,
                AccessWidth::Word => value & u64::from(u32::MAX),
                AccessWidth::DoubleWord => {
                    return Err(DeviceError::new("RA SPI data is at most 32 bits"));
                }
            } as u32;
            if state.control & SPI_SPCR_SPE != 0 && state.status & SPI_SPSR_SPTEF != 0 {
                state.tx_data = value;
                state.status &= !SPI_SPSR_SPTEF;
                state.tx.push(value);
                if state.control & SPI_SPCR_TXMD == 0 {
                    state.rx_data = state.rx.pop_front().unwrap_or(0);
                    state.status |= SPI_SPSR_SPRF;
                }
                // Functional execution completes the transfer immediately;
                // the transmit buffer is empty again at the next observable
                // boundary.
                state.status |= SPI_SPSR_SPTEF;
            }
            return Ok(());
        }
        if offset == 0x10 {
            match width {
                AccessWidth::Byte => {
                    state.command = (state.command & 0xff00) | (value as u16 & 0xff);
                }
                AccessWidth::HalfWord => state.command = value as u16 & SPI_SPCMD0_MASK,
                _ => {
                    return Err(DeviceError::new(
                        "RA SPI SPCMD0 requires byte or halfword access",
                    ));
                }
            }
            state.command &= SPI_SPCMD0_MASK;
            return Ok(());
        }
        if offset == 0x11 && width == AccessWidth::Byte {
            state.command = (state.command & 0x00ff) | ((value as u16 & 0xff) << 8);
            state.command &= SPI_SPCMD0_MASK;
            return Ok(());
        }
        if width != AccessWidth::Byte {
            return Err(DeviceError::new(
                "RA SPI control registers require byte accesses",
            ));
        }
        let value = value as u8;
        match offset {
            0x00 => {
                let was_enabled = state.control & SPI_SPCR_SPE != 0;
                state.control = value & SPI_SPCR_MASK;
                if was_enabled && value & SPI_SPCR_SPE == 0 {
                    state.status |= SPI_SPSR_SPTEF;
                }
            }
            0x03 => {
                // Error flags clear only when firmware writes zero after
                // observing them; SPTEF/SPRF are hardware flags and the
                // reserved bit is read as zero.
                state.status &= !(SPI_SPSR_CLEAR_ZERO_MASK & !value);
            }
            0x01 => self.registers[0x01] = value & 0x03,
            0x02 => self.registers[0x02] = value & 0x37,
            0x0b => self.registers[0x0b] = value & SPI_SPDCR_MASK,
            0x0c | 0x0d | 0x0e => {
                if let Some(register) = self.registers.get_mut(offset as usize) {
                    *register = value & 0x07;
                }
            }
            0x0f => self.registers[0x0f] = value & SPI_SPCR2_MASK,
            _ => {
                if let Some(register) = self.registers.get_mut(offset as usize) {
                    *register = value;
                }
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset_state();
    }
}

struct IcuState {
    ielsr: [u32; 96],
}

impl Default for IcuState {
    fn default() -> Self {
        Self { ielsr: [0; 96] }
    }
}

/// Host-facing explicit ICU event-to-NVIC router.
#[derive(Clone)]
pub struct RaIcuHandle(Arc<Mutex<IcuState>>);

impl RaIcuHandle {
    /// Latches one ELC event and returns every NVIC line configured for it.
    pub fn route_event(&self, event: u16) -> Vec<u16> {
        let mut state = self.0.lock().expect("RA ICU lock poisoned");
        let mut lines = Vec::new();
        for (line, register) in state.ielsr.iter_mut().enumerate() {
            if *register & 0x1ff == u32::from(event) {
                *register |= 1 << 16;
                lines.push(u16::try_from(line).expect("RA ICU line fits u16"));
            }
        }
        lines
    }
}

/// RA4M1 ICU IELSR register window.
pub struct RaIcu {
    name: String,
    state: Arc<Mutex<IcuState>>,
}

impl RaIcu {
    /// Creates the ICU and shared routing handle.
    pub fn new(name: impl Into<String>) -> (Self, RaIcuHandle) {
        let state = Arc::new(Mutex::new(IcuState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaIcuHandle(state),
        )
    }
}

impl Device for RaIcu {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !(0x300..0x480).contains(&offset) || offset & 3 != 0 {
            return Ok(0);
        }
        let index = usize::try_from((offset - 0x300) / 4).expect("IELSR index fits usize");
        Ok(u64::from(
            self.state.lock().expect("RA ICU lock poisoned").ielsr[index],
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !(0x300..0x480).contains(&offset) || offset & 3 != 0 {
            return Ok(());
        }
        let index = usize::try_from((offset - 0x300) / 4).expect("IELSR index fits usize");
        let mut state = self.state.lock().expect("RA ICU lock poisoned");
        let old = state.ielsr[index];
        let mut updated = value as u32 & 0x0101_01ff;
        if value & (1 << 16) != 0 {
            updated = (updated & !(1 << 16)) | (old & (1 << 16));
        }
        state.ielsr[index] = updated;
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("RA ICU lock poisoned")
            .ielsr
            .fill(0);
    }
}

#[path = "ra_elc.rs"]
mod elc;
pub use elc::*;

#[cfg(test)]
#[path = "ra_tests.rs"]
mod tests;
