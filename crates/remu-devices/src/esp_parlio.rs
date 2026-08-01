use super::*;

const REGISTER_BYTES: usize = 0x400;
const RX_CFG0: u64 = 0x00;
const RX_CFG1: u64 = 0x04;
const TX_CFG0: u64 = 0x08;
const TX_CFG1: u64 = 0x0c;
const STATUS: u64 = 0x10;
const INT_ENABLE: u64 = 0x14;
const INT_RAW: u64 = 0x18;
const INT_STATUS: u64 = 0x1c;
const INT_CLEAR: u64 = 0x20;
const FIFO: u64 = 0x24;
const CLOCK: u64 = 0x120;
const VERSION: u64 = 0x3fc;

const TX_EMPTY: u32 = 1 << 0;
const RX_FULL: u32 = 1 << 1;
const TX_EOF: u32 = 1 << 2;
const TX_READY: u32 = 1 << 31;
const FIFO_LIMIT: usize = 64;

#[derive(Default)]
struct EspParlioState {
    registers: Vec<u32>,
    tx_fifo: VecDeque<u16>,
    rx_fifo: VecDeque<u16>,
}

impl EspParlioState {
    fn new() -> Self {
        let mut registers = vec![0; REGISTER_BYTES / 4];
        registers[RX_CFG1 as usize / 4] = (15 << 12) | (4095 << 16) | (1 << 3);
        registers[STATUS as usize / 4] = TX_READY;
        registers[VERSION as usize / 4] = 35_660_352;
        Self {
            registers,
            ..Self::default()
        }
    }

    fn refresh(&mut self) {
        let mut status = self.registers[STATUS as usize / 4] & !TX_READY;
        if self.tx_fifo.is_empty() {
            status |= TX_READY;
        }
        self.registers[STATUS as usize / 4] = status;
        if self.tx_fifo.is_empty() {
            self.registers[INT_RAW as usize / 4] |= TX_EMPTY;
        }
        if self.rx_fifo.len() >= FIFO_LIMIT {
            self.registers[INT_RAW as usize / 4] |= RX_FULL;
        }
        self.registers[INT_STATUS as usize / 4] =
            self.registers[INT_RAW as usize / 4] & self.registers[INT_ENABLE as usize / 4];
    }
}

/// Host-facing ESP32-C6 PARLIO sample stream.
#[derive(Clone)]
pub struct EspParlioHandle {
    state: Arc<Mutex<EspParlioState>>,
}

impl EspParlioHandle {
    /// Queues one 16-bit parallel-bus sample for firmware RX tests.
    pub fn queue_rx_word(&self, word: u16) {
        let mut state = self.state.lock().expect("ESP PARLIO lock poisoned");
        if state.rx_fifo.len() < FIFO_LIMIT {
            state.rx_fifo.push_back(word);
        }
        state.refresh();
    }

    /// Returns and clears samples written by firmware through the test FIFO aperture.
    pub fn take_tx_words(&self) -> Vec<u16> {
        let mut state = self.state.lock().expect("ESP PARLIO lock poisoned");
        let words = state.tx_fifo.drain(..).collect();
        state.refresh();
        words
    }

    /// Reports whether a host sample is waiting for firmware.
    pub fn rx_available(&self) -> bool {
        !self
            .state
            .lock()
            .expect("ESP PARLIO lock poisoned")
            .rx_fifo
            .is_empty()
    }
}

/// Functional ESP32-C6 PARLIO0 register block.
///
/// Native RX/TX configuration, status, interrupt, clock and version registers
/// are modeled. The silicon normally moves parallel samples through GDMA; the
/// reserved `0x24` aperture is therefore an explicit deterministic 16-bit FIFO
/// extension for firmware tests and host fixtures. It does not claim parallel
/// pin timing, DMA descriptor ownership, or external enable pulse fidelity.
pub struct EspParlio {
    name: String,
    state: Arc<Mutex<EspParlioState>>,
    hub: SignalHub,
    tx_signal: SignalId,
    rx_signal: SignalId,
}

