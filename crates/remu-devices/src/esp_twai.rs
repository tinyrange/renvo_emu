use super::*;

const REGISTER_BYTES: usize = 0x94;
const MODE: u64 = 0x00;
const COMMAND: u64 = 0x04;
const STATUS: u64 = 0x08;
const INTERRUPT: u64 = 0x0c;
const INTERRUPT_ENABLE: u64 = 0x10;
const DATA_BASE: u64 = 0x40;
const DATA_END: u64 = 0x70;

const COMMAND_TX_REQUEST: u32 = 1 << 0;
const COMMAND_ABORT_TX: u32 = 1 << 1;
const COMMAND_RELEASE_BUFFER: u32 = 1 << 2;
const COMMAND_CLEAR_DATA_OVERRUN: u32 = 1 << 3;
const COMMAND_SELF_RX_REQUEST: u32 = 1 << 4;

const STATUS_RECEIVE_BUFFER: u32 = 1 << 0;
const STATUS_DATA_OVERRUN: u32 = 1 << 1;
const STATUS_TRANSMIT_BUFFER_RELEASED: u32 = 1 << 2;
const STATUS_TRANSMISSION_COMPLETE: u32 = 1 << 3;

const INTERRUPT_RECEIVE: u32 = 1 << 0;
const INTERRUPT_TRANSMIT: u32 = 1 << 1;
const INTERRUPT_DATA_OVERRUN: u32 = 1 << 3;
const INTERRUPT_ENABLE_MASK: u32 = 0x1ef;

#[derive(Default)]
struct EspTwaiState {
    registers: Vec<u32>,
    rx_frames: VecDeque<Vec<u8>>,
    tx_frames: Vec<Vec<u8>>,
}

impl EspTwaiState {
    fn new() -> Self {
        let mut state = Self {
            registers: vec![0; REGISTER_BYTES / 4],
            ..Self::default()
        };
        // The controller powers up in reset mode. Buffer-released and
        // transmission-complete are asserted only after a command completes.
        state.registers[MODE as usize / 4] = 1;
        state
    }

    fn status(&mut self) -> u32 {
        let mut status = self.registers[STATUS as usize / 4];
        if self.rx_frames.is_empty() {
            status &= !STATUS_RECEIVE_BUFFER;
        } else {
            status |= STATUS_RECEIVE_BUFFER;
        }
        self.registers[STATUS as usize / 4] = status;
        status
    }

    fn refresh_receive_interrupt(&mut self) {
        if self.rx_frames.is_empty() {
            self.registers[INTERRUPT as usize / 4] &= !INTERRUPT_RECEIVE;
        } else {
            self.registers[INTERRUPT as usize / 4] |= INTERRUPT_RECEIVE;
        }
    }

    fn frame_from_data(&self) -> Vec<u8> {
        (0..13)
            .map(|index| self.registers[(DATA_BASE as usize / 4) + index] as u8)
            .collect()
    }

    fn queue_frame(&mut self, frame: &[u8]) {
        let mut padded = vec![0; 13];
        let length = frame.len().min(padded.len());
        padded[..length].copy_from_slice(&frame[..length]);
        self.rx_frames.push_back(padded);
        self.refresh_receive_interrupt();
    }
}

/// Host-facing ESP32-C6 TWAI controller handle.
#[derive(Clone)]
pub struct EspTwaiHandle {
    state: Arc<Mutex<EspTwaiState>>,
}

impl EspTwaiHandle {
    /// Queues one CAN/TWAI data-register frame for firmware to receive.
    ///
    /// The hardware exposes thirteen byte registers. The functional model
    /// copies shorter host frames into that window and pads the remainder with
    /// zeroes, keeping register reads deterministic.
    pub fn queue_rx(&self, frame: &[u8]) {
        self.state
            .lock()
            .expect("ESP TWAI lock poisoned")
            .queue_frame(frame);
    }

    /// Returns and clears frames submitted by firmware with TX_REQUEST.
    pub fn take_tx_frames(&self) -> Vec<Vec<u8>> {
        let mut state = self.state.lock().expect("ESP TWAI lock poisoned");
        core::mem::take(&mut state.tx_frames)
    }

    /// Reports whether firmware has a frame waiting in the receive buffer.
    pub fn rx_available(&self) -> bool {
        !self
            .state
            .lock()
            .expect("ESP TWAI lock poisoned")
            .rx_frames
            .is_empty()
    }
}

/// Functional ESP32-C6 TWAI0/TWAI1 controller.
///
/// This models the native register window used by the ESP-IDF TWAI driver:
/// data-register writes form a thirteen-byte frame, `TX_REQUEST` exposes it to
/// the host, and `SELF_RX_REQUEST` loops it back into the receive FIFO. Host
/// frames can be queued through [`EspTwaiHandle`]. Arbitration, bit timing,
/// error confinement, and physical-bus contention are intentionally outside
/// this functional baseline.
pub struct EspTwai {
    name: String,
    state: Arc<Mutex<EspTwaiState>>,
    hub: SignalHub,
    tx_signal: SignalId,
    rx_signal: SignalId,
}

