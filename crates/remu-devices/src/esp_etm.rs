use super::*;

const REGISTER_BYTES: usize = 0x1b0;
const CH_ENA_AD0: u64 = 0x00;
const CH_ENA_AD0_SET: u64 = 0x04;
const CH_ENA_AD0_CLR: u64 = 0x08;
const CH_ENA_AD1: u64 = 0x0c;
const CH_ENA_AD1_SET: u64 = 0x10;
const CH_ENA_AD1_CLR: u64 = 0x14;
const CHANNEL_BASE: u64 = 0x18;
const CHANNEL_STRIDE: u64 = 0x08;
const CLOCK_ENABLE: u64 = 0x1a8;
const DATE: u64 = 0x1ac;
const CHANNEL_COUNT: usize = 50;
const AD1_MASK: u32 = (1 << (CHANNEL_COUNT - 32)) - 1;

#[derive(Default)]
struct EspEtmState {
    registers: Vec<u32>,
    tasks: VecDeque<u8>,
}

impl EspEtmState {
    fn new() -> Self {
        let mut registers = vec![0; REGISTER_BYTES / 4];
        registers[DATE as usize / 4] = 35_664_018;
        Self {
            registers,
            ..Self::default()
        }
    }

    fn enabled(&self, channel: usize) -> bool {
        if self.channel_event(channel) == 0 || self.channel_task(channel) == 0 {
            // The native ETM disables a channel whose event or task selector
            // is zero, even if its enable bit was previously set.
            return false;
        }
        if channel < 32 {
            self.registers[CH_ENA_AD0 as usize / 4] & (1 << channel) != 0
        } else {
            self.registers[CH_ENA_AD1 as usize / 4] & (1 << (channel - 32)) != 0
        }
    }

    fn channel_event(&self, channel: usize) -> u8 {
        self.registers[(CHANNEL_BASE + channel as u64 * CHANNEL_STRIDE) as usize / 4] as u8
    }

    fn channel_task(&self, channel: usize) -> u8 {
        self.registers[(CHANNEL_BASE + channel as u64 * CHANNEL_STRIDE + 4) as usize / 4] as u8
    }

    fn enabled_word(&self, first_channel: usize, count: usize) -> u32 {
        (0..count).fold(0, |word, index| {
            word | (u32::from(self.enabled(first_channel + index)) << index)
        })
    }
}

/// Host-facing event/task state for ESP32-C6 ETM.
#[derive(Clone)]
pub struct EspEtmHandle {
    state: Arc<Mutex<EspEtmState>>,
    hub: SignalHub,
    event_signal: SignalId,
    task_signal: SignalId,
}

impl EspEtmHandle {
    /// Triggers an event ID and returns the enabled task IDs it dispatches.
    pub fn trigger(&self, event: u8, at: SimTime) -> Result<Vec<u8>, SignalError> {
        let mut state = self.state.lock().expect("ESP ETM lock poisoned");
        let tasks = (0..CHANNEL_COUNT)
            .filter(|&channel| state.enabled(channel) && state.channel_event(channel) == event)
            .map(|channel| state.channel_task(channel))
            .collect::<Vec<_>>();
        state.tasks.extend(tasks.iter().copied());
        drop(state);
        self.hub.set(
            self.event_signal,
            SignalValue::from_u64(u64::from(event), 8)?,
            at,
        )?;
        for task in tasks.iter().copied() {
            self.hub.set(
                self.task_signal,
                SignalValue::from_u64(u64::from(task), 8)?,
                at,
            )?;
        }
        Ok(tasks)
    }

    /// Returns and clears tasks dispatched by [`Self::trigger`].
    pub fn take_tasks(&self) -> Vec<u8> {
        let mut state = self.state.lock().expect("ESP ETM lock poisoned");
        state.tasks.drain(..).collect()
    }
}

/// Functional ESP32-C6 event task matrix.
///
/// All fifty native channel event/task selectors and the split enable
/// registers are modeled. Host tests can inject an event through the handle;
/// matching enabled channels queue their task IDs and emit deterministic VCD
/// event/task values. Peripheral-specific task side effects and interrupt
/// routing remain owned by their individual models.
pub struct EspEtm {
    name: String,
    state: Arc<Mutex<EspEtmState>>,
    hub: SignalHub,
    event_signal: SignalId,
    task_signal: SignalId,
}

impl EspEtm {
    /// Creates the ETM register block and event-injection handle.
    pub fn new(
        name: impl Into<String>,
        signal_prefix: &str,
        hub: SignalHub,
    ) -> Result<(Self, EspEtmHandle), SignalError> {
        let event_signal = hub.declare(
            format!("{signal_prefix}.event"),
            SignalValue::from_u64(0, 8)?,
            Some("last triggered ETM event ID".to_string()),
        )?;
        let task_signal = hub.declare(
            format!("{signal_prefix}.task"),
            SignalValue::from_u64(0, 8)?,
            Some("last dispatched ETM task ID".to_string()),
        )?;
        let state = Arc::new(Mutex::new(EspEtmState::new()));
        let handle = EspEtmHandle {
            state: state.clone(),
            hub: hub.clone(),
            event_signal,
            task_signal,
        };
        Ok((
            Self {
                name: name.into(),
                state,
                hub,
                event_signal,
                task_signal,
            },
            handle,
        ))
    }
}

impl Device for EspEtm {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP ETM requires aligned word access"));
        }
        let state = self.state.lock().expect("ESP ETM lock poisoned");
        if offset == CH_ENA_AD0 {
            return Ok(u64::from(state.enabled_word(0, 32)));
        }
        if offset == CH_ENA_AD1 {
            return Ok(u64::from(state.enabled_word(32, CHANNEL_COUNT - 32)));
        }
        let index = usize::try_from(offset / 4).expect("ETM register index fits");
        state
            .registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP ETM requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP ETM lock poisoned");
        let value = value as u32;
        match offset {
            CH_ENA_AD0 => state.registers[CH_ENA_AD0 as usize / 4] = value,
            CH_ENA_AD0_SET => state.registers[CH_ENA_AD0 as usize / 4] |= value,
            CH_ENA_AD0_CLR => state.registers[CH_ENA_AD0 as usize / 4] &= !value,
            CH_ENA_AD1 => state.registers[CH_ENA_AD1 as usize / 4] = value & AD1_MASK,
            CH_ENA_AD1_SET => state.registers[CH_ENA_AD1 as usize / 4] |= value & AD1_MASK,
            CH_ENA_AD1_CLR => state.registers[CH_ENA_AD1 as usize / 4] &= !(value & AD1_MASK),
            CHANNEL_BASE..=0x1a4 => {
                let index =
                    usize::try_from((offset - CHANNEL_BASE) / 4).expect("ETM channel index fits");
                state.registers[(CHANNEL_BASE as usize / 4) + index] = value & 0xff;
            }
            CLOCK_ENABLE => state.registers[CLOCK_ENABLE as usize / 4] = value & 1,
            _ => {
                let index = usize::try_from(offset / 4).expect("ETM register index fits");
                let register = state.registers.get_mut(index).ok_or_else(|| {
                    DeviceError::new(format!("{} write at {offset:#x}", self.name))
                })?;
                *register = value;
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ESP ETM lock poisoned");
        *state = EspEtmState::new();
        let zero = SignalValue::from_u64(0, 8).expect("8-bit signal");
        let _ = self.hub.set(self.event_signal, zero.clone(), SimTime::ZERO);
        let _ = self.hub.set(self.task_signal, zero, SimTime::ZERO);
    }
}
