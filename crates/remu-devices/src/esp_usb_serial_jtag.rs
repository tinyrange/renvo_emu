use super::*;

/// Host-facing state for the ESP USB Serial/JTAG CDC-ACM data endpoint.
#[derive(Clone)]
pub struct EspUsbSerialJtagHandle {
    state: Arc<Mutex<EspUsbSerialJtagState>>,
}

#[derive(Default)]
struct EspUsbSerialJtagState {
    rx: VecDeque<u8>,
    tx_packet: Vec<u8>,
    output: Vec<u8>,
    input_queued: bool,
    host_connected: bool,
    sof_epoch: SimTime,
    interrupt_raw: u32,
    interrupt_enable: u32,
    registers: BTreeMap<u64, u32>,
}

const HOST_SCRIPT_COMPLETE_MARKER: &[u8] = b"__REMU_HOST_SCRIPT_COMPLETE__";

impl EspUsbSerialJtagHandle {
    /// Queues bytes sent by the deterministic host to the CDC-ACM OUT endpoint.
    pub fn queue_input(&self, bytes: &[u8]) {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        state.rx.extend(bytes.iter().copied());
        if !bytes.is_empty() {
            state.input_queued = true;
            state.interrupt_raw |= 1 << 2;
        }
    }

    /// Selects whether the deterministic USB host is attached.
    ///
    /// A connected host emits one start-of-frame indication every
    /// [`EspUsbSerialJtag::SOF_PERIOD_TICKS`] abstract ticks. The epoch is
    /// reset when the connection changes so tests can make the transition
    /// reproducible at a chosen simulation timestamp.
    pub fn set_host_connected(&self, connected: bool, at: SimTime) {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        state.host_connected = connected;
        state.sof_epoch = at;
        if !connected {
            state.interrupt_raw &= !EspUsbSerialJtag::SERIAL_SOF;
        }
    }

    /// Returns whether the deterministic host is currently attached.
    pub fn host_connected(&self) -> bool {
        self.state
            .lock()
            .expect("USB Serial/JTAG lock poisoned")
            .host_connected
    }

    /// Advances host USB scheduling and returns true on a newly asserted SOF.
    ///
    /// SOF is intentionally functional rather than clock accurate: one
    /// abstract tick is one completed architectural action, and the fixed
    /// period gives firmware a stable connected-host signal without tying the
    /// model to a particular CPU frequency.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        if !state.host_connected {
            state.interrupt_raw &= !EspUsbSerialJtag::SERIAL_SOF;
            return false;
        }

        let elapsed = now.ticks().saturating_sub(state.sof_epoch.ticks());
        if elapsed < EspUsbSerialJtag::SOF_PERIOD_TICKS {
            return false;
        }
        let periods = elapsed / EspUsbSerialJtag::SOF_PERIOD_TICKS;
        let advance = periods.saturating_mul(EspUsbSerialJtag::SOF_PERIOD_TICKS);
        state.sof_epoch = SimTime::from_ticks(state.sof_epoch.ticks().saturating_add(advance));
        let newly_asserted = state.interrupt_raw & EspUsbSerialJtag::SERIAL_SOF == 0;
        state.interrupt_raw |= EspUsbSerialJtag::SERIAL_SOF;
        newly_asserted
    }

    /// Returns all bytes transmitted to the deterministic CDC-ACM host.
    pub fn output(&self) -> Vec<u8> {
        self.state
            .lock()
            .expect("USB Serial/JTAG lock poisoned")
            .output
            .clone()
    }

    /// Reports that all queued raw-REPL input ran and its final prompt was flushed.
    pub fn input_complete(&self) -> bool {
        let state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        state.input_queued
            && state
                .output
                .windows(HOST_SCRIPT_COMPLETE_MARKER.len())
                .any(|window| window == HOST_SCRIPT_COMPLETE_MARKER)
            && state.output.ends_with(b"\x04\x04>")
    }

    /// Reports whether an enabled USB Serial/JTAG interrupt is pending.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        state.interrupt_raw & state.interrupt_enable != 0
    }

    /// Clears captured host output without changing endpoint configuration.
    pub fn clear_output(&self) {
        self.state
            .lock()
            .expect("USB Serial/JTAG lock poisoned")
            .output
            .clear();
    }
}

