use super::SignalHub;
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{SignalError, SignalId, SignalValue};
use std::sync::{Arc, Mutex};

fn read_le(bytes: &[u8], offset: usize, width: AccessWidth) -> Result<u64, DeviceError> {
    let end = offset
        .checked_add(usize::from(width.bytes()))
        .ok_or_else(|| DeviceError::new("timer register access overflow"))?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| DeviceError::new("timer register access exceeds register map"))?;
    Ok(slice.iter().enumerate().fold(0, |value, (shift, byte)| {
        value | (u64::from(*byte) << (shift * 8))
    }))
}

fn write_le(
    bytes: &mut [u8],
    offset: usize,
    width: AccessWidth,
    value: u64,
) -> Result<(), DeviceError> {
    let end = offset
        .checked_add(usize::from(width.bytes()))
        .ok_or_else(|| DeviceError::new("timer register access overflow"))?;
    let slice = bytes
        .get_mut(offset..end)
        .ok_or_else(|| DeviceError::new("timer register access exceeds register map"))?;
    for (shift, byte) in slice.iter_mut().enumerate() {
        *byte = (value >> (shift * 8)) as u8;
    }
    Ok(())
}

const TCC_VALUE_MASK: u32 = 0x00ff_ffff;
const TCC_INTERRUPT_MASK: u32 = 0x000f_fc0f;
const TCC_CTRLA_MASK: u32 = 0x0f00_7f63;
const TCC_OVF: u32 = 1;
const TCC_MC0: u32 = 1 << 16;

/// Native SAM D21 TCC register identifiers used by the functional model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Samd21TccRegister {
    /// Control A.
    Ctrla,
    /// Control B clear alias.
    Ctrlbclr,
    /// Control B set alias.
    Ctrlbset,
    /// Synchronization status.
    Syncbusy,
    /// Interrupt enable clear alias.
    Intenclr,
    /// Interrupt enable set alias.
    Intenset,
    /// Interrupt flags.
    Intflag,
    /// Counter value.
    Count,
    /// Period value.
    Per,
    /// Compare/capture channel 0 through 3.
    Cc(usize),
    /// Buffered period value.
    Perb,
    /// Buffered compare/capture channel 0 through 3.
    Ccb(usize),
}

impl Samd21TccRegister {
    /// Decodes a native TCC register offset.
    pub const fn from_offset(offset: usize) -> Option<Self> {
        match offset {
            0x00..=0x03 => Some(Self::Ctrla),
            0x04 => Some(Self::Ctrlbclr),
            0x05 => Some(Self::Ctrlbset),
            0x08..=0x0b => Some(Self::Syncbusy),
            0x24..=0x27 => Some(Self::Intenclr),
            0x28..=0x2b => Some(Self::Intenset),
            0x2c..=0x2f => Some(Self::Intflag),
            0x34..=0x37 => Some(Self::Count),
            0x40..=0x43 => Some(Self::Per),
            0x44..=0x53 => Some(Self::Cc((offset - 0x44) / 4)),
            0x6c..=0x6f => Some(Self::Perb),
            0x70..=0x7f => Some(Self::Ccb((offset - 0x70) / 4)),
            _ => None,
        }
    }
}

struct TccState {
    enabled: bool,
    start: u64,
    period: u32,
    compare: [u32; 4],
    interrupt_enable: u32,
    interrupt_flags: u32,
    last_cycle: u64,
    matched: u8,
}

impl Default for TccState {
    fn default() -> Self {
        Self {
            enabled: false,
            start: 0,
            period: TCC_VALUE_MASK,
            compare: [0; 4],
            interrupt_enable: 0,
            interrupt_flags: 0,
            last_cycle: 0,
            matched: 0,
        }
    }
}

fn tcc_position(state: &TccState, at: SimTime) -> (u64, u32) {
    let elapsed = at.ticks().saturating_sub(state.start);
    let modulus = u64::from(state.period) + 1;
    (elapsed / modulus, (elapsed % modulus) as u32)
}

