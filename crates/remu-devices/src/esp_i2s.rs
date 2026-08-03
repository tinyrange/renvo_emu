use super::*;

const REGISTER_BYTES: usize = 0x84;
const FIFO: u64 = 0x00;
const INT_RAW: u64 = 0x0c;
const INT_STATUS: u64 = 0x10;
const INT_ENABLE: u64 = 0x14;
const INT_CLEAR: u64 = 0x18;
const RX_CONF: u64 = 0x20;
const TX_CONF: u64 = 0x24;
const RX_EOF_NUM: u64 = 0x64;
const STATE: u64 = 0x6c;
const DATE: u64 = 0x80;

const RX_DONE: u32 = 1 << 0;
const TX_DONE: u32 = 1 << 1;
const RX_RESET: u32 = 1 << 0;
const RX_FIFO_RESET: u32 = 1 << 1;
const TX_RESET: u32 = 1 << 0;
const TX_FIFO_RESET: u32 = 1 << 1;
const TX_IDLE: u32 = 1;
const RX_RESET_FIELDS: u32 = RX_RESET | RX_FIFO_RESET;
const TX_RESET_FIELDS: u32 = TX_RESET | TX_FIFO_RESET;
const RX_EOF_MASK: u32 = 0x0fff;

#[derive(Default)]
struct EspI2sState {
    registers: Vec<u32>,
    tx_fifo: VecDeque<u32>,
    rx_fifo: VecDeque<u32>,
}

impl EspI2sState {
    fn new() -> Self {
        let mut registers = vec![0; REGISTER_BYTES / 4];
        // Stable reset fields consumed by the ESP-IDF standard-mode driver.
        // Reset values from ESP-IDF's esp32c6 I2S register description:
        // RX_MONO_FST_VLD=1, RX_PCM_CONF=1, RX_PCM_BYPASS=1 and
        // RX_LEFT_ALIGN=1.
        registers[RX_CONF as usize / 4] = (1 << 9) | (1 << 10) | (1 << 12) | (1 << 15);
        registers[TX_CONF as usize / 4] = (1 << 9) | (1 << 12) | (1 << 13) | (1 << 15);
        registers[RX_EOF_NUM as usize / 4] = 64;
        registers[STATE as usize / 4] = TX_IDLE;
        registers[0x60 / 4] = 16 | (1 << 11);
        registers[DATE as usize / 4] = 35_655_792;
        Self {
            registers,
            ..Self::default()
        }
    }

    fn refresh_status(&mut self) {
        self.registers[INT_STATUS as usize / 4] =
            self.registers[INT_RAW as usize / 4] & self.registers[INT_ENABLE as usize / 4];
        self.registers[STATE as usize / 4] = if self.tx_fifo.is_empty() { TX_IDLE } else { 0 };
    }

    fn clear_rx(&mut self) {
        self.rx_fifo.clear();
        self.registers[INT_RAW as usize / 4] &= !RX_DONE;
    }

    fn clear_tx(&mut self) {
        self.tx_fifo.clear();
        self.registers[INT_RAW as usize / 4] &= !TX_DONE;
    }
}

/// Host-facing state for the ESP32-C6 I2S sample stream.
#[derive(Clone)]
pub struct EspI2sHandle {
    state: Arc<Mutex<EspI2sState>>,
}

impl EspI2sHandle {
    /// Queues a deterministic 32-bit sample for the functional RX FIFO.
    pub fn queue_rx_word(&self, word: u32) {
        let mut state = self.state.lock().expect("ESP I2S lock poisoned");
        state.rx_fifo.push_back(word);
        state.registers[INT_RAW as usize / 4] |= RX_DONE;
        state.refresh_status();
    }

    /// Returns and clears samples written through the functional TX FIFO.
    pub fn take_tx_words(&self) -> Vec<u32> {
        let mut state = self.state.lock().expect("ESP I2S lock poisoned");
        let words = state.tx_fifo.drain(..).collect();
        state.refresh_status();
        words
    }

    /// Reports whether a host sample is waiting for firmware.
    pub fn rx_available(&self) -> bool {
        !self
            .state
            .lock()
            .expect("ESP I2S lock poisoned")
            .rx_fifo
            .is_empty()
    }
}

/// Functional ESP32-C6 I2S0 register block and sample stream.
///
/// The native register layout covers the standard-mode interrupt, RX/TX
/// configuration, clock-divider, slot, TDM, timing, single-data, state and
/// version registers. Since the silicon transports samples through GDMA rather
/// than a CPU-visible FIFO, offset `0x00` is an explicit deterministic FIFO
/// aperture for emulator tests and host integration. It does not claim to be a
/// replacement for native GDMA descriptors or bit-level audio timing.
pub struct EspI2s {
    name: String,
    state: Arc<Mutex<EspI2sState>>,
    hub: SignalHub,
    tx_signal: SignalId,
    rx_signal: SignalId,
}