/// Functional ESP32-C6/S3 USB Serial/JTAG endpoint.
///
/// The model implements the software-visible CDC-ACM FIFO contract used by
/// ESP-IDF: EP1 byte access, endpoint availability, TX flush, and interrupt
/// raw/status/enable/clear registers. The deterministic host consumes every
/// flushed IN packet immediately.
pub struct EspUsbSerialJtag {
    name: String,
    state: Arc<Mutex<EspUsbSerialJtagState>>,
}

impl EspUsbSerialJtag {
    const EP1: u64 = 0x00;
    const EP1_CONF: u64 = 0x04;
    const INT_RAW: u64 = 0x08;
    const INT_ST: u64 = 0x0c;
    const INT_ENA: u64 = 0x10;
    const INT_CLR: u64 = 0x14;
    /// USB full-speed start-of-frame period in abstract simulation ticks.
    pub const SOF_PERIOD_TICKS: u64 = 1_000;
    const SERIAL_OUT_RECV_PKT: u32 = 1 << 2;
    const SERIAL_SOF: u32 = 1 << 1;
    const SERIAL_IN_EMPTY: u32 = 1 << 3;
    const INTERRUPT_MASK: u32 = 0x7ffff;
    const ENDPOINT_SIZE: usize = 64;

    /// Creates the peripheral and its host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, EspUsbSerialJtagHandle) {
        let state = Arc::new(Mutex::new(EspUsbSerialJtagState {
            // Hardware reset state: an empty IN endpoint is writable and its
            // raw empty indication is asserted. The deterministic host starts
            // connected so existing console tests model a plugged-in USB
            // cable unless they explicitly select disconnected mode.
            host_connected: true,
            interrupt_raw: Self::SERIAL_IN_EMPTY,
            ..EspUsbSerialJtagState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspUsbSerialJtagHandle { state },
        )
    }

    fn flush_tx(state: &mut EspUsbSerialJtagState) {
        state.output.append(&mut state.tx_packet);
        // The functional host takes the packet immediately, making EP1
        // writable again and producing the hardware's IN-empty indication.
        state.interrupt_raw |= Self::SERIAL_IN_EMPTY;
    }
}

impl Device for EspUsbSerialJtag {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, _width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        match offset {
            Self::EP1 => {
                let byte = state.rx.pop_front().unwrap_or_default();
                if state.rx.is_empty() {
                    state.interrupt_raw &= !Self::SERIAL_OUT_RECV_PKT;
                }
                Ok(u64::from(byte))
            }
            Self::EP1_CONF => {
                let rx_available = u32::from(!state.rx.is_empty()) << 2;
                // The deterministic host drains packets immediately, so the
                // 64-byte IN FIFO is always available to firmware.
                Ok(u64::from((1 << 1) | rx_available))
            }
            Self::INT_RAW => Ok(u64::from(state.interrupt_raw)),
            Self::INT_ST => Ok(u64::from(state.interrupt_raw & state.interrupt_enable)),
            Self::INT_ENA => Ok(u64::from(state.interrupt_enable)),
            Self::INT_CLR => Ok(0),
            _ => Ok(u64::from(
                state.registers.get(&offset).copied().unwrap_or_default(),
            )),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        let value = value as u32;
        match offset {
            Self::EP1 => {
                state.tx_packet.push(value as u8);
                if state.tx_packet.len() == Self::ENDPOINT_SIZE {
                    Self::flush_tx(&mut state);
                }
            }
            Self::EP1_CONF => {
                if value & 1 != 0 {
                    Self::flush_tx(&mut state);
                }
            }
            Self::INT_RAW => {
                // R/WTC/SS fields: writing one clears an asserted raw status.
                state.interrupt_raw &= !(value & Self::INTERRUPT_MASK);
            }
            Self::INT_ENA => state.interrupt_enable = value & Self::INTERRUPT_MASK,
            Self::INT_CLR => state.interrupt_raw &= !(value & Self::INTERRUPT_MASK),
            _ => {
                state.registers.insert(offset, value);
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("USB Serial/JTAG lock poisoned");
        *state = EspUsbSerialJtagState {
            host_connected: true,
            interrupt_raw: Self::SERIAL_IN_EMPTY,
            ..EspUsbSerialJtagState::default()
        };
    }
}
