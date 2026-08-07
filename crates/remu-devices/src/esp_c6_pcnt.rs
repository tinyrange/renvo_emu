use super::*;

const UNIT_COUNT: usize = 4;
const UNIT_STRIDE: usize = 0x0c;
const CONF0: usize = 0x00;
const CONF1: usize = 0x04;
const CONF2: usize = 0x08;
const COUNT_BASE: usize = 0x30;
const INT_RAW: usize = 0x40;
const INT_ST: usize = 0x44;
const INT_ENA: usize = 0x48;
const INT_CLR: usize = 0x4c;
const STATUS_BASE: usize = 0x50;
const CTRL: usize = 0x60;

/// Host-facing input and interrupt view of the ESP32-C6 PCNT block.
#[derive(Clone)]
pub struct EspPcntHandle {
    state: Arc<Mutex<EspPcntState>>,
}

impl EspPcntHandle {
    /// Connects a PCNT channel to a GPIO pin in the functional input matrix.
    pub fn bind_input(&self, unit: usize, channel: usize, pin: u8) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("ESP PCNT lock poisoned");
        if unit >= UNIT_COUNT || channel >= 2 {
            return Err(DeviceError::new("ESP PCNT channel is out of range"));
        }
        state.input_pins[unit][channel] = pin;
        Ok(())
    }

    /// Feeds one resolved GPIO transition into the pulse-counter matrix.
    pub fn observe_input(&self, pin: u8, value: Logic, at: SimTime) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("ESP PCNT lock poisoned");
        let level = value == Logic::One;
        let mut changed = Vec::new();
        for unit in 0..UNIT_COUNT {
            for channel in 0..2 {
                if state.input_pins[unit][channel] != pin {
                    continue;
                }
                let previous = state.input_levels[unit][channel];
                if previous == level {
                    continue;
                }
                state.input_levels[unit][channel] = level;
                let rising = !previous && level;
                let old_count = state.counts[unit];
                state.apply_edge(unit, channel, rising);
                if state.counts[unit] != old_count {
                    changed.push((unit, state.counts[unit] as u16));
                }
            }
        }
        for (unit, count) in changed {
            state
                .hub
                .set(
                    state.signals[unit],
                    SignalValue::from_u64(u64::from(count), 16)
                        .expect("PCNT count signal has fixed width"),
                    at,
                )
                .map_err(|error| DeviceError::new(error.to_string()))?;
        }
        Ok(())
    }

    /// Returns the masked threshold interrupt level for the four units.
    pub fn pending(&self) -> bool {
        let state = self.state.lock().expect("ESP PCNT lock poisoned");
        state.registers[INT_ST / 4] & 0x0f != 0
    }
}

struct EspPcntState {
    registers: Vec<u32>,
    counts: [i16; UNIT_COUNT],
    input_pins: [[u8; 2]; UNIT_COUNT],
    input_levels: [[bool; 2]; UNIT_COUNT],
    hub: SignalHub,
    signals: [SignalId; UNIT_COUNT],
}

impl EspPcntState {
    fn new(hub: SignalHub, signals: [SignalId; UNIT_COUNT]) -> Self {
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            counts: [0; UNIT_COUNT],
            input_pins: [[0, 1], [2, 3], [4, 5], [6, 7]],
            input_levels: [[false; 2]; UNIT_COUNT],
            hub,
            signals,
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.counts = [0; UNIT_COUNT];
        self.input_levels = [[false; 2]; UNIT_COUNT];
        for unit in 0..UNIT_COUNT {
            let base = unit * UNIT_STRIDE;
            // PCNT reset enables the filter and the zero/limit comparators.
            self.registers[(base + CONF0) / 4] = (16 & 0x3ff) | (0xf << 10);
        }
        self.registers[CTRL / 4] = 0;
        self.registers[0xfc / 4] = 419_898_881;
    }

    fn paused(&self, unit: usize) -> bool {
        self.registers[CTRL / 4] & (1 << (unit * 2 + 1)) != 0
    }