fn read_u32_lane(
    value: u32,
    base: usize,
    offset: usize,
    width: AccessWidth,
) -> Result<u64, DeviceError> {
    let lane = offset
        .checked_sub(base)
        .ok_or_else(|| DeviceError::new("register lane underflow"))?;
    let bytes = usize::from(width.bytes());
    if lane + bytes > 4 {
        return Err(DeviceError::new("register access crosses native boundary"));
    }
    let mask = if bytes == 4 {
        u32::MAX
    } else {
        (1_u32 << (bytes * 8)) - 1
    };
    Ok(u64::from((value >> (lane * 8)) & mask))
}

fn merge_u32_lane(
    current: u32,
    base: usize,
    offset: usize,
    width: AccessWidth,
    value: u64,
) -> Result<u32, DeviceError> {
    let lane = offset
        .checked_sub(base)
        .ok_or_else(|| DeviceError::new("register lane underflow"))?;
    let bytes = usize::from(width.bytes());
    if lane + bytes > 4 {
        return Err(DeviceError::new("register access crosses native boundary"));
    }
    let lane_mask = if bytes == 4 {
        u32::MAX
    } else {
        (1_u32 << (bytes * 8)) - 1
    };
    let shift = lane * 8;
    let mask = lane_mask << shift;
    Ok((current & !mask) | (((value as u32) & lane_mask) << shift))
}

/// Host-facing state for one SAM D21 TCC instance.
#[derive(Clone)]
pub struct Samd21TccHandle {
    state: Arc<Mutex<TccState>>,
    outputs: Option<(SignalHub, [SignalId; 4])>,
}

impl Samd21TccHandle {
    /// Advances the counter, updates deterministic PWM observations, and returns the IRQ level.
    pub fn poll(&self, at: SimTime) -> Result<bool, SignalError> {
        let (pending, levels) = {
            let mut state = self.state.lock().expect("TCC lock poisoned");
            if !state.enabled {
                (
                    state.interrupt_enable & state.interrupt_flags != 0,
                    [false; 4],
                )
            } else {
                let (cycle, count) = tcc_position(&state, at);
                if cycle > state.last_cycle {
                    state.interrupt_flags |= TCC_OVF;
                    state.matched = 0;
                    state.last_cycle = cycle;
                }
                let mut levels = [false; 4];
                for (channel, compare) in state.compare.into_iter().enumerate() {
                    levels[channel] = compare != 0 && count < compare;
                    if compare != 0 && count >= compare && state.matched & (1 << channel) == 0 {
                        state.interrupt_flags |= TCC_MC0 << channel;
                        state.matched |= 1 << channel;
                    }
                }
                (state.interrupt_enable & state.interrupt_flags != 0, levels)
            }
        };
        if let Some((hub, outputs)) = &self.outputs {
            for (signal, level) in outputs.iter().zip(levels) {
                hub.set(*signal, SignalValue::from_u64(u64::from(level), 1)?, at)?;
            }
        }
        Ok(pending)
    }

    /// Returns the current counter value.
    pub fn count(&self, at: SimTime) -> u32 {
        let state = self.state.lock().expect("TCC lock poisoned");
        if state.enabled {
            tcc_position(&state, at).1
        } else {
            0
        }
    }

    /// Returns the latched interrupt flags.
    pub fn interrupt_flags(&self) -> u32 {
        self.state
            .lock()
            .expect("TCC lock poisoned")
            .interrupt_flags
    }
}

/// Functional SAM D21 TCC counter, compare/PWM, buffer, and interrupt slice.
pub struct Samd21Tcc {
    name: String,
    state: Arc<Mutex<TccState>>,
    registers: [u8; 0x80],
    channel_mask: u32,
}

impl Samd21Tcc {
    /// Constructs a TCC without waveform signals.
    pub fn new(name: impl Into<String>, channels: usize) -> (Self, Samd21TccHandle) {
        Self::new_inner(name.into(), channels, None)
    }