impl EspI2s {
    /// Creates the I2S register block and host-facing sample handle.
    pub fn new(
        name: impl Into<String>,
        signal_prefix: &str,
        hub: SignalHub,
    ) -> Result<(Self, EspI2sHandle), SignalError> {
        let tx_signal = hub.declare(
            format!("{signal_prefix}.tx"),
            SignalValue::from_u64(0, 32)?,
            Some("last transmitted I2S sample".to_string()),
        )?;
        let rx_signal = hub.declare(
            format!("{signal_prefix}.rx"),
            SignalValue::from_u64(0, 32)?,
            Some("last received I2S sample".to_string()),
        )?;
        let state = Arc::new(Mutex::new(EspI2sState::new()));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
                hub,
                tx_signal,
                rx_signal,
            },
            EspI2sHandle { state },
        ))
    }

    fn emit(&self, signal: SignalId, value: u32, at: SimTime) -> Result<(), DeviceError> {
        let value = SignalValue::from_u64(u64::from(value), 32)
            .map_err(|error| DeviceError::new(format!("{} signal value: {error}", self.name)))?;
        self.hub
            .set(signal, value, at)
            .map_err(|error| DeviceError::new(format!("{} signal update: {error}", self.name)))
    }

    fn push_tx(&self, state: &mut EspI2sState, word: u32, at: SimTime) -> Result<(), DeviceError> {
        state.tx_fifo.push_back(word);
        state.registers[INT_RAW as usize / 4] |= TX_DONE;
        self.emit(self.tx_signal, word, at)
    }
}

impl Device for EspI2s {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP I2S requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP I2S lock poisoned");
        if offset == FIFO {
            let word = state.rx_fifo.pop_front().unwrap_or_default();
            if state.rx_fifo.is_empty() {
                state.registers[INT_RAW as usize / 4] &= !RX_DONE;
            }
            state.refresh_status();
            drop(state);
            self.emit(self.rx_signal, word, at)?;
            return Ok(u64::from(word));
        }
        let value = if offset == INT_STATUS {
            state.refresh_status();
            state.registers[INT_STATUS as usize / 4]
        } else if offset == STATE {
            state.refresh_status();
            state.registers[STATE as usize / 4]
        } else {
            let index = usize::try_from(offset / 4).expect("I2S register index fits");
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
            return Err(DeviceError::new("ESP I2S requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP I2S lock poisoned");
        let value = value as u32;
        if offset == FIFO {
            self.push_tx(&mut state, value, at)?;
            state.refresh_status();
            return Ok(());
        }
        match offset {
            INT_RAW | INT_CLEAR => state.registers[INT_RAW as usize / 4] &= !value,
            INT_ENABLE => state.registers[INT_ENABLE as usize / 4] = value & 0x0f,
            RX_CONF => {
                // RX_RESET and RX_FIFO_RESET are write-trigger fields (WT),
                // not persistent configuration bits.  The LL driver writes
                // one and then writes zero, so the readback must remain zero.
                state.registers[RX_CONF as usize / 4] = value & !RX_RESET_FIELDS;
                if value & RX_RESET_FIELDS != 0 {
                    state.clear_rx();
                }
            }
            TX_CONF => {
                // TX_RESET and TX_FIFO_RESET have the same write-trigger
                // semantics as their RX counterparts.
                state.registers[TX_CONF as usize / 4] = value & !TX_RESET_FIELDS;
                if value & TX_RESET_FIELDS != 0 {
                    state.clear_tx();
                }
            }
            // INT_STATUS and STATE are read-only in the native register
            // block.  Ignore firmware writes rather than creating state that
            // hardware cannot expose.
            INT_STATUS | STATE => {}
            RX_EOF_NUM => state.registers[RX_EOF_NUM as usize / 4] = value & RX_EOF_MASK,
            _ => {
                let index = usize::try_from(offset / 4).expect("I2S register index fits");
                let register = state.registers.get_mut(index).ok_or_else(|| {
                    DeviceError::new(format!("{} write at {offset:#x}", self.name))
                })?;
                *register = value;
            }
        }
        state.refresh_status();
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ESP I2S lock poisoned");
        *state = EspI2sState::new();
        let zero = SignalValue::from_u64(0, 32).expect("32-bit signal");
        let _ = self.hub.set(self.tx_signal, zero.clone(), SimTime::ZERO);
        let _ = self.hub.set(self.rx_signal, zero, SimTime::ZERO);
    }
}