    fn mode(conf: u32, channel: usize, rising: bool) -> u32 {
        let shift = 16 + channel * 8 + usize::from(rising) * 2;
        (conf >> shift) & 0x03
    }

    fn control_mode(conf: u32, channel: usize) -> u32 {
        (conf >> (22 + channel * 8)) & 0x03
    }

    fn apply_edge(&mut self, unit: usize, channel: usize, rising: bool) {
        if self.paused(unit) {
            return;
        }
        let conf = self.registers[(unit * UNIT_STRIDE + CONF0) / 4];
        let mut mode = Self::mode(conf, channel, rising);
        // The functional model has no separate control-input net; its low
        // level therefore applies the configured low-control action.
        match Self::control_mode(conf, channel) {
            1 => {
                if mode == 1 {
                    mode = 2;
                } else if mode == 2 {
                    mode = 1;
                }
            }
            2 | 3 => mode = 0,
            _ => {}
        }
        let delta = match mode {
            1 => 1,
            2 => -1,
            _ => return,
        };
        let previous = self.counts[unit];
        self.counts[unit] = previous.saturating_add(delta);
        self.latch_events(unit);
    }

    fn latch_events(&mut self, unit: usize) {
        let count = self.counts[unit];
        let conf0 = self.registers[(unit * UNIT_STRIDE + CONF0) / 4];
        let conf1 = self.registers[(unit * UNIT_STRIDE + CONF1) / 4];
        let conf2 = self.registers[(unit * UNIT_STRIDE + CONF2) / 4];
        let threshold0 = (conf1 & 0xffff) as i16;
        let threshold1 = (conf1 >> 16) as i16;
        let high = (conf2 & 0xffff) as i16;
        let low = (conf2 >> 16) as i16;
        let mut status = 0_u32;
        if conf0 & (1 << 11) != 0 && count == 0 {
            status |= 1 << 6;
        }
        if conf0 & (1 << 12) != 0 && count == high {
            status |= 1 << 5;
        }
        if conf0 & (1 << 13) != 0 && count == low {
            status |= 1 << 4;
        }
        if conf0 & (1 << 14) != 0 && count == threshold0 {
            status |= 1 << 3;
        }
        if conf0 & (1 << 15) != 0 && count == threshold1 {
            status |= 1 << 2;
        }
        self.registers[(STATUS_BASE + unit * 4) / 4] = status;
        if status != 0 {
            self.registers[INT_RAW / 4] |= 1 << unit;
            self.registers[INT_ST / 4] = self.registers[INT_RAW / 4] & self.registers[INT_ENA / 4];
        }
    }
}

/// Functional ESP32-C6 pulse-count controller.
pub struct EspPcnt {
    name: String,
    state: Arc<Mutex<EspPcntState>>,
}

impl EspPcnt {
    /// Creates four 16-bit pulse counters and declares their VCD signals.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, EspPcntHandle), SignalError> {
        let mut signals = Vec::with_capacity(UNIT_COUNT);
        for unit in 0..UNIT_COUNT {
            signals.push(hub.declare(
                format!("board.esp32c6.pcnt.u{unit}"),
                SignalValue::from_u64(0, 16)?,
                Some(format!("ESP32-C6 PCNT unit {unit} count")),
            )?);
        }
        let signals: [SignalId; UNIT_COUNT] = signals.try_into().expect("four PCNT signals");
        let state = Arc::new(Mutex::new(EspPcntState::new(hub, signals)));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspPcntHandle { state },
        ))
    }
}