    /// Constructs a TCC with four one-bit digital waveform observations.
    pub fn new_with_signals(
        name: impl Into<String>,
        channels: usize,
        hub: SignalHub,
        path: impl Into<String>,
    ) -> Result<(Self, Samd21TccHandle), SignalError> {
        let path = path.into();
        let mut ids = Vec::with_capacity(4);
        for channel in 0..4 {
            ids.push(hub.declare(
                format!("{path}.wo{channel}"),
                SignalValue::from_u64(0, 1)?,
                Some("deterministic TCC PWM level".to_owned()),
            )?);
        }
        let outputs: [SignalId; 4] = ids.try_into().expect("four TCC outputs");
        Ok(Self::new_inner(name.into(), channels, Some((hub, outputs))))
    }

    fn new_inner(
        name: String,
        channels: usize,
        outputs: Option<(SignalHub, [SignalId; 4])>,
    ) -> (Self, Samd21TccHandle) {
        let state = Arc::new(Mutex::new(TccState::default()));
        let channel_mask = if channels >= 4 {
            0x000f_0000
        } else {
            ((1_u32 << channels) - 1) << 16
        };
        (
            Self {
                name,
                state: state.clone(),
                registers: [0; 0x80],
                channel_mask,
            },
            Samd21TccHandle { state, outputs },
        )
    }

    fn reset_state(&mut self) {
        *self.state.lock().expect("TCC lock poisoned") = TccState::default();
        self.registers = [0; 0x80];
    }
}

impl Device for Samd21Tcc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let offset =
            usize::try_from(offset).map_err(|_| DeviceError::new("TCC offset overflow"))?;
        let register = Samd21TccRegister::from_offset(offset);
        let state = self.state.lock().expect("TCC lock poisoned");
        match register {
            Some(Samd21TccRegister::Ctrla) => read_le(&self.registers, offset, width),
            Some(Samd21TccRegister::Intenclr | Samd21TccRegister::Intenset) => {
                let base = if offset < 0x28 { 0x24 } else { 0x28 };
                read_u32_lane(state.interrupt_enable, base, offset, width)
            }
            Some(Samd21TccRegister::Intflag) => {
                read_u32_lane(state.interrupt_flags, 0x2c, offset, width)
            }
            Some(Samd21TccRegister::Count) => read_u32_lane(
                if state.enabled {
                    tcc_position(&state, at).1
                } else {
                    0
                },
                0x34,
                offset,
                width,
            ),
            Some(Samd21TccRegister::Per) => read_u32_lane(state.period, 0x40, offset, width),
            Some(Samd21TccRegister::Cc(channel)) => {
                read_u32_lane(state.compare[channel], 0x44 + channel * 4, offset, width)
            }
            Some(Samd21TccRegister::Syncbusy) => Ok(0),
            _ => read_le(&self.registers, offset, width),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let offset =
            usize::try_from(offset).map_err(|_| DeviceError::new("TCC offset overflow"))?;
        match Samd21TccRegister::from_offset(offset) {
            Some(Samd21TccRegister::Ctrla) => {
                write_le(&mut self.registers, offset, width, value)?;
                let control =
                    read_le(&self.registers, 0, AccessWidth::Word)? as u32 & TCC_CTRLA_MASK;
                if control & 1 != 0 {
                    self.reset_state();
                } else {
                    let mut state = self.state.lock().expect("TCC lock poisoned");
                    let enabled = control & 2 != 0;
                    if enabled && !state.enabled {
                        state.start = at.ticks();
                        state.last_cycle = 0;
                        state.matched = 0;
                    }
                    state.enabled = enabled;
                    drop(state);
                    self.registers[0..4].copy_from_slice(&control.to_le_bytes());
                }
            }
            Some(Samd21TccRegister::Intenclr) => {
                let payload = merge_u32_lane(0, 0x24, offset, width, value)?;
                self.state
                    .lock()
                    .expect("TCC lock poisoned")
                    .interrupt_enable &=
                    !(payload & (TCC_INTERRUPT_MASK & (self.channel_mask | 0x0000_fc0f)));
            }
            Some(Samd21TccRegister::Intenset) => {
                let payload = merge_u32_lane(0, 0x28, offset, width, value)?;
                self.state
                    .lock()
                    .expect("TCC lock poisoned")
                    .interrupt_enable |=
                    payload & (TCC_INTERRUPT_MASK & (self.channel_mask | 0x0000_fc0f));
            }
            Some(Samd21TccRegister::Intflag) => {
                let payload = merge_u32_lane(0, 0x2c, offset, width, value)?;
                self.state
                    .lock()
                    .expect("TCC lock poisoned")
                    .interrupt_flags &= !(payload & TCC_INTERRUPT_MASK);
            }
            Some(Samd21TccRegister::Count) => {
                let mut state = self.state.lock().expect("TCC lock poisoned");
                let current = if state.enabled {
                    tcc_position(&state, at).1
                } else {
                    0
                };
                let count = merge_u32_lane(current, 0x34, offset, width, value)? & TCC_VALUE_MASK;
                state.start = at.ticks().saturating_sub(u64::from(count));
                state.last_cycle = 0;
                state.matched = 0;
            }
            Some(Samd21TccRegister::Per) => {
                let mut state = self.state.lock().expect("TCC lock poisoned");
                state.period =
                    merge_u32_lane(state.period, 0x40, offset, width, value)? & TCC_VALUE_MASK;
            }
            Some(Samd21TccRegister::Cc(channel)) => {
                if self.channel_mask & (TCC_MC0 << channel) != 0 {
                    let mut state = self.state.lock().expect("TCC lock poisoned");
                    state.compare[channel] = merge_u32_lane(
                        state.compare[channel],
                        0x44 + channel * 4,
                        offset,
                        width,
                        value,
                    )? & TCC_VALUE_MASK;
                }
            }
            Some(Samd21TccRegister::Syncbusy) => {}
            _ => write_le(&mut self.registers, offset, width, value)?,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset_state();
    }
}

