use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId};
use std::sync::{Arc, Mutex};

/// RA4M1 ELC event number for GPT0 counter overflow.
pub const RA4M1_EVENT_GPT0_OVERFLOW: u16 = 0x05d;
/// RA4M1 ELC event number for SCI9 transmit-data-empty.
pub const RA4M1_EVENT_SCI9_TXI: u16 = 0x0a9;
/// RA4M1 ELC event number for software event 0.
pub const RA4M1_EVENT_ELC_SOFTWARE0: u16 = 0x053;
/// RA4M1 ELC event number for software event 1.
pub const RA4M1_EVENT_ELC_SOFTWARE1: u16 = 0x054;

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

const RA4M1_ELC_LINKS: usize = 23;

#[derive(Default)]
struct ElcState {
    enabled: bool,
    links: [u16; RA4M1_ELC_LINKS],
    software_events: Vec<u16>,
}

/// Host-facing RA4M1 Event Link Controller state.
#[derive(Clone)]
pub struct RaElcHandle(Arc<Mutex<ElcState>>);

impl RaElcHandle {
    /// Returns and clears software events generated by guest writes.
    pub fn take_software_events(&self) -> Vec<u16> {
        let mut state = self.0.lock().expect("RA ELC lock poisoned");
        std::mem::take(&mut state.software_events)
    }

    /// Returns destination link indices selected for an event source.
    pub fn route_event(&self, event: u16) -> Vec<u8> {
        let state = self.0.lock().expect("RA ELC lock poisoned");
        if !state.enabled {
            return Vec::new();
        }
        state
            .links
            .iter()
            .enumerate()
            .filter_map(|(index, link)| {
                (*link == (event & 0x01ff))
                    .then(|| u8::try_from(index).expect("RA ELC link index fits u8"))
            })
            .collect()
    }

    /// Indicates whether the all-links enable bit is set.
    pub fn enabled(&self) -> bool {
        self.0.lock().expect("RA ELC lock poisoned").enabled
    }
}

/// Functional RA4M1 Event Link Controller register/link slice.
pub struct RaElc {
    name: String,
    state: Arc<Mutex<ElcState>>,
}

impl RaElc {
    /// Creates the ELC and its host-facing event handle.
    pub fn new(name: impl Into<String>) -> (Self, RaElcHandle) {
        let state = Arc::new(Mutex::new(ElcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaElcHandle(state),
        )
    }
}

impl Device for RaElc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("RA ELC lock poisoned");
        match (RaElcRegister::from_offset(offset), width) {
            (Some(RaElcRegister::Elcr), AccessWidth::Byte) => {
                Ok(u64::from(u8::from(state.enabled) << 7))
            }
            (Some(RaElcRegister::Elsegr0 | RaElcRegister::Elsegr1), AccessWidth::Byte) => Ok(0),
            (Some(RaElcRegister::Elsr(index)), AccessWidth::HalfWord) => state
                .links
                .get(usize::from(index))
                .copied()
                .map(u64::from)
                .ok_or_else(|| DeviceError::new("RA ELC link index out of range")),
            _ => Err(DeviceError::new("RA ELC access width is not supported")),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("RA ELC lock poisoned");
        match (RaElcRegister::from_offset(offset), width) {
            (Some(RaElcRegister::Elcr), AccessWidth::Byte) => state.enabled = value & 0x80 != 0,
            (
                Some(register @ (RaElcRegister::Elsegr0 | RaElcRegister::Elsegr1)),
                AccessWidth::Byte,
            ) => {
                // SEG is write-only and requires WE; WI disables writes.
                if value & 0x40 != 0 && value & 0x80 == 0 && value & 1 != 0 && state.enabled {
                    state
                        .software_events
                        .push(if register == RaElcRegister::Elsegr0 {
                            RA4M1_EVENT_ELC_SOFTWARE0
                        } else {
                            RA4M1_EVENT_ELC_SOFTWARE1
                        });
                }
            }
            (Some(RaElcRegister::Elsr(index)), AccessWidth::HalfWord) => {
                state.links[usize::from(index)] = value as u16 & 0x01ff;
            }
            _ => return Err(DeviceError::new("RA ELC access width is not supported")),
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA ELC lock poisoned") = ElcState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elc_register_ids_are_named_and_native() {
        assert_eq!(RaElcRegister::ALL.len(), 26);
        assert_eq!(RaElcRegister::Elcr.offset(), 0x00);
        assert_eq!(RaElcRegister::Elsegr1.name(), "elsegr1");
        assert_eq!(
            RaElcRegister::from_offset(0x10),
            Some(RaElcRegister::Elsr(0))
        );
        assert_eq!(RaElcRegister::Elsr(22).offset(), 0x68);
        assert_eq!(RaElcRegister::from_offset(0x0c), None);
    }

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
    fn elc_links_and_software_events_are_enable_gated() {
        let (mut elc, handle) = RaElc::new("elc");
        elc.write(
            RaElcRegister::Elsr(0).offset(),
            AccessWidth::HalfWord,
            RA4M1_EVENT_ELC_SOFTWARE0.into(),
            SimTime::ZERO,
        )
        .unwrap();
        elc.write(
            RaElcRegister::Elsegr0.offset(),
            AccessWidth::Byte,
            0x41,
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.take_software_events().is_empty());
        assert!(handle.route_event(RA4M1_EVENT_ELC_SOFTWARE0).is_empty());

        elc.write(
            RaElcRegister::Elcr.offset(),
            AccessWidth::Byte,
            0x80,
            SimTime::ZERO,
        )
        .unwrap();
        elc.write(
            RaElcRegister::Elsegr0.offset(),
            AccessWidth::Byte,
            0x41,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            handle.take_software_events(),
            vec![RA4M1_EVENT_ELC_SOFTWARE0]
        );
        assert_eq!(handle.route_event(RA4M1_EVENT_ELC_SOFTWARE0), vec![0]);
        elc.write(
            RaElcRegister::Elcr.offset(),
            AccessWidth::Byte,
            0,
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.enabled());
        assert!(handle.route_event(RA4M1_EVENT_ELC_SOFTWARE0).is_empty());
    }
}
