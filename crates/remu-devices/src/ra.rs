use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// RA4M1 ELC event number for GPT0 counter overflow.
pub const RA4M1_EVENT_GPT0_OVERFLOW: u16 = 0x05d;
/// RA4M1 ELC event number for SCI9 transmit-data-empty.
pub const RA4M1_EVENT_SCI9_TXI: u16 = 0x0a9;
/// RA4M1 ELC event number for AGT0 underflow/interrupt.
pub const RA4M1_EVENT_AGT0_INT: u16 = 0x01e;
/// RA4M1 ELC event number for AGT1 underflow/interrupt.
pub const RA4M1_EVENT_AGT1_INT: u16 = 0x021;

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
}

/// Host-facing RA4M1 GPT0 state.
#[derive(Clone)]
pub struct RaGptHandle(Arc<Mutex<GptState>>);

impl RaGptHandle {
    /// Advances GPT0 and reports an overflow event pulse/level.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("RA GPT lock poisoned");
        let divider = 1_u64 << state.divider.min(7);
        let period = u64::from(state.period)
            .saturating_add(1)
            .saturating_mul(divider);
        if state.running && period != 0 && now.ticks().saturating_sub(state.started) >= period {
            state.pending = true;
            state.started = now.ticks();
        }
        state.pending && state.overflow_interrupt
    }
}

/// Functional RA4M1 GPT0 counter/overflow slice.
pub struct RaGpt {
    name: String,
    state: Arc<Mutex<GptState>>,
    registers: [u32; 64],
}

