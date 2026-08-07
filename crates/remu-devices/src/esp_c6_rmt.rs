use super::*;

const CHANNEL_COUNT: usize = 4;
const INT_RAW: u64 = 0x38;
const INT_STATUS: u64 = 0x3c;
const INT_ENABLE: u64 = 0x40;
const INT_CLEAR: u64 = 0x44;

#[derive(Default)]
struct RmtChannel {
    configuration: u32,
    fifo: Vec<u32>,
    transmission: Vec<u32>,
    started_at: SimTime,
    active: bool,
}

struct EspRmtState {
    registers: Vec<u32>,
    channels: [RmtChannel; CHANNEL_COUNT],
    outputs: [Logic; CHANNEL_COUNT],
}

impl EspRmtState {
    fn new() -> Self {
        let mut state = Self {
            registers: vec![0; 0x100 / 4],
            channels: core::array::from_fn(|_| RmtChannel::default()),
            outputs: [Logic::Z; CHANNEL_COUNT],
        };
        state.registers[0xcc / 4] = 34_636_307;
        state
    }

    fn idle_level(channel: &RmtChannel) -> Logic {
        if channel.configuration & (1 << 6) == 0 {
            Logic::Z
        } else if channel.configuration & (1 << 5) == 0 {
            Logic::Zero
        } else {
            Logic::One
        }
    }

    fn channel_output(channel: &mut RmtChannel, at: SimTime) -> Logic {
        if !channel.active {
            return Self::idle_level(channel);
        }
        let divider = u64::from((channel.configuration >> 8) as u8).max(1);
        let mut remaining = at.ticks().saturating_sub(channel.started_at.ticks());
        let mut total = 0_u64;
        for word in &channel.transmission {
            total = total
                .saturating_add(u64::from(word & 0x7fff).saturating_mul(divider))
                .saturating_add(u64::from((word >> 16) & 0x7fff).saturating_mul(divider));
        }
        if total == 0 {
            channel.active = false;
            channel.transmission.clear();
            return Self::idle_level(channel);
        }
        if channel.configuration & (1 << 3) != 0 {
            remaining %= total;
        } else if remaining >= total {
            channel.active = false;
            channel.transmission.clear();
            return Self::idle_level(channel);
        }
        for word in &channel.transmission {
            for (duration, level) in [
                (word & 0x7fff, (word >> 15) & 1),
                ((word >> 16) & 0x7fff, word >> 31),
            ] {
                let span = u64::from(duration).saturating_mul(divider);
                if span == 0 {
                    continue;
                }
                if remaining < span {
                    return if level == 0 { Logic::Zero } else { Logic::One };
                }
                remaining -= span;
            }
        }
        channel.active = false;
        channel.transmission.clear();
        Self::idle_level(channel)
    }
}

/// Host-facing observation handle for ESP32-C6 RMT outputs.
#[derive(Clone)]
pub struct EspRmtHandle {
    state: Rc<RefCell<EspRmtState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
}

impl EspRmtHandle {
    /// Advances all four RMT output channels at an abstract simulation time.
    pub fn poll(&self, at: SimTime) -> Result<u8, DeviceError> {
        let mut transitions = Vec::new();
        {
            let mut state = self.state.borrow_mut();
            for channel in 0..CHANNEL_COUNT {
                let value = EspRmtState::channel_output(&mut state.channels[channel], at);
                if state.outputs[channel] != value {
                    state.outputs[channel] = value;
                    transitions.push((channel, value));
                }
            }
        }
        for (channel, value) in &transitions {
            self.hub
                .set(
                    self.signals[*channel],
                    SignalValue::repeat(*value, 1)
                        .expect("one-bit RMT signal construction cannot fail"),
                    at,
                )
                .map_err(|error| DeviceError::new(error.to_string()))?;
        }
        Ok(u8::try_from(transitions.len()).expect("four RMT channels fit in u8"))
    }

    /// Returns the latest resolved value for one RMT channel.
    pub fn output(&self, channel: usize) -> Result<Logic, DeviceError> {
        self.state
            .borrow()
            .outputs
            .get(channel)
            .copied()
            .ok_or_else(|| DeviceError::new(format!("RMT channel {channel} is out of range")))
    }
}

/// Functional ESP32-C6 RMT transmitter/receiver register block.
///
/// The model covers the four channel FIFO/config/status windows and decodes
/// transmitter symbols written through the APB FIFO. A symbol uses the native
/// 15-bit duration/one-bit level pairs, allowing WS2812 and IR-style pulse
/// streams to be inspected as deterministic VCD output. Receiver DMA, carrier
/// modulation, and exact interrupt timing are outside this functional slice.
pub struct EspRmt {
    name: String,
    state: Rc<RefCell<EspRmtState>>,
    handle: EspRmtHandle,
}