impl Device for EspPcnt {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP PCNT requires aligned word access"));
        }
        let offset = usize::try_from(offset).expect("PCNT offset fits");
        let state = self.state.lock().expect("ESP PCNT lock poisoned");
        let value = if (COUNT_BASE..COUNT_BASE + UNIT_COUNT * 4).contains(&offset)
            && (offset - COUNT_BASE) % 4 == 0
        {
            u32::from(state.counts[(offset - COUNT_BASE) / 4] as u16)
        } else {
            *state
                .registers
                .get(offset / 4)
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP PCNT requires aligned word access"));
        }
        let offset = usize::try_from(offset).expect("PCNT offset fits");
        let value = value as u32;
        let mut state = self.state.lock().expect("ESP PCNT lock poisoned");
        if offset >= state.registers.len() * 4 {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        match offset {
            INT_RAW | INT_ST => {}
            INT_ENA => {
                state.registers[INT_ENA / 4] = value & 0x0f;
                state.registers[INT_ST / 4] =
                    state.registers[INT_RAW / 4] & state.registers[INT_ENA / 4];
            }
            INT_CLR => {
                state.registers[INT_RAW / 4] &= !(value & 0x0f);
                state.registers[INT_ST / 4] =
                    state.registers[INT_RAW / 4] & state.registers[INT_ENA / 4];
            }
            CTRL => {
                for unit in 0..UNIT_COUNT {
                    if value & (1 << (unit * 2)) != 0 {
                        state.counts[unit] = 0;
                        state.registers[(STATUS_BASE + unit * 4) / 4] = 0;
                        state
                            .hub
                            .set(
                                state.signals[unit],
                                SignalValue::from_u64(0, 16).expect("PCNT count signal width"),
                                SimTime::ZERO,
                            )
                            .map_err(|error| DeviceError::new(error.to_string()))?;
                    }
                }
                state.registers[CTRL / 4] = value & 0x0001_00ff & !(0x55);
                state.registers[CTRL / 4] |= value & 0xaa;
            }
            offset if offset >= COUNT_BASE && offset < COUNT_BASE + UNIT_COUNT * 4 => {}
            _ => state.registers[offset / 4] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.lock().expect("ESP PCNT lock poisoned").reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_configured_rising_edges_and_latches_threshold_interrupt() {
        let hub = SignalHub::new();
        let (mut pcnt, handle) = EspPcnt::new("pcnt", hub).unwrap();
        handle.bind_input(0, 0, 9).unwrap();
        // CH0 positive-edge mode = increment, threshold0 = 1, enable it.
        pcnt.write(
            CONF0 as u64,
            AccessWidth::Word,
            (1 << 18) | (1 << 14),
            SimTime::ZERO,
        )
        .unwrap();
        pcnt.write(CONF1 as u64, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pcnt.write(INT_ENA as u64, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        handle.observe_input(9, Logic::Zero, SimTime::ZERO).unwrap();
        handle
            .observe_input(9, Logic::One, SimTime::from_ticks(1))
            .unwrap();
        assert_eq!(
            pcnt.read(COUNT_BASE as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
        assert_eq!(
            pcnt.read(INT_ST as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
        assert!(handle.pending());
        pcnt.write(INT_CLR as u64, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(!handle.pending());
    }

    #[test]
    fn reset_and_pause_control_follow_native_register_windows() {
        let hub = SignalHub::new();
        let (mut pcnt, handle) = EspPcnt::new("pcnt", hub).unwrap();
        handle.bind_input(1, 0, 3).unwrap();
        pcnt.write(
            CONF0 as u64 + 0x0c,
            AccessWidth::Word,
            1 << 18,
            SimTime::ZERO,
        )
        .unwrap();
        handle.observe_input(3, Logic::One, SimTime::ZERO).unwrap();
        assert_eq!(
            pcnt.read((COUNT_BASE + 4) as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
        pcnt.write(CTRL as u64, AccessWidth::Word, 1 << 3, SimTime::ZERO)
            .unwrap();
        handle
            .observe_input(3, Logic::Zero, SimTime::from_ticks(1))
            .unwrap();
        handle
            .observe_input(3, Logic::One, SimTime::from_ticks(2))
            .unwrap();
        assert_eq!(
            pcnt.read((COUNT_BASE + 4) as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
        pcnt.write(CTRL as u64, AccessWidth::Word, 1 << 2, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            pcnt.read((COUNT_BASE + 4) as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
    }
}