const RTC_INTERRUPT_MASK: u8 = 0xc1;

/// Native SAM D21 RTC MODE0 register identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Samd21RtcRegister {
    /// Control register.
    Ctrl,
    /// Read request.
    Readreq,
    /// Event control.
    Evctrl,
    /// Interrupt enable clear.
    Intenclr,
    /// Interrupt enable set.
    Intenset,
    /// Interrupt flags.
    Intflag,
    /// Synchronization status.
    Status,
    /// Debug control.
    Dbgctrl,
    /// Frequency correction.
    Freqcorr,
    /// 32-bit counter.
    Count,
    /// 32-bit comparison value.
    Comp0,
}

impl Samd21RtcRegister {
    /// Decodes a native RTC MODE0 register offset.
    pub const fn from_offset(offset: usize) -> Option<Self> {
        match offset {
            0x00 | 0x01 => Some(Self::Ctrl),
            0x02 | 0x03 => Some(Self::Readreq),
            0x04 | 0x05 => Some(Self::Evctrl),
            0x06 => Some(Self::Intenclr),
            0x07 => Some(Self::Intenset),
            0x08 => Some(Self::Intflag),
            0x0a => Some(Self::Status),
            0x0b => Some(Self::Dbgctrl),
            0x0c => Some(Self::Freqcorr),
            0x10..=0x13 => Some(Self::Count),
            0x18..=0x1b => Some(Self::Comp0),
            _ => None,
        }
    }
}

#[derive(Default)]
struct RtcState {
    control: u16,
    start: u64,
    count: u32,
    compare: u32,
    interrupt_enable: u8,
    interrupt_flags: u8,
    matched: bool,
}

/// Host-facing state for the SAM D21 RTC MODE0 counter.
#[derive(Clone)]
pub struct Samd21RtcHandle(Arc<Mutex<RtcState>>);

impl Samd21RtcHandle {
    /// Advances MODE0 and returns the IRQ 3 request level.
    pub fn poll(&self, at: SimTime) -> bool {
        let mut state = self.0.lock().expect("RTC lock poisoned");
        if state.control & 2 != 0 {
            let elapsed = at.ticks().saturating_sub(state.start);
            let count = state.count.wrapping_add(elapsed as u32);
            if !state.matched && state.compare != 0 && count >= state.compare {
                state.interrupt_flags |= 1;
                state.matched = true;
            }
            if elapsed > u64::from(u32::MAX - state.count) {
                state.interrupt_flags |= 1 << 7;
            }
        }
        state.interrupt_enable & state.interrupt_flags != 0
    }

