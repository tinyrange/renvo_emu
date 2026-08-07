use super::*;

const CHANNEL_COUNT: usize = 6;
const TIMER_COUNT: usize = 4;
const CHANNEL_STRIDE: u64 = 0x14;
const TIMER_BASE: u64 = 0xa0;
const TIMER_STRIDE: u64 = 0x08;
const INT_RAW: u64 = 0xc0;
const INT_STATUS: u64 = 0xc4;
const INT_ENABLE: u64 = 0xc8;
const INT_CLEAR: u64 = 0xcc;

#[derive(Clone, Copy)]
struct TimerState {
    configuration: u32,
    base_counter: u64,
    base_time: SimTime,
}

impl Default for TimerState {
    fn default() -> Self {
        Self {
            configuration: 1 << 24,
            base_counter: 0,
            base_time: SimTime::ZERO,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ChannelState {
    configuration: u32,
    hpoint: u32,
    duty: u32,
    duty_readback: u32,
    started: bool,
}

struct EspLedcState {
    registers: Vec<u32>,
    timers: [TimerState; TIMER_COUNT],
    channels: [ChannelState; CHANNEL_COUNT],
    outputs: [Logic; CHANNEL_COUNT],
}

impl EspLedcState {
    fn new() -> Self {
        let mut state = Self {
            registers: vec![0; 0x200 / 4],
            timers: [TimerState::default(); TIMER_COUNT],
            channels: [ChannelState::default(); CHANNEL_COUNT],
            outputs: [Logic::Zero; CHANNEL_COUNT],
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.timers = [TimerState::default(); TIMER_COUNT];
        self.channels = [ChannelState::default(); CHANNEL_COUNT];
        self.outputs = [Logic::Zero; CHANNEL_COUNT];
        for timer in 0..TIMER_COUNT {
            self.registers[(TIMER_BASE as usize / 4) + timer * 2] = 1 << 24;
        }
    }

    fn timer_counter(&self, timer: usize, at: SimTime) -> u32 {
        let state = self.timers[timer];
        let configuration = state.configuration;
        if configuration & ((1 << 23) | (1 << 24)) != 0 {
            return state.base_counter as u32 & 0x000f_ffff;
        }
        let divider = u64::from((configuration >> 5) & 0x0003_ffff).max(1);
        let elapsed = at.ticks().saturating_sub(state.base_time.ticks());
        let increments = elapsed.saturating_mul(256) / divider;
        state.base_counter.wrapping_add(increments) as u32 & 0x000f_ffff
    }

    fn duty_resolution(&self, timer: usize) -> u32 {
        (self.timers[timer].configuration & 0x1f).min(20)
    }

    fn output_for(&self, channel: usize, at: SimTime) -> Logic {
        let channel_state = self.channels[channel];
        let enabled = channel_state.configuration & (1 << 2) != 0 && channel_state.started;
        if !enabled {
            return if channel_state.configuration & (1 << 3) != 0 {
                Logic::One
            } else {
                Logic::Zero
            };
        }
        let timer = (channel_state.configuration & 3) as usize;
        let resolution = self.duty_resolution(timer);
        let period = 1_u32 << resolution;
        let duty = channel_state.duty.min(period);
        if duty == 0 {
            return Logic::Zero;
        }
        if duty >= period {
            return Logic::One;
        }
        let counter = self.timer_counter(timer, at) % period;
        let hpoint = channel_state.hpoint % period;
        let end = hpoint.saturating_add(duty);
        let high = if end <= period {
            counter >= hpoint && counter < end
        } else {
            counter >= hpoint || counter < end - period
        };
        if high { Logic::One } else { Logic::Zero }
    }

    fn output_values(&self, at: SimTime) -> [Logic; CHANNEL_COUNT] {
        core::array::from_fn(|channel| self.output_for(channel, at))
    }
}

/// Host-facing observation handle for ESP32-C6 LEDC channels.
#[derive(Clone)]
pub struct EspLedcHandle {
    state: Rc<RefCell<EspLedcState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl EspLedcHandle {
    /// Updates the six channel outputs at an abstract simulation time.
    ///
    /// The model intentionally uses abstract ticks rather than APB frequency;
    /// this preserves deterministic PWM edges without claiming clock accuracy.
    pub fn poll(&self, at: SimTime) -> Result<u8, DeviceError> {
        let values = self.state.borrow().output_values(at);
        let mut changed = 0;
        let mut transitions = Vec::new();
        {
            let mut state = self.state.borrow_mut();
            for (channel, value) in values.into_iter().enumerate() {
                if state.outputs[channel] != value {
                    state.outputs[channel] = value;
                    transitions.push((channel, value));
                    changed += 1;
                }
            }
        }
        for (channel, value) in transitions {
            self.hub
                .set(
                    self.signals[channel],
                    SignalValue::repeat(value, 1)
                        .expect("one-bit LEDC signal construction cannot fail"),
                    at,
                )
                .map_err(|error| DeviceError::new(error.to_string()))?;
        }
        Ok(changed)
    }

    /// Returns the latest resolved output value for one channel.
    pub fn output(&self, channel: usize) -> Result<Logic, DeviceError> {
        self.state
            .borrow()
            .outputs
            .get(channel)
            .copied()
            .ok_or_else(|| DeviceError::new(format!("LEDC channel {channel} is out of range")))
    }
}

/// Functional ESP32-C6 LED PWM controller.
///
/// The model covers the six low-speed channels, four timer configuration/value
/// pairs, duty/hpoint shadow registers, output-enable/start sequencing, and
/// deterministic channel waveforms. Fade, gamma, event-task, and interrupt
/// overflow details remain intentionally outside this functional slice.
pub struct EspLedc {
    name: String,
    state: Rc<RefCell<EspLedcState>>,
    handle: EspLedcHandle,
}

impl EspLedc {
    /// Creates an ESP32-C6 LEDC block and channel observation handle.
    pub fn new(
        name: impl Into<String>,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, EspLedcHandle), SignalError> {
        let mut signals = Vec::with_capacity(CHANNEL_COUNT);
        for channel in 0..CHANNEL_COUNT {
            signals.push(hub.declare(
                format!("{path}.ch{channel}"),
                SignalValue::repeat(Logic::Zero, 1)?,
                Some(format!("LEDC channel {channel} output")),
            )?);
        }
        let state = Rc::new(RefCell::new(EspLedcState::new()));
        let handle = EspLedcHandle {
            state: state.clone(),
            signals,
            hub,
        };
        Ok((
            Self {
                name: name.into(),
                state,
                handle: handle.clone(),
            },
            handle,
        ))
    }

    fn channel_register(offset: u64) -> Option<(usize, u64)> {
        if offset >= CHANNEL_STRIDE * CHANNEL_COUNT as u64 {
            return None;
        }
        let channel = usize::try_from(offset / CHANNEL_STRIDE).ok()?;
        Some((channel, offset % CHANNEL_STRIDE))
    }

    fn timer_register(offset: u64) -> Option<(usize, u64)> {
        if !(TIMER_BASE..TIMER_BASE + TIMER_STRIDE * TIMER_COUNT as u64).contains(&offset) {
            return None;
        }
        let relative = offset - TIMER_BASE;
        Some((
            usize::try_from(relative / TIMER_STRIDE).ok()?,
            relative % TIMER_STRIDE,
        ))
    }

    fn update_outputs(&self, at: SimTime) -> Result<(), DeviceError> {
        self.handle.poll(at).map(|_| ())
    }
}

impl Device for EspLedc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP LEDC requires aligned word access"));
        }
        let state = self.state.borrow();
        let value = if let Some((channel, register)) = Self::channel_register(offset) {
            let channel_state = state.channels[channel];
            match register {
                0x00 => channel_state.configuration,
                0x04 => channel_state.hpoint,
                0x08 => channel_state.duty,
                0x0c => u32::from(channel_state.started) << 31,
                0x10 => channel_state.duty_readback,
                _ => unreachable!(),
            }
        } else if let Some((timer, register)) = Self::timer_register(offset) {
            match register {
                0x00 => state.timers[timer].configuration,
                0x04 => state.timer_counter(timer, at),
                _ => unreachable!(),
            }
        } else {
            let index = usize::try_from(offset / 4).expect("LEDC offset fits");
            match offset {
                INT_STATUS => state.registers[index] & state.registers[INT_ENABLE as usize / 4],
                _ => *state.registers.get(index).ok_or_else(|| {
                    DeviceError::new(format!("{} read at {offset:#x}", self.name))
                })?,
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP LEDC requires aligned word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits");
        let mut state = self.state.borrow_mut();
        if let Some((channel, register)) = Self::channel_register(offset) {
            let channel_state = &mut state.channels[channel];
            match register {
                0x00 => channel_state.configuration = value & 0x0001_ffff,
                0x04 => channel_state.hpoint = value & 0x000f_ffff,
                0x08 => channel_state.duty = value & 0x01ff_ffff,
                0x0c => {
                    channel_state.started = value & (1 << 31) != 0;
                    if channel_state.started {
                        channel_state.duty_readback = channel_state.duty;
                    }
                }
                0x10 => {}
                _ => unreachable!(),
            }
            state.registers[(offset / 4) as usize] = value;
        } else if let Some((timer, register)) = Self::timer_register(offset) {
            if register == 0 {
                let current = state.timer_counter(timer, at);
                let timer_state = &mut state.timers[timer];
                timer_state.base_counter = if value & (1 << 24) != 0 {
                    0
                } else {
                    u64::from(current)
                };
                timer_state.base_time = at;
                timer_state.configuration = value & !(1 << 26);
            }
            state.registers[(offset / 4) as usize] = value;
        } else {
            let index = usize::try_from(offset / 4).expect("LEDC offset fits");
            if index >= state.registers.len() {
                return Err(DeviceError::new(format!(
                    "{} write at {offset:#x}",
                    self.name
                )));
            }
            match offset {
                INT_CLEAR => {
                    state.registers[INT_RAW as usize / 4] &= !value;
                    state.registers[index] = 0;
                }
                INT_STATUS | INT_RAW => {}
                _ => state.registers[index] = value,
            }
        }
        drop(state);
        self.update_outputs(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
        let _ = self.handle.poll(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledc_channel_generates_deterministic_half_duty_waveform() {
        let hub = SignalHub::new();
        let (mut ledc, handle) = EspLedc::new("ledc", "board.ledc", hub.clone()).unwrap();
        // Timer 0: 3-bit period, divider 1 (Q8), running.
        ledc.write(0xa0, AccessWidth::Word, 3 | (256 << 5), SimTime::ZERO)
            .unwrap();
        ledc.write(0x04, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        ledc.write(0x08, AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        ledc.write(0x00, AccessWidth::Word, 1 << 2, SimTime::ZERO)
            .unwrap();
        ledc.write(0x0c, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();

        assert_eq!(handle.output(0).unwrap(), Logic::One);
        handle.poll(SimTime::from_ticks(4)).unwrap();
        assert_eq!(handle.output(0).unwrap(), Logic::Zero);
        handle.poll(SimTime::from_ticks(8)).unwrap();
        assert_eq!(handle.output(0).unwrap(), Logic::One);
        assert!(hub.drain_changes().len() >= 2);
    }

    #[test]
    fn ledc_reads_timer_and_duty_readback_registers() {
        let hub = SignalHub::new();
        let (mut ledc, _) = EspLedc::new("ledc", "board.ledc", hub).unwrap();
        ledc.write(0xa0, AccessWidth::Word, 4 | (256 << 5), SimTime::ZERO)
            .unwrap();
        ledc.write(0x08, AccessWidth::Word, 7, SimTime::ZERO)
            .unwrap();
        ledc.write(0x0c, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            ledc.read(0x10, AccessWidth::Word, SimTime::ZERO).unwrap(),
            7
        );
        assert_eq!(
            ledc.read(0xa4, AccessWidth::Word, SimTime::from_ticks(2))
                .unwrap(),
            2
        );
    }
}