impl RaGpt {
    /// Creates GPT0 and its event handle.
    pub fn new(name: impl Into<String>) -> (Self, RaGptHandle) {
        let state = Arc::new(Mutex::new(GptState {
            period: u32::MAX,
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
        let value = match offset {
            0x2c => u32::from(state.running) | (u32::from(state.divider) << 24),
            0x3c => u32::from(state.pending) << 6,
            0x48 => state.counter.wrapping_add(
                (at.ticks().saturating_sub(state.started) >> state.divider.min(7)) as u32,
            ),
            0x64 => state.period,
            _ => self.registers[usize::try_from(offset / 4).unwrap_or(0).min(63)],
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
        match offset {
            0x2c => {
                state.running = value & 1 != 0;
                state.divider = ((value >> 24) & 7) as u8;
                state.started = at.ticks();
            }
            0x38 => state.overflow_interrupt = value & (3 << 6) != 0,
            0x3c => {
                if value & (1 << 6) == 0 {
                    state.pending = false;
                }
            }
            0x48 => {
                state.counter = value;
                state.started = at.ticks();
            }
            0x64 => state.period = value,
            _ => self.registers[usize::try_from(offset / 4).unwrap_or(0).min(63)] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA GPT lock poisoned") = GptState {
            period: u32::MAX,
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
    status: u8,
    interrupt_enable: u8,
    transmitted: Vec<u8>,
    received: VecDeque<u8>,
}

impl IicState {
    const ENABLE: u8 = 1 << 7;
    const START: u8 = 1 << 2;
    const STOP: u8 = 1;
    const BUS_BUSY: u8 = 1 << 7;
    const TDRE: u8 = 1 << 7;
    const TEND: u8 = 1 << 6;
    const RDRF: u8 = 1 << 5;
    const NACKF: u8 = 1 << 4;
    const STOP_DETECTED: u8 = 1 << 3;
    const START_DETECTED: u8 = 1 << 2;

    fn reset() -> Self {
        Self {
            status: Self::TDRE | Self::TEND,
            ..Self::default()
        }
    }

    fn interrupt_pending(&self) -> bool {
        self.status & self.interrupt_enable != 0
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
        state.status |= IicState::RDRF;
    }

    /// Injects a deterministic missing-acknowledgement condition.
    pub fn set_nack(&self) {
        self.0.lock().expect("RA IIC lock poisoned").status |= IicState::NACKF;
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
        self.0.lock().expect("RA IIC lock poisoned").status & IicState::BUS_BUSY != 0
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
            0x00 => state.iccr1,
            0x01 => (state.iccr2 & !IicState::BUS_BUSY) | (state.status & IicState::BUS_BUSY),
            0x02..=0x04 | 0x05..=0x07 => state.registers[offset as usize],
            0x08 => state.status,
            0x09 => 0,
            0x0a..=0x11 => state.registers[offset as usize],
            0x12 => 0,
            0x13 => {
                let value = state.received.pop_front().unwrap_or(0xff);
                if state.received.is_empty() {
                    state.status &= !IicState::RDRF;
                }
                value
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
                state.iccr1 = value;
                if value & IicState::ENABLE == 0 {
                    state.status &= !IicState::BUS_BUSY;
                }
            }
            0x01 => {
                state.iccr2 = value & !IicState::BUS_BUSY;
                if value & IicState::START != 0 {
                    state.status |= IicState::BUS_BUSY | IicState::START_DETECTED;
                }
                if value & IicState::STOP != 0 {
                    state.status &= !IicState::BUS_BUSY;
                    state.status |= IicState::STOP_DETECTED;
                }
            }
            0x07 => state.interrupt_enable = value,
            0x08 => state.status &= !value,
            0x09 => state.registers[9] = value,
            0x12 => {
                state.transmitted.push(value);
                state.status |= IicState::TDRE | IicState::TEND;
            }
            0x13 => {}
            0x02..=0x06 | 0x0a..=0x11 => state.registers[offset as usize] = value,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioport_atomic_output_and_pfs_direction_are_visible() {
        let hub = SignalHub::new();
        let (mut port, handle) = RaIoPort::new("port1", "board.ra.port1", hub).unwrap();
        port.write(0, AccessWidth::Word, 1 << 11, SimTime::ZERO)
            .unwrap();
        port.write(8, AccessWidth::Word, 1 << 11, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.output(), 1 << 11);
        assert_eq!(handle.direction(), 1 << 11);
    }

    #[test]
    fn gpt_and_sci_events_route_through_ielsr() {
        let (mut icu, handle) = RaIcu::new("icu");
        icu.write(
            0x300 + 7 * 4,
            AccessWidth::Word,
            u64::from(RA4M1_EVENT_GPT0_OVERFLOW),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.route_event(RA4M1_EVENT_GPT0_OVERFLOW), vec![7]);

        let (mut gpt, gpt_handle) = RaGpt::new("gpt0");
        gpt.write(0x64, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        gpt.write(0x38, AccessWidth::Word, 1 << 6, SimTime::ZERO)
            .unwrap();
        gpt.write(0x2c, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(gpt_handle.poll(SimTime::from_ticks(4)));

        let (mut sci, sci_handle) = RaSci::new("sci9");
        sci.write(3, AccessWidth::Byte, b'R'.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(sci_handle.bytes(), b"R");
    }

    #[test]
    fn agt_channels_count_down_and_emit_underflow_events() {
        let (mut agt0, handle0) = RaAgt::new("agt0");
        agt0.write(0x00, AccessWidth::HalfWord, 3, SimTime::ZERO)
            .unwrap();
        agt0.write(0x08, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert!(!handle0.poll(SimTime::from_ticks(3)));
        assert!(handle0.poll(SimTime::from_ticks(4)));
        assert!(!handle0.poll(SimTime::from_ticks(4)));
        assert_eq!(
            agt0.read(0x08, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x23
        );
        agt0.write(0x08, AccessWidth::Byte, 1, SimTime::from_ticks(4))
            .unwrap();
        assert_eq!(
            agt0.read(0x08, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x03
        );

        let (mut agt1, handle1) = RaAgt::new("agt1");
        agt1.write(0x00, AccessWidth::HalfWord, 1, SimTime::ZERO)
            .unwrap();
        agt1.write(0x08, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert!(handle1.poll(SimTime::from_ticks(2)));
    }

    #[test]
    fn spi_transfer_sets_status_and_exposes_host_bytes() {
        let (mut spi, handle) = RaSpi::new("spi0");
        spi.write(0, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
            .unwrap();
        spi.write(0x0b, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
            .unwrap();
        handle.queue_rx(0xa5);
        spi.write(4, AccessWidth::Byte, 0x5a, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.take_tx(), vec![0x5a]);
        assert_eq!(
            spi.read(3, AccessWidth::Byte, SimTime::ZERO).unwrap() & 0xa0,
            0xa0
        );
        assert_eq!(spi.read(4, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0xa5);
        assert_eq!(
            spi.read(3, AccessWidth::Byte, SimTime::ZERO).unwrap() & 0x80,
            0
        );
    }

    #[test]
    fn spi_reset_defaults_and_width_selection_follow_ra4m1_registers() {
        let (mut spi, _) = RaSpi::new("spi0");
        assert_eq!(spi.read(0, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0);
        assert_eq!(spi.read(3, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0x20);
        assert_eq!(
            spi.read(0x0a, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0xff
        );
        assert_eq!(
            spi.read(0x10, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0x0401
        );
        assert!(
            spi.write(4, AccessWidth::Byte, 0x12, SimTime::ZERO)
                .is_err()
        );

        spi.write(0x0b, AccessWidth::Byte, 0x40, SimTime::ZERO)
            .unwrap();
        spi.write(0, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
            .unwrap();
        spi.write(4, AccessWidth::Byte, 0x12, SimTime::ZERO)
            .unwrap();
        assert_eq!(spi.read(4, AccessWidth::Byte, SimTime::ZERO).unwrap(), 0);

        spi.reset(ResetKind::PowerOn);
        spi.write(0, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
            .unwrap();
        assert!(
            spi.write(4, AccessWidth::Byte, 0x12, SimTime::ZERO)
                .is_err()
        );
        spi.write(4, AccessWidth::HalfWord, 0x1234, SimTime::ZERO)
            .unwrap();

        spi.reset(ResetKind::PowerOn);
        spi.write(0x0b, AccessWidth::Byte, 1 << 5, SimTime::ZERO)
            .unwrap();
        spi.write(0, AccessWidth::Byte, 1 << 6, SimTime::ZERO)
            .unwrap();
        spi.write(4, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        assert_eq!(spi.read(4, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
    }

    #[test]
    fn iic_start_transmit_receive_and_stop_are_deterministic() {
        let (mut iic, handle) = RaIic::new("iic0");
        iic.write(
            0x00,
            AccessWidth::Byte,
            IicState::ENABLE.into(),
            SimTime::ZERO,
        )
        .unwrap();
        iic.write(
            0x01,
            AccessWidth::Byte,
            IicState::START.into(),
            SimTime::ZERO,
        )
        .unwrap();
        iic.write(0x12, AccessWidth::Byte, 0x6e, SimTime::ZERO)
            .unwrap();
        iic.write(0x12, AccessWidth::Byte, 0xa5, SimTime::ZERO)
            .unwrap();
        iic.write(
            0x07,
            AccessWidth::Byte,
            IicState::TDRE.into(),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.bus_busy());
        assert_eq!(handle.transmitted(), [0x6e, 0xa5]);
        assert!(handle.interrupt_pending());

        handle.enqueue_receive(0x5a);
        assert_eq!(
            iic.read(0x13, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x5a
        );
        handle.set_nack();
        assert_ne!(
            iic.read(0x08, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8 & IicState::NACKF,
            0
        );
        iic.write(
            0x01,
            AccessWidth::Byte,
            IicState::STOP.into(),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.bus_busy());
        assert_eq!(
            iic.read(0x08, AccessWidth::Byte, SimTime::ZERO).unwrap() as u8
                & IicState::STOP_DETECTED,
            IicState::STOP_DETECTED
        );
    }
}