impl EspParlio {
    /// Creates the PARLIO register block and sample-stream handle.
    pub fn new(
        name: impl Into<String>,
        signal_prefix: &str,
        hub: SignalHub,
    ) -> Result<(Self, EspParlioHandle), SignalError> {
        let tx_signal = hub.declare(
            format!("{signal_prefix}.tx"),
            SignalValue::from_u64(0, 16)?,
            Some("last transmitted PARLIO word".to_string()),
        )?;
        let rx_signal = hub.declare(
            format!("{signal_prefix}.rx"),
            SignalValue::from_u64(0, 16)?,
            Some("last received PARLIO word".to_string()),
        )?;
        let state = Arc::new(Mutex::new(EspParlioState::new()));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
                hub,
                tx_signal,
                rx_signal,
            },
            EspParlioHandle { state },
        ))
    }

    fn emit(&self, signal: SignalId, value: u16, at: SimTime) -> Result<(), DeviceError> {
        let value = SignalValue::from_u64(u64::from(value), 16)
            .map_err(|error| DeviceError::new(format!("{} signal value: {error}", self.name)))?;
        self.hub
            .set(signal, value, at)
            .map_err(|error| DeviceError::new(format!("{} signal update: {error}", self.name)))
    }
}

impl Device for EspParlio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP PARLIO requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP PARLIO lock poisoned");
        if offset == FIFO {
            let word = state.rx_fifo.pop_front().unwrap_or_default();
            state.refresh();
            drop(state);
            self.emit(self.rx_signal, word, at)?;
            return Ok(u64::from(word));
        }
        let value = if offset == INT_STATUS {
            state.refresh();
            state.registers[INT_STATUS as usize / 4]
        } else if offset == STATUS {
            state.refresh();
            state.registers[STATUS as usize / 4]
        } else {
            let index = usize::try_from(offset / 4).expect("PARLIO register index fits");
            *state
                .registers
                .get(index)
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?
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
            return Err(DeviceError::new("ESP PARLIO requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP PARLIO lock poisoned");
        let value = value as u32;
        if offset == FIFO {
            let word = value as u16;
            state.tx_fifo.push_back(word);
            state.registers[INT_RAW as usize / 4] |= TX_EOF;
            state.refresh();
            drop(state);
            self.emit(self.tx_signal, word, at)?;
            return Ok(());
        }
        match offset {
            INT_ENABLE => state.registers[INT_ENABLE as usize / 4] = value & 0x07,
            INT_RAW | INT_CLEAR => state.registers[INT_RAW as usize / 4] &= !value,
            RX_CFG0 if value & (1 << 31) != 0 => {
                state.registers[RX_CFG0 as usize / 4] = value;
                state.rx_fifo.clear();
            }
            TX_CFG0 if value & (1 << 30) != 0 => {
                state.registers[TX_CFG0 as usize / 4] = value;
                state.tx_fifo.clear();
            }
            RX_CFG0 | RX_CFG1 | TX_CFG0 | TX_CFG1 | CLOCK => {
                state.registers[offset as usize / 4] = value;
            }
            _ => {
                let index = usize::try_from(offset / 4).expect("PARLIO register index fits");
                let register = state.registers.get_mut(index).ok_or_else(|| {
                    DeviceError::new(format!("{} write at {offset:#x}", self.name))
                })?;
                *register = value;
            }
        }
        state.refresh();
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ESP PARLIO lock poisoned");
        *state = EspParlioState::new();
        let zero = SignalValue::from_u64(0, 16).expect("16-bit signal");
        let _ = self.hub.set(self.tx_signal, zero.clone(), SimTime::ZERO);
        let _ = self.hub.set(self.rx_signal, zero, SimTime::ZERO);
    }
}
