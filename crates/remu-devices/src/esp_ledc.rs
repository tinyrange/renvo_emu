use super::*;

const CHANNELS: usize = 8;
const TIMERS: usize = 4;
const CHANNEL_STRIDE: u64 = 0x14;
const TIMER_BASE: u64 = 0xa0;
const TIMER_STRIDE: u64 = 0x08;
const INT_RAW: u64 = 0xc0;
const INT_ST: u64 = 0xc4;
const INT_ENA: u64 = 0xc8;
const INT_CLR: u64 = 0xcc;
const GLOBAL_CONF: u64 = 0xd0;
const DATE: u64 = 0xfc;

#[derive(Clone, Copy)]
struct LedcChannel {
    conf0: u32,
    hpoint: u32,
    duty: u32,
    conf1: u32,
    duty_r: u32,
}

impl LedcChannel {
    const fn reset() -> Self {
        Self {
            conf0: 0,
            hpoint: 0,
            duty: 0,
            conf1: 1 << 30,
            duty_r: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct LedcTimer {
    conf: u32,
    value: u32,
    last_time: SimTime,
}

impl LedcTimer {
    const fn reset() -> Self {
        Self {
            conf: 0,
            value: 0,
            last_time: SimTime::ZERO,
        }
    }

    fn resolution(self) -> u32 {
        (self.conf & 0xf).clamp(1, 14)
    }

    fn period(self) -> u32 {
        1_u32 << self.resolution()
    }

    fn divider(self) -> u64 {
        u64::from((self.conf >> 4) & 0x3ffff).max(1)
    }
}

struct LedcState {
    channels: [LedcChannel; CHANNELS],
    timers: [LedcTimer; TIMERS],
    int_raw: u32,
    int_ena: u32,
    global_conf: u32,
    outputs: [bool; CHANNELS],
}

impl LedcState {
    const fn reset() -> Self {
        Self {
            channels: [LedcChannel::reset(); CHANNELS],
            timers: [LedcTimer::reset(); TIMERS],
            int_raw: 0,
            int_ena: 0,
            global_conf: 0,
            outputs: [false; CHANNELS],
        }
    }

    fn advance(&mut self, now: SimTime) {
        for (index, timer) in self.timers.iter_mut().enumerate() {
            if timer.conf & (1 << 23) != 0 {
                timer.value = 0;
                timer.last_time = now;
                continue;
            }
            let elapsed = now.ticks().saturating_sub(timer.last_time.ticks());
            if elapsed == 0 || timer.conf & (1 << 22) != 0 {
                timer.last_time = now;
                continue;
            }
            let increments = elapsed / timer.divider();
            if increments != 0 {
                let period = u64::from(timer.period());
                let total = u64::from(timer.value) + increments;
                if total >= period {
                    self.int_raw |= 1 << index;
                }
                timer.value = u32::try_from(total % period).expect("LEDC counter fits u32");
            }
            timer.last_time = now;
        }
    }

    fn channel_level(&self, channel: usize) -> bool {
        let channel_state = self.channels[channel];
        if channel_state.conf0 & (1 << 2) == 0 {
            return false;
        }
        let timer = usize::try_from(channel_state.conf0 & 3).expect("LEDC timer selector fits");
        let timer = self.timers[timer];
        let period = timer.period();
        let counter = timer.value % period;
        let hpoint = channel_state.hpoint & 0x3fff;
        let duty = channel_state.duty_r & 0x7ffff;
        let phase = (counter + period - (hpoint % period)) % period;
        phase < duty.min(period)
    }
}

/// Scheduler-facing handle for the ESP32-S3 LEDC low-speed PWM block.
#[derive(Clone)]
pub struct Esp32S3LedcHandle {
    state: Rc<RefCell<LedcState>>,
    hub: SignalHub,
    output_signals: Vec<SignalId>,
}

impl Esp32S3LedcHandle {
    /// Advances the functional PWM counters and publishes changed outputs.
    pub fn poll(&self, now: SimTime) -> Result<bool, SignalError> {
        let mut state = self.state.borrow_mut();
        state.advance(now);
        let mut changed = false;
        for channel in 0..CHANNELS {
            let level = state.channel_level(channel);
            if state.outputs[channel] == level {
                continue;
            }
            state.outputs[channel] = level;
            self.hub.set(
                self.output_signals[channel],
                SignalValue::from_u64(u64::from(level), 1)?,
                now,
            )?;
            changed = true;
        }
        Ok(changed)
    }

    /// Returns the current output level for a channel.
    pub fn channel_level(&self, channel: usize) -> bool {
        self.state.borrow().outputs[channel]
    }

    /// Returns a timer's current abstract counter value.
    pub fn timer_value(&self, timer: usize) -> u32 {
        self.state.borrow().timers[timer].value
    }
}

/// Functional ESP32-S3 LEDC low-speed PWM controller.
///
/// The model follows the native eight-channel/four-timer register layout. It
/// provides deterministic abstract-tick counters, duty/hpoint output levels,
/// duty-update and timer-overflow interrupt latches, and one-bit VCD signals.
/// Clock-source frequency, GPIO-matrix routing, fade hardware, high-speed
/// mode, and exact divider timing remain outside this functional slice.
pub struct Esp32S3Ledc {
    name: String,
    state: Rc<RefCell<LedcState>>,
    hub: SignalHub,
    output_signals: Vec<SignalId>,
}

impl Esp32S3Ledc {
    /// Creates an ESP32-S3 LEDC block and scheduler handle.
    pub fn new(
        name: impl Into<String>,
        signal_path: &str,
        hub: SignalHub,
    ) -> Result<(Self, Esp32S3LedcHandle), SignalError> {
        let mut output_signals = Vec::with_capacity(CHANNELS);
        for channel in 0..CHANNELS {
            output_signals.push(hub.declare(
                format!("{signal_path}.ch{channel}"),
                SignalValue::from_u64(0, 1)?,
                Some("Functional ESP32-S3 LEDC PWM output".to_owned()),
            )?);
        }
        let state = Rc::new(RefCell::new(LedcState::reset()));
        let device = Self {
            name: name.into(),
            state: state.clone(),
            hub: hub.clone(),
            output_signals: output_signals.clone(),
        };
        let handle = Esp32S3LedcHandle {
            state,
            hub,
            output_signals,
        };
        Ok((device, handle))
    }

    fn advance_and_refresh(&self, at: SimTime) -> Result<(), DeviceError> {
        let handle = Esp32S3LedcHandle {
            state: self.state.clone(),
            hub: self.hub.clone(),
            output_signals: self.output_signals.clone(),
        };
        handle
            .poll(at)
            .map(|_| ())
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn read_register(&self, offset: u64) -> Result<u32, DeviceError> {
        let state = self.state.borrow();
        if let Some(channel) = channel_register(offset) {
            let (channel, register) = channel;
            let channel_state = state.channels[channel];
            return match register {
                0x00 => Ok(channel_state.conf0),
                0x04 => Ok(channel_state.hpoint),
                0x08 => Ok(channel_state.duty),
                0x0c => Ok(channel_state.conf1),
                0x10 => Ok(channel_state.duty_r),
                _ => unreachable!(),
            };
        }
        if let Some(timer) = timer_register(offset) {
            let (timer, register) = timer;
            return match register {
                0x00 => Ok(state.timers[timer].conf),
                0x04 => Ok(state.timers[timer].value & 0x3fff),
                _ => unreachable!(),
            };
        }
        match offset {
            INT_RAW => Ok(state.int_raw),
            INT_ST => Ok(state.int_raw & state.int_ena),
            INT_ENA => Ok(state.int_ena),
            GLOBAL_CONF => Ok(state.global_conf),
            DATE => Ok(0x1904_0200),
            _ => Err(DeviceError::new(format!(
                "unmodeled ESP32-S3 LEDC read at offset {offset:#x}"
            ))),
        }
    }
}

fn channel_register(offset: u64) -> Option<(usize, u64)> {
    if offset >= TIMER_BASE {
        return None;
    }
    let channel = usize::try_from(offset / CHANNEL_STRIDE).ok()?;
    let register = offset % CHANNEL_STRIDE;
    (channel < CHANNELS && register <= 0x10 && register.is_multiple_of(4))
        .then_some((channel, register))
}

fn timer_register(offset: u64) -> Option<(usize, u64)> {
    if !(TIMER_BASE..INT_RAW).contains(&offset) {
        return None;
    }
    let timer = usize::try_from((offset - TIMER_BASE) / TIMER_STRIDE).ok()?;
    let register = (offset - TIMER_BASE) % TIMER_STRIDE;
    (timer < TIMERS && (register == 0 || register == 4)).then_some((timer, register))
}

impl Device for Esp32S3Ledc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 LEDC requires aligned word access",
            ));
        }
        self.advance_and_refresh(at)?;
        Ok(u64::from(self.read_register(offset)?))
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
                "ESP32-S3 LEDC requires aligned word access",
            ));
        }
        self.advance_and_refresh(at)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("LEDC value fits u32");
        let mut state = self.state.borrow_mut();
        if let Some((channel, register)) = channel_register(offset) {
            let channel_state = &mut state.channels[channel];
            match register {
                0x00 => channel_state.conf0 = value & 0x0003_ffff,
                0x04 => channel_state.hpoint = value & 0x3fff,
                0x08 => channel_state.duty = value & 0x7ffff,
                0x0c => {
                    channel_state.conf1 = value;
                    if value & (1 << 31) != 0 {
                        channel_state.duty_r = channel_state.duty;
                        state.int_raw |= 1 << (4 + channel);
                    }
                }
                0x10 => return Err(DeviceError::new("LEDC DUTY_R is read-only")),
                _ => unreachable!(),
            }
        } else if let Some((timer, register)) = timer_register(offset) {
            let timer_state = &mut state.timers[timer];
            match register {
                0x00 => {
                    timer_state.conf = value & 0x03ff_ffff;
                    if value & (1 << 23) != 0 {
                        timer_state.value = 0;
                    }
                    timer_state.last_time = at;
                }
                0x04 => return Err(DeviceError::new("LEDC timer VALUE is read-only")),
                _ => unreachable!(),
            }
        } else {
            match offset {
                INT_ENA => state.int_ena = value & 0x000f_ffff,
                INT_CLR => state.int_raw &= !value,
                GLOBAL_CONF => state.global_conf = value & 0x8000_0003,
                DATE => return Err(DeviceError::new("LEDC DATE is read-only")),
                INT_RAW | INT_ST => {
                    return Err(DeviceError::new("LEDC interrupt status is read-only"));
                }
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled ESP32-S3 LEDC write at offset {offset:#x}"
                    )));
                }
            }
        }
        drop(state);
        self.advance_and_refresh(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = LedcState::reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_follow_timer_duty_and_emit_waveform_changes() {
        let hub = SignalHub::new();
        let (mut ledc, handle) =
            Esp32S3Ledc::new("ledc", "board.esp32s3.ledc", hub.clone()).expect("signals are valid");
        ledc.write(TIMER_BASE, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        ledc.write(0x08, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        ledc.write(0x0c, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        ledc.write(0x00, AccessWidth::Word, 1 << 2, SimTime::ZERO)
            .unwrap();
        handle.poll(SimTime::from_ticks(1)).unwrap();
        assert!(handle.channel_level(0));
        assert_eq!(handle.timer_value(0), 1);
        assert!(handle.poll(SimTime::from_ticks(3)).unwrap());
        assert!(!handle.channel_level(0));
        let changes = hub.drain_changes();
        assert!(
            changes
                .iter()
                .any(|change| change.value.bit(0) == Some(Logic::One))
        );
        assert!(
            changes
                .iter()
                .any(|change| change.value.bit(0) == Some(Logic::Zero))
        );
    }

    #[test]
    fn duty_update_and_timer_overflow_latch_interrupts() {
        let hub = SignalHub::new();
        let (mut ledc, handle) = Esp32S3Ledc::new("ledc", "board.esp32s3.ledc", hub).unwrap();
        ledc.write(TIMER_BASE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        ledc.write(0x08, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        ledc.write(0x0c, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        ledc.write(0xc8, AccessWidth::Word, 1 | (1 << 4), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            ledc.read(0xc0, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 4
        );
        handle.poll(SimTime::from_ticks(2)).unwrap();
        assert_ne!(
            ledc.read(0xc0, AccessWidth::Word, SimTime::from_ticks(2))
                .unwrap()
                & 1,
            0
        );
        ledc.write(0xcc, AccessWidth::Word, u64::MAX, SimTime::from_ticks(2))
            .unwrap();
        assert_eq!(
            ledc.read(0xc0, AccessWidth::Word, SimTime::from_ticks(2))
                .unwrap(),
            0
        );
    }
}
