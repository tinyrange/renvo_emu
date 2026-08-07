use super::*;

const REGISTER_BYTES: usize = 0xa0;
const FIFO: u64 = 0x00;
const INT_RAW: u64 = 0x04;
const INT_STATUS: u64 = 0x08;
const INT_ENABLE: u64 = 0x0c;
const INT_CLEAR: u64 = 0x10;
const CLKDIV: u64 = 0x14;
const RX_FILTER: u64 = 0x18;
const STATUS: u64 = 0x1c;
const CONFIG0: u64 = 0x20;
const CONFIG1: u64 = 0x24;
const MEM_CONFIG: u64 = 0x60;
const TIMEOUT_CONFIG: u64 = 0x64;
const CLOCK_CONFIG: u64 = 0x88;
const DATE: u64 = 0x8c;
const REGISTER_UPDATE: u64 = 0x98;
const ID: u64 = 0x9c;

const RX_FULL: u32 = 1 << 0;
const TX_EMPTY: u32 = 1 << 1;
const TX_DONE: u32 = 1 << 14;
const FIFO_LIMIT: usize = 16;
const RX_COUNT_SHIFT: u32 = 3;
const TX_COUNT_SHIFT: u32 = 19;
const CTSN: u32 = 1 << 14;
const RXD: u32 = 1 << 15;
const DTRN: u32 = 1 << 29;
const RTSN: u32 = 1 << 30;
const TXD: u32 = 1 << 31;

#[derive(Default)]
struct EspLpUartState {
    registers: Vec<u32>,
    rx_fifo: VecDeque<u8>,
    tx_fifo: VecDeque<u8>,
}

impl EspLpUartState {
    fn new() -> Self {
        let mut registers = vec![0; REGISTER_BYTES / 4];
        registers[INT_RAW as usize / 4] = TX_EMPTY;
        registers[CLKDIV as usize / 4] = 694;
        registers[RX_FILTER as usize / 4] = 8;
        registers[CONFIG0 as usize / 4] = (3 << 2) | (1 << 4) | (1 << 20);
        registers[CONFIG1 as usize / 4] = (12 << 3) | (12 << 11);
        registers[STATUS as usize / 4] = CTSN | RXD | DTRN | RTSN | TXD;
        registers[DATE as usize / 4] = 35_656_288;
        registers[ID as usize / 4] = 1_280;
        Self {
            registers,
            ..Self::default()
        }
    }

    fn refresh(&mut self) {
        let rx_threshold = ((self.registers[CONFIG1 as usize / 4] >> 3) & 0x1f) as usize;
        let tx_threshold = ((self.registers[CONFIG1 as usize / 4] >> 11) & 0x1f) as usize;
        if self.rx_fifo.len() > rx_threshold {
            self.registers[INT_RAW as usize / 4] |= RX_FULL;
        }
        if self.tx_fifo.len() <= tx_threshold {
            self.registers[INT_RAW as usize / 4] |= TX_EMPTY;
        }
        self.registers[INT_STATUS as usize / 4] =
            self.registers[INT_RAW as usize / 4] & self.registers[INT_ENABLE as usize / 4];

        let mut status = CTSN | RXD | DTRN | RTSN | TXD;
        status |= (self.rx_fifo.len() as u32) << RX_COUNT_SHIFT;
        status |= (self.tx_fifo.len() as u32) << TX_COUNT_SHIFT;
        self.registers[STATUS as usize / 4] = status;
    }
}

/// Host-facing LP UART control and receive handle.
#[derive(Clone)]
pub struct EspLpUartHandle {
    state: Arc<Mutex<EspLpUartState>>,
}

impl EspLpUartHandle {
    /// Queues bytes for firmware reads from the LP UART FIFO.
    pub fn queue_rx(&self, bytes: &[u8]) {
        let mut state = self.state.lock().expect("ESP LP UART lock poisoned");
        for byte in bytes {
            if state.rx_fifo.len() < FIFO_LIMIT {
                state.rx_fifo.push_back(*byte);
            } else {
                state.registers[INT_RAW as usize / 4] |= 1 << 4;
            }
        }
        state.refresh();
    }

    /// Reports whether firmware has a byte waiting in the RX FIFO.
    pub fn rx_available(&self) -> bool {
        !self
            .state
            .lock()
            .expect("ESP LP UART lock poisoned")
            .rx_fifo
            .is_empty()
    }
}