impl EspRmt {
    /// Creates an ESP32-C6 RMT block and channel observation handle.
    pub fn new(
        name: impl Into<String>,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, EspRmtHandle), SignalError> {
        let mut signals = Vec::with_capacity(CHANNEL_COUNT);
        for channel in 0..CHANNEL_COUNT {
            signals.push(hub.declare(
                format!("{path}.ch{channel}"),
                SignalValue::repeat(Logic::Z, 1)?,
                Some(format!("RMT channel {channel} output")),
            )?);
        }
        let state = Rc::new(RefCell::new(EspRmtState::new()));
        let handle = EspRmtHandle {
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

    fn channel_data(offset: u64) -> Option<usize> {
        (offset < 0x10)
            .then(|| usize::try_from(offset / 4).ok())
            .flatten()
    }

    fn channel_config(offset: u64) -> Option<usize> {
        (0x10..0x20)
            .contains(&offset)
            .then(|| usize::try_from((offset - 0x10) / 4).ok())
            .flatten()
    }

    fn channel_status(offset: u64) -> Option<usize> {
        (0x28..0x38)
            .contains(&offset)
            .then(|| usize::try_from((offset - 0x28) / 4).ok())
            .flatten()
    }
}

impl Device for EspRmt {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP RMT requires aligned word access"));
        }
        let state = self.state.borrow();
        let value = if let Some(channel) = Self::channel_data(offset) {
            state.channels[channel].fifo.last().copied().unwrap_or(0)
        } else if let Some(channel) = Self::channel_config(offset) {
            state.channels[channel].configuration
        } else if let Some(channel) = Self::channel_status(offset) {
            let channel_state = &state.channels[channel];
            let mut status = 0_u32;
            if channel_state.transmission.is_empty() && !channel_state.active {
                status |= 1 << 22;
            }
            status
        } else {
            let index = usize::try_from(offset / 4).expect("RMT offset fits");
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
            return Err(DeviceError::new("ESP RMT requires aligned word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits");
        let mut state = self.state.borrow_mut();
        if let Some(channel) = Self::channel_data(offset) {
            state.channels[channel].fifo.push(value);
        } else if let Some(channel) = Self::channel_config(offset) {
            let channel_state = &mut state.channels[channel];
            if value & (1 << 1) != 0 {
                channel_state.transmission.clear();
            }
            if value & (1 << 7) != 0 {
                channel_state.active = false;
            }
            if value & 1 != 0 {
                channel_state.transmission = core::mem::take(&mut channel_state.fifo);
                channel_state.started_at = at;
                channel_state.active = true;
            }
            channel_state.configuration = value & !(1 | (1 << 1) | (1 << 2) | (1 << 7) | (1 << 24));
            state.registers[(offset / 4) as usize] = value;
        } else {
            let index = usize::try_from(offset / 4).expect("RMT offset fits");
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
        self.handle.poll(at).map(|_| ())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.borrow_mut();
        state.registers.fill(0);
        for channel in &mut state.channels {
            *channel = RmtChannel::default();
        }
        state.outputs = [Logic::Z; CHANNEL_COUNT];
        drop(state);
        let _ = self.handle.poll(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rmt_decodes_native_two_level_symbol() {
        let hub = SignalHub::new();
        let (mut rmt, handle) = EspRmt::new("rmt", "board.rmt", hub.clone()).unwrap();
        // Channel 0 divider=1; high for two ticks, then low for three.
        let symbol = 2 | (1 << 15) | (3 << 16);
        rmt.write(0x10, AccessWidth::Word, 1 << 8, SimTime::ZERO)
            .unwrap();
        rmt.write(0x00, AccessWidth::Word, symbol, SimTime::ZERO)
            .unwrap();
        rmt.write(0x10, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.output(0).unwrap(), Logic::One);
        handle.poll(SimTime::from_ticks(2)).unwrap();
        assert_eq!(handle.output(0).unwrap(), Logic::Zero);
        handle.poll(SimTime::from_ticks(5)).unwrap();
        assert_eq!(handle.output(0).unwrap(), Logic::Z);
        assert!(hub.drain_changes().len() >= 3);
    }

    #[test]
    fn rmt_status_reports_empty_after_transmission() {
        let hub = SignalHub::new();
        let (mut rmt, handle) = EspRmt::new("rmt", "board.rmt", hub).unwrap();
        rmt.write(0x00, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        rmt.write(0x10, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        handle.poll(SimTime::from_ticks(1)).unwrap();
        assert_ne!(
            rmt.read(0x28, AccessWidth::Word, SimTime::from_ticks(1))
                .unwrap()
                & (1 << 22),
            0
        );
    }
}