    /// Returns the deterministic MODE0 count.
    pub fn count(&self, at: SimTime) -> u32 {
        let state = self.0.lock().expect("RTC lock poisoned");
        if state.control & 2 != 0 {
            state
                .count
                .wrapping_add(at.ticks().saturating_sub(state.start) as u32)
        } else {
            state.count
        }
    }
}

/// Functional SAM D21 RTC MODE0 count/compare/interrupt slice.
pub struct Samd21Rtc {
    name: String,
    state: Arc<Mutex<RtcState>>,
    registers: [u8; 0x20],
}

impl Samd21Rtc {
    /// Constructs the RTC and its IRQ handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21RtcHandle) {
        let state = Arc::new(Mutex::new(RtcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 0x20],
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
        let offset =
            usize::try_from(offset).map_err(|_| DeviceError::new("RTC offset overflow"))?;
        let state = self.state.lock().expect("RTC lock poisoned");
        match Samd21RtcRegister::from_offset(offset) {
            Some(Samd21RtcRegister::Ctrl) => {
                read_u32_lane(u32::from(state.control), 0, offset, width)
            }
            Some(Samd21RtcRegister::Intenclr | Samd21RtcRegister::Intenset) => {
                Ok(u64::from(state.interrupt_enable))
            }
            Some(Samd21RtcRegister::Intflag) => Ok(u64::from(state.interrupt_flags)),
            Some(Samd21RtcRegister::Status) => Ok(0),
            Some(Samd21RtcRegister::Count) => read_u32_lane(
                if state.control & 2 != 0 {
                    state
                        .count
                        .wrapping_add(at.ticks().saturating_sub(state.start) as u32)
                } else {
                    state.count
                },
                0x10,
                offset,
                width,
            ),
            Some(Samd21RtcRegister::Comp0) => read_u32_lane(state.compare, 0x18, offset, width),
            _ => read_le(&self.registers, offset, width),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let offset =
            usize::try_from(offset).map_err(|_| DeviceError::new("RTC offset overflow"))?;
        match Samd21RtcRegister::from_offset(offset) {
            Some(Samd21RtcRegister::Ctrl) => {
                let current = u32::from(self.state.lock().expect("RTC lock poisoned").control);
                let control = merge_u32_lane(current, 0, offset, width, value)? as u16 & 0x0f83;
                if control & 1 != 0 {
                    *self.state.lock().expect("RTC lock poisoned") = RtcState::default();
                    self.registers = [0; 0x20];
                } else {
                    let mut state = self.state.lock().expect("RTC lock poisoned");
                    state.control = control & 0x0f82;
                    state.start = at.ticks();
                    state.matched = false;
                }
            }
            Some(Samd21RtcRegister::Intenclr) => {
                self.state
                    .lock()
                    .expect("RTC lock poisoned")
                    .interrupt_enable &= !(value as u8 & RTC_INTERRUPT_MASK);
            }
            Some(Samd21RtcRegister::Intenset) => {
                self.state
                    .lock()
                    .expect("RTC lock poisoned")
                    .interrupt_enable |= value as u8 & RTC_INTERRUPT_MASK;
            }
            Some(Samd21RtcRegister::Intflag) => {
                self.state
                    .lock()
                    .expect("RTC lock poisoned")
                    .interrupt_flags &= !(value as u8 & RTC_INTERRUPT_MASK);
            }
            Some(Samd21RtcRegister::Status) => {}
            Some(Samd21RtcRegister::Count) => {
                let mut state = self.state.lock().expect("RTC lock poisoned");
                let current = if state.control & 2 != 0 {
                    state
                        .count
                        .wrapping_add(at.ticks().saturating_sub(state.start) as u32)
                } else {
                    state.count
                };
                state.count = merge_u32_lane(current, 0x10, offset, width, value)?;
                state.start = at.ticks();
                state.matched = false;
            }
            Some(Samd21RtcRegister::Comp0) => {
                let mut state = self.state.lock().expect("RTC lock poisoned");
                state.compare = merge_u32_lane(state.compare, 0x18, offset, width, value)?;
            }
            _ => write_le(&mut self.registers, offset, width, value)?,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RTC lock poisoned") = RtcState::default();
        self.registers = [0; 0x20];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcc_period_compare_pwm_and_w1_interrupts_are_deterministic() {
        let hub = SignalHub::new();
        let (mut tcc, handle) = Samd21Tcc::new_with_signals("tcc0", 4, hub, "board.tcc0").unwrap();
        tcc.write(0x40, AccessWidth::Word, 9, SimTime::ZERO)
            .unwrap();
        tcc.write(0x44, AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        tcc.write(
            0x28,
            AccessWidth::Word,
            u64::from(TCC_OVF | TCC_MC0),
            SimTime::ZERO,
        )
        .unwrap();
        tcc.write(0x00, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(3)).unwrap());
        assert!(handle.poll(SimTime::from_ticks(4)).unwrap());
        assert_eq!(handle.interrupt_flags() & TCC_MC0, TCC_MC0);
        tcc.write(
            0x2c,
            AccessWidth::Word,
            u64::from(TCC_MC0),
            SimTime::from_ticks(4),
        )
        .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(4)).unwrap());
        assert!(handle.poll(SimTime::from_ticks(10)).unwrap());
        assert_eq!(handle.interrupt_flags() & TCC_OVF, TCC_OVF);
    }

    #[test]
    fn tcc_channel_mask_rejects_unavailable_compare_interrupts() {
        let (mut tcc, handle) = Samd21Tcc::new("tcc2", 2);
        tcc.write(
            0x28,
            AccessWidth::Word,
            u64::from(TCC_MC0 << 3),
            SimTime::ZERO,
        )
        .unwrap();
        tcc.write(0x50, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        tcc.write(0x00, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(2)).unwrap());
        assert_eq!(handle.interrupt_flags(), 0);
    }

    #[test]
    fn rtc_mode0_compare_interrupt_and_w1c_follow_native_offsets() {
        let (mut rtc, handle) = Samd21Rtc::new("rtc");
        rtc.write(0x18, AccessWidth::Word, 5, SimTime::ZERO)
            .unwrap();
        rtc.write(0x07, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        rtc.write(0x00, AccessWidth::HalfWord, 2, SimTime::ZERO)
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(4)));
        assert!(handle.poll(SimTime::from_ticks(5)));
        assert_eq!(
            rtc.read(0x08, AccessWidth::Byte, SimTime::from_ticks(5))
                .unwrap(),
            1
        );
        rtc.write(0x08, AccessWidth::Byte, 1, SimTime::from_ticks(5))
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(5)));
    }

    #[test]
    fn timer_register_ids_and_native_byte_lanes_are_preserved() {
        assert_eq!(
            Samd21TccRegister::from_offset(0x4b),
            Some(Samd21TccRegister::Cc(1))
        );
        assert_eq!(
            Samd21RtcRegister::from_offset(0x1a),
            Some(Samd21RtcRegister::Comp0)
        );

        let (mut tcc, handle) = Samd21Tcc::new("tcc0", 4);
        tcc.write(0x40, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        tcc.write(0x41, AccessWidth::Byte, 0x12, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            tcc.read(0x40, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0x1200
        );
        tcc.write(0x44, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        tcc.write(0x2a, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        tcc.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(2)).unwrap());

        let (mut rtc, _) = Samd21Rtc::new("rtc");
        rtc.write(0x18, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        rtc.write(0x19, AccessWidth::Byte, 0x34, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rtc.read(0x18, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0x3400
        );
        rtc.write(0x01, AccessWidth::Byte, 0x0f, SimTime::ZERO)
            .unwrap();
        rtc.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rtc.read(0x00, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0x0f02
        );
    }
}