/// Functional ESP32-C6 LP UART register block.
pub struct EspLpUart {
    name: String,
    state: Arc<Mutex<EspLpUartState>>,
    output: UartHandle,
    hub: SignalHub,
    tx_signal: SignalId,
    rx_signal: SignalId,
}

impl EspLpUart {
    /// Creates the LP UART and returns terminal output plus a host RX handle.
    pub fn new(
        name: impl Into<String>,
        signal_prefix: &str,
        hub: SignalHub,
    ) -> Result<(Self, UartHandle, EspLpUartHandle), SignalError> {
        let tx_signal = hub.declare(
            format!("{signal_prefix}.tx"),
            SignalValue::from_u64(0, 8)?,
            Some("last transmitted LP UART byte".to_string()),
        )?;
        let rx_signal = hub.declare(
            format!("{signal_prefix}.rx"),
            SignalValue::from_u64(0, 8)?,
            Some("last received LP UART byte".to_string()),
        )?;
        let state = Arc::new(Mutex::new(EspLpUartState::new()));
        let output = UartHandle::default();
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
                output: output.clone(),
                hub,
                tx_signal,
                rx_signal,
            },
            output,
            EspLpUartHandle { state },
        ))
    }

    fn emit(&self, signal: SignalId, value: u8, at: SimTime) -> Result<(), DeviceError> {
        let value = SignalValue::from_u64(u64::from(value), 8)
            .map_err(|error| DeviceError::new(format!("{} signal value: {error}", self.name)))?;
        self.hub
            .set(signal, value, at)
            .map_err(|error| DeviceError::new(format!("{} signal update: {error}", self.name)))
    }
}

impl Device for EspLpUart {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if offset & 3 != 0 || width != AccessWidth::Word {
            return Err(DeviceError::new("ESP LP UART requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP LP UART lock poisoned");
        if offset == FIFO {
            let byte = state.rx_fifo.pop_front().unwrap_or_default();
            state.refresh();
            drop(state);
            self.emit(self.rx_signal, byte, at)?;
            return Ok(u64::from(byte));
        }
        let value = if offset == INT_STATUS || offset == STATUS {
            state.refresh();
            state.registers[offset as usize / 4]
        } else {
            let index = usize::try_from(offset / 4).expect("LP UART register index fits");
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
        if offset & 3 != 0 || width != AccessWidth::Word {
            return Err(DeviceError::new("ESP LP UART requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP LP UART lock poisoned");
        let value = value as u32;
        if offset == FIFO {
            let byte = value as u8;
            if state.tx_fifo.len() < FIFO_LIMIT {
                state.tx_fifo.push_back(byte);
            }
            state.tx_fifo.pop_front();
            state.registers[INT_RAW as usize / 4] |= TX_DONE;
            state.refresh();
            drop(state);
            self.output.transmit(&[byte]);
            self.emit(self.tx_signal, byte, at)?;
            return Ok(());
        }
        match offset {
            INT_ENABLE => state.registers[INT_ENABLE as usize / 4] = value & ((1 << 20) - 1),
            INT_RAW => state.registers[INT_RAW as usize / 4] = value & ((1 << 20) - 1),
            INT_CLEAR => state.registers[INT_RAW as usize / 4] &= !value,
            CONFIG0 => {
                state.registers[CONFIG0 as usize / 4] = value;
                if value & (1 << 22) != 0 {
                    state.rx_fifo.clear();
                }
                if value & (1 << 23) != 0 {
                    state.tx_fifo.clear();
                }
            }
            CONFIG1 | CLKDIV | RX_FILTER | MEM_CONFIG | TIMEOUT_CONFIG | CLOCK_CONFIG
            | REGISTER_UPDATE => state.registers[offset as usize / 4] = value,
            _ => {
                let index = usize::try_from(offset / 4).expect("LP UART register index fits");
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
        let mut state = self.state.lock().expect("ESP LP UART lock poisoned");
        *state = EspLpUartState::new();
        self.output.clear();
        let zero = SignalValue::from_u64(0, 8).expect("8-bit signal");
        let _ = self.hub.set(self.tx_signal, zero.clone(), SimTime::ZERO);
        let _ = self.hub.set(self.rx_signal, zero, SimTime::ZERO);
    }
}