impl EspTwai {
    /// Creates a TWAI controller and host-facing frame handle.
    pub fn new(
        name: impl Into<String>,
        signal_prefix: &str,
        hub: SignalHub,
    ) -> Result<(Self, EspTwaiHandle), SignalError> {
        let tx_signal = hub.declare(
            format!("{signal_prefix}.tx"),
            SignalValue::from_u64(0, 8)?,
            Some("last transmitted TWAI data byte".to_string()),
        )?;
        let rx_signal = hub.declare(
            format!("{signal_prefix}.rx"),
            SignalValue::from_u64(0, 8)?,
            Some("last received TWAI data byte".to_string()),
        )?;
        let state = Arc::new(Mutex::new(EspTwaiState::new()));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
                hub,
                tx_signal,
                rx_signal,
            },
            EspTwaiHandle { state },
        ))
    }

    fn emit(&self, signal: SignalId, value: u8, at: SimTime) -> Result<(), DeviceError> {
        let value = SignalValue::from_u64(u64::from(value), 8)
            .map_err(|error| DeviceError::new(format!("{} signal value: {error}", self.name)))?;
        self.hub
            .set(signal, value, at)
            .map_err(|error| DeviceError::new(format!("{} signal update: {error}", self.name)))
    }

    fn transmit(
        &self,
        state: &mut EspTwaiState,
        self_receive: bool,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let frame = state.frame_from_data();
        state.tx_frames.push(frame.clone());
        state.registers[STATUS as usize / 4] |=
            STATUS_TRANSMIT_BUFFER_RELEASED | STATUS_TRANSMISSION_COMPLETE;
        state.registers[INTERRUPT as usize / 4] |= INTERRUPT_TRANSMIT;
        for byte in frame.iter().copied() {
            self.emit(self.tx_signal, byte, at)?;
        }
        if self_receive {
            state.queue_frame(&frame);
        }
        Ok(())
    }
}

impl Device for EspTwai {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP TWAI requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP TWAI lock poisoned");
        let value =
            if offset == STATUS {
                u64::from(state.status())
            } else if offset == INTERRUPT {
                let value = state.registers[INTERRUPT as usize / 4]
                    & state.registers[INTERRUPT_ENABLE as usize / 4]
                    & INTERRUPT_ENABLE_MASK;
                // Reading clears edge/status interrupts except receive, which
                // remains asserted while the RX FIFO is non-empty.
                state.registers[INTERRUPT as usize / 4] &= !(value & !INTERRUPT_RECEIVE);
                u64::from(value)
            } else if offset == INTERRUPT_ENABLE {
                u64::from(state.registers[INTERRUPT_ENABLE as usize / 4])
            } else if (DATA_BASE..=DATA_END).contains(&offset) {
                let byte = usize::try_from((offset - DATA_BASE) / 4).expect("TWAI data index fits");
                let value = state
                    .rx_frames
                    .front()
                    .and_then(|frame| frame.get(byte))
                    .copied()
                    .unwrap_or_default();
                drop(state);
                self.emit(self.rx_signal, value, at)?;
                return Ok(u64::from(value));
            } else {
                let index = usize::try_from(offset / 4).expect("TWAI register index fits");
                u64::from(*state.registers.get(index).ok_or_else(|| {
                    DeviceError::new(format!("{} read at {offset:#x}", self.name))
                })?)
            };
        Ok(value)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP TWAI requires aligned word access"));
        }
        let mut state = self.state.lock().expect("ESP TWAI lock poisoned");
        if (DATA_BASE..=DATA_END).contains(&offset) {
            let index = usize::try_from((offset - DATA_BASE) / 4).expect("TWAI data index fits");
            state.registers[(DATA_BASE as usize / 4) + index] = (value & 0xff) as u32;
            return Ok(());
        }
        match offset {
            COMMAND => {
                let command = value as u32;
                if command & COMMAND_ABORT_TX != 0 {
                    state.registers[STATUS as usize / 4] |= STATUS_TRANSMIT_BUFFER_RELEASED;
                }
                if command & COMMAND_CLEAR_DATA_OVERRUN != 0 {
                    state.registers[STATUS as usize / 4] &= !STATUS_DATA_OVERRUN;
                    state.registers[INTERRUPT as usize / 4] &= !INTERRUPT_DATA_OVERRUN;
                }
                if command & COMMAND_RELEASE_BUFFER != 0 {
                    state.rx_frames.pop_front();
                    state.refresh_receive_interrupt();
                }
                if command & COMMAND_TX_REQUEST != 0 {
                    self.transmit(&mut state, command & COMMAND_SELF_RX_REQUEST != 0, at)?;
                }
            }
            INTERRUPT => {}
            INTERRUPT_ENABLE => {
                state.registers[INTERRUPT_ENABLE as usize / 4] =
                    value as u32 & INTERRUPT_ENABLE_MASK;
            }
            MODE => state.registers[MODE as usize / 4] = value as u32 & 0x0f,
            _ => {
                let index = usize::try_from(offset / 4).expect("TWAI register index fits");
                let register = state.registers.get_mut(index).ok_or_else(|| {
                    DeviceError::new(format!("{} write at {offset:#x}", self.name))
                })?;
                *register = value as u32;
            }
        }
        state.status();
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ESP TWAI lock poisoned");
        *state = EspTwaiState::new();
        let _ = self.hub.set(
            self.tx_signal,
            SignalValue::from_u64(0, 8).expect("8-bit signal"),
            SimTime::ZERO,
        );
        let _ = self.hub.set(
            self.rx_signal,
            SignalValue::from_u64(0, 8).expect("8-bit signal"),
            SimTime::ZERO,
        );
    }
}
