use super::*;

const REGISTER_BYTES: usize = 0x80;
const MODE: u64 = 0x00;
const COMMAND: u64 = 0x04;
const STATUS: u64 = 0x08;
const INTERRUPT: u64 = 0x0c;
const INTERRUPT_ENABLE: u64 = 0x10;
const BUS_TIMING_0: u64 = 0x18;
const BUS_TIMING_1: u64 = 0x1c;
const ARBITRATION_LOST_CAPTURE: u64 = 0x2c;
const ERROR_CODE_CAPTURE: u64 = 0x30;
const ERROR_WARNING_LIMIT: u64 = 0x34;
const RX_ERROR_COUNTER: u64 = 0x38;
const TX_ERROR_COUNTER: u64 = 0x3c;
const DATA_BASE: u64 = 0x40;
const DATA_END: u64 = 0x70;
const RX_MESSAGE_COUNTER: u64 = 0x74;
const CLOCK_DIVIDER: u64 = 0x7c;

const MODE_MASK: u32 = 0x0000_000f;
const MODE_RESET: u32 = 1 << 0;
const COMMAND_MASK: u32 = 0x0000_001f;
const STATUS_MASK: u32 = 0x0000_01ff;
const INTERRUPT_MASK: u32 = 0x0000_00ef;
const BUS_TIMING_0_MASK: u32 = 0x0000_dfff;
const BUS_TIMING_1_MASK: u32 = 0x0000_00ff;
const ARBITRATION_LOST_CAPTURE_MASK: u32 = 0x0000_001f;
const ERROR_CODE_CAPTURE_MASK: u32 = 0x0000_00ff;
const ERROR_WARNING_LIMIT_MASK: u32 = 0x0000_00ff;
const ERROR_COUNTER_MASK: u32 = 0x0000_00ff;
const DATA_MASK: u32 = 0x0000_00ff;
const RX_MESSAGE_COUNTER_MASK: u32 = 0x0000_007f;
const CLOCK_DIVIDER_MASK: u32 = 0x0000_01ff;

/// Named ESP32-S3 TWAI register IDs covered by the functional model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32s3TwaiRegister {
    /// Mode and reset control.
    Mode,
    /// Transmission/reception command strobes.
    Command,
    /// Controller status.
    Status,
    /// Raw interrupt status.
    Interrupt,
    /// Interrupt enables.
    InterruptEnable,
    /// Baud-rate prescaler and synchronisation jump width.
    BusTiming0,
    /// Time-segment and sampling configuration.
    BusTiming1,
    /// Arbitration-lost capture.
    ArbitrationLostCapture,
    /// Error-code capture.
    ErrorCodeCapture,
    /// Error-warning threshold.
    ErrorWarningLimit,
    /// Receive error counter.
    RxErrorCounter,
    /// Transmit error counter.
    TxErrorCounter,
    /// Transmit/receive data byte window.
    Data(usize),
    /// Receive message counter.
    RxMessageCounter,
    /// Clock-divider and clock-output configuration.
    ClockDivider,
}

impl Esp32s3TwaiRegister {
    /// Returns the native byte offset of this register ID.
    pub const fn offset(self) -> u64 {
        match self {
            Self::Mode => MODE,
            Self::Command => COMMAND,
            Self::Status => STATUS,
            Self::Interrupt => INTERRUPT,
            Self::InterruptEnable => INTERRUPT_ENABLE,
            Self::BusTiming0 => BUS_TIMING_0,
            Self::BusTiming1 => BUS_TIMING_1,
            Self::ArbitrationLostCapture => ARBITRATION_LOST_CAPTURE,
            Self::ErrorCodeCapture => ERROR_CODE_CAPTURE,
            Self::ErrorWarningLimit => ERROR_WARNING_LIMIT,
            Self::RxErrorCounter => RX_ERROR_COUNTER,
            Self::TxErrorCounter => TX_ERROR_COUNTER,
            Self::Data(index) => DATA_BASE + (index as u64) * 4,
            Self::RxMessageCounter => RX_MESSAGE_COUNTER,
            Self::ClockDivider => CLOCK_DIVIDER,
        }
    }

    /// Converts a modeled native offset into a named register ID.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        if offset >= DATA_BASE && offset <= DATA_END {
            let index = (offset - DATA_BASE) / 4;
            if index < 13 {
                return Some(Self::Data(index as usize));
            }
        }
        Some(match offset {
            MODE => Self::Mode,
            COMMAND => Self::Command,
            STATUS => Self::Status,
            INTERRUPT => Self::Interrupt,
            INTERRUPT_ENABLE => Self::InterruptEnable,
            BUS_TIMING_0 => Self::BusTiming0,
            BUS_TIMING_1 => Self::BusTiming1,
            ARBITRATION_LOST_CAPTURE => Self::ArbitrationLostCapture,
            ERROR_CODE_CAPTURE => Self::ErrorCodeCapture,
            ERROR_WARNING_LIMIT => Self::ErrorWarningLimit,
            RX_ERROR_COUNTER => Self::RxErrorCounter,
            TX_ERROR_COUNTER => Self::TxErrorCounter,
            RX_MESSAGE_COUNTER => Self::RxMessageCounter,
            CLOCK_DIVIDER => Self::ClockDivider,
            _ => return None,
        })
    }
}

const COMMAND_TX_REQUEST: u32 = 1 << 0;
const COMMAND_ABORT_TX: u32 = 1 << 1;
const COMMAND_RELEASE_BUFFER: u32 = 1 << 2;
const COMMAND_CLEAR_DATA_OVERRUN: u32 = 1 << 3;
const COMMAND_SELF_RX_REQUEST: u32 = 1 << 4;

const STATUS_RECEIVE_BUFFER: u32 = 1 << 0;
const STATUS_DATA_OVERRUN: u32 = 1 << 1;
const STATUS_TRANSMIT_BUFFER_RELEASED: u32 = 1 << 2;
const STATUS_TRANSMISSION_COMPLETE: u32 = 1 << 3;
const STATUS_RECEIVE: u32 = 1 << 4;
const STATUS_TRANSMIT: u32 = 1 << 5;

const INTERRUPT_RECEIVE: u32 = 1 << 0;
const INTERRUPT_TRANSMIT: u32 = 1 << 1;
const INTERRUPT_DATA_OVERRUN: u32 = 1 << 3;

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
        state.registers[MODE as usize / 4] = MODE_RESET;
        state.registers[STATUS as usize / 4] =
            STATUS_TRANSMIT_BUFFER_RELEASED | STATUS_TRANSMISSION_COMPLETE;
        state
    }

    fn status(&mut self) -> u32 {
        let mut status = self.registers[STATUS as usize / 4];
        if self.rx_frames.is_empty() {
            status &= !STATUS_RECEIVE_BUFFER;
            status &= !STATUS_RECEIVE;
        } else {
            status |= STATUS_RECEIVE_BUFFER;
            status |= STATUS_RECEIVE;
        }
        self.registers[STATUS as usize / 4] = status & STATUS_MASK;
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
        if !self.rx_frames.is_empty() {
            self.registers[STATUS as usize / 4] |= STATUS_DATA_OVERRUN;
            self.registers[INTERRUPT as usize / 4] |= INTERRUPT_DATA_OVERRUN;
            return;
        }
        let mut padded = vec![0; 13];
        let length = frame.len().min(padded.len());
        padded[..length].copy_from_slice(&frame[..length]);
        self.rx_frames.push_back(padded);
        self.registers[STATUS as usize / 4] |= STATUS_RECEIVE;
        self.refresh_receive_interrupt();
    }
}

/// Host-facing ESP32 TWAI controller handle.
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

/// Functional classic ESP32-S3/C6 TWAI controller.
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
            STATUS_TRANSMIT | STATUS_TRANSMIT_BUFFER_RELEASED | STATUS_TRANSMISSION_COMPLETE;
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
        match Esp32s3TwaiRegister::from_offset(offset) {
            Some(Esp32s3TwaiRegister::Mode) => {
                Ok(u64::from(state.registers[MODE as usize / 4] & MODE_MASK))
            }
            Some(Esp32s3TwaiRegister::Command) => {
                Err(DeviceError::new("ESP TWAI COMMAND is write-only"))
            }
            Some(Esp32s3TwaiRegister::Status) => Ok(u64::from(state.status())),
            Some(Esp32s3TwaiRegister::Interrupt) => Ok(u64::from(
                state.registers[INTERRUPT as usize / 4] & INTERRUPT_MASK,
            )),
            Some(Esp32s3TwaiRegister::InterruptEnable) => Ok(u64::from(
                state.registers[INTERRUPT_ENABLE as usize / 4] & INTERRUPT_MASK,
            )),
            Some(Esp32s3TwaiRegister::BusTiming0) => Ok(u64::from(
                state.registers[BUS_TIMING_0 as usize / 4] & BUS_TIMING_0_MASK,
            )),
            Some(Esp32s3TwaiRegister::BusTiming1) => Ok(u64::from(
                state.registers[BUS_TIMING_1 as usize / 4] & BUS_TIMING_1_MASK,
            )),
            Some(Esp32s3TwaiRegister::ArbitrationLostCapture) => Ok(u64::from(
                state.registers[ARBITRATION_LOST_CAPTURE as usize / 4]
                    & ARBITRATION_LOST_CAPTURE_MASK,
            )),
            Some(Esp32s3TwaiRegister::ErrorCodeCapture) => Ok(u64::from(
                state.registers[ERROR_CODE_CAPTURE as usize / 4] & ERROR_CODE_CAPTURE_MASK,
            )),
            Some(Esp32s3TwaiRegister::ErrorWarningLimit) => Ok(u64::from(
                state.registers[ERROR_WARNING_LIMIT as usize / 4] & ERROR_WARNING_LIMIT_MASK,
            )),
            Some(Esp32s3TwaiRegister::RxErrorCounter) => Ok(u64::from(
                state.registers[RX_ERROR_COUNTER as usize / 4] & ERROR_COUNTER_MASK,
            )),
            Some(Esp32s3TwaiRegister::TxErrorCounter) => Ok(u64::from(
                state.registers[TX_ERROR_COUNTER as usize / 4] & ERROR_COUNTER_MASK,
            )),
            Some(Esp32s3TwaiRegister::Data(index)) => {
                let value = state
                    .rx_frames
                    .front()
                    .and_then(|frame| frame.get(index))
                    .copied()
                    .unwrap_or_default();
                drop(state);
                self.emit(self.rx_signal, value, at)?;
                Ok(u64::from(value))
            }
            Some(Esp32s3TwaiRegister::RxMessageCounter) => Ok(u64::from(
                state.registers[RX_MESSAGE_COUNTER as usize / 4] & RX_MESSAGE_COUNTER_MASK,
            )),
            Some(Esp32s3TwaiRegister::ClockDivider) => Ok(u64::from(
                state.registers[CLOCK_DIVIDER as usize / 4] & CLOCK_DIVIDER_MASK,
            )),
            None => Err(DeviceError::new(format!(
                "{} read at unmodeled offset {offset:#x}",
                self.name
            ))),
        }
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
        let value =
            u32::try_from(value).map_err(|_| DeviceError::new("ESP TWAI value exceeds u32"))?;
        let mut state = self.state.lock().expect("ESP TWAI lock poisoned");
        match Esp32s3TwaiRegister::from_offset(offset) {
            Some(Esp32s3TwaiRegister::Mode) => {
                state.registers[MODE as usize / 4] = value & MODE_MASK;
            }
            Some(Esp32s3TwaiRegister::Command) => {
                let command = value & COMMAND_MASK;
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
            Some(Esp32s3TwaiRegister::Status) => {
                return Err(DeviceError::new("ESP TWAI STATUS is read-only"));
            }
            Some(Esp32s3TwaiRegister::Interrupt) => {
                state.registers[INTERRUPT as usize / 4] &= !(value & INTERRUPT_MASK);
            }
            Some(Esp32s3TwaiRegister::InterruptEnable) => {
                state.registers[INTERRUPT_ENABLE as usize / 4] = value & INTERRUPT_MASK;
            }
            Some(Esp32s3TwaiRegister::BusTiming0) => {
                state.registers[BUS_TIMING_0 as usize / 4] = value & BUS_TIMING_0_MASK;
            }
            Some(Esp32s3TwaiRegister::BusTiming1) => {
                state.registers[BUS_TIMING_1 as usize / 4] = value & BUS_TIMING_1_MASK;
            }
            Some(Esp32s3TwaiRegister::ArbitrationLostCapture) => {
                return Err(DeviceError::new(
                    "ESP TWAI arbitration capture is read-only",
                ));
            }
            Some(Esp32s3TwaiRegister::ErrorCodeCapture) => {
                return Err(DeviceError::new("ESP TWAI error capture is read-only"));
            }
            Some(Esp32s3TwaiRegister::ErrorWarningLimit) => {
                state.registers[ERROR_WARNING_LIMIT as usize / 4] =
                    value & ERROR_WARNING_LIMIT_MASK;
            }
            Some(Esp32s3TwaiRegister::RxErrorCounter) => {
                return Err(DeviceError::new("ESP TWAI RX error counter is read-only"));
            }
            Some(Esp32s3TwaiRegister::TxErrorCounter) => {
                return Err(DeviceError::new("ESP TWAI TX error counter is read-only"));
            }
            Some(Esp32s3TwaiRegister::Data(index)) => {
                state.registers[(DATA_BASE as usize / 4) + index] = value & DATA_MASK;
            }
            Some(Esp32s3TwaiRegister::RxMessageCounter) => {
                return Err(DeviceError::new("ESP TWAI RX message counter is read-only"));
            }
            Some(Esp32s3TwaiRegister::ClockDivider) => {
                state.registers[CLOCK_DIVIDER as usize / 4] = value & CLOCK_DIVIDER_MASK;
            }
            None => {
                return Err(DeviceError::new(format!(
                    "{} write at unmodeled offset {offset:#x}",
                    self.name
                )));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_ids_cover_only_the_documented_esp32s3_window() {
        let expected = [
            (Esp32s3TwaiRegister::Mode, 0x00),
            (Esp32s3TwaiRegister::Command, 0x04),
            (Esp32s3TwaiRegister::Status, 0x08),
            (Esp32s3TwaiRegister::Interrupt, 0x0c),
            (Esp32s3TwaiRegister::InterruptEnable, 0x10),
            (Esp32s3TwaiRegister::BusTiming0, 0x18),
            (Esp32s3TwaiRegister::BusTiming1, 0x1c),
            (Esp32s3TwaiRegister::ArbitrationLostCapture, 0x2c),
            (Esp32s3TwaiRegister::ErrorCodeCapture, 0x30),
            (Esp32s3TwaiRegister::ErrorWarningLimit, 0x34),
            (Esp32s3TwaiRegister::RxErrorCounter, 0x38),
            (Esp32s3TwaiRegister::TxErrorCounter, 0x3c),
            (Esp32s3TwaiRegister::Data(0), 0x40),
            (Esp32s3TwaiRegister::Data(12), 0x70),
            (Esp32s3TwaiRegister::RxMessageCounter, 0x74),
            (Esp32s3TwaiRegister::ClockDivider, 0x7c),
        ];
        for (register, offset) in expected {
            assert_eq!(register.offset(), offset);
            assert_eq!(Esp32s3TwaiRegister::from_offset(offset), Some(register));
        }
        assert_eq!(Esp32s3TwaiRegister::from_offset(0x14), None);
        assert_eq!(Esp32s3TwaiRegister::from_offset(0x78), None);
        assert_eq!(Esp32s3TwaiRegister::from_offset(0x80), None);
    }

    #[test]
    fn register_masks_reset_values_and_access_modes_follow_vendor_struct() {
        let hub = SignalHub::new();
        let (mut twai, _) = EspTwai::new("twai", "board.twai", hub).unwrap();
        assert_eq!(
            twai.read(MODE, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(MODE_RESET)
        );
        for (offset, expected) in [
            (MODE, MODE_MASK),
            (BUS_TIMING_0, BUS_TIMING_0_MASK),
            (BUS_TIMING_1, BUS_TIMING_1_MASK),
            (ERROR_WARNING_LIMIT, ERROR_WARNING_LIMIT_MASK),
            (CLOCK_DIVIDER, CLOCK_DIVIDER_MASK),
        ] {
            twai.write(
                offset,
                AccessWidth::Word,
                u64::from(u32::MAX),
                SimTime::ZERO,
            )
            .unwrap();
            assert_eq!(
                twai.read(offset, AccessWidth::Word, SimTime::ZERO).unwrap(),
                u64::from(expected)
            );
        }
        twai.write(
            INTERRUPT_ENABLE,
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            twai.read(INTERRUPT_ENABLE, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(INTERRUPT_MASK)
        );
        assert!(
            twai.read(COMMAND, AccessWidth::Word, SimTime::ZERO)
                .is_err()
        );
        assert!(
            twai.write(STATUS, AccessWidth::Word, 0, SimTime::ZERO)
                .is_err()
        );
        assert!(
            twai.write(
                ARBITRATION_LOST_CAPTURE,
                AccessWidth::Word,
                0,
                SimTime::ZERO
            )
            .is_err()
        );
        assert!(
            twai.write(RX_ERROR_COUNTER, AccessWidth::Word, 0, SimTime::ZERO)
                .is_err()
        );
        assert!(
            twai.write(
                CLOCK_DIVIDER,
                AccessWidth::Word,
                u64::from(u32::MAX) + 1,
                SimTime::ZERO
            )
            .is_err()
        );
    }

    #[test]
    fn self_reception_round_trips_the_native_data_window() {
        let hub = SignalHub::new();
        let (mut twai, handle) = EspTwai::new("twai", "board.twai", hub.clone()).unwrap();
        for (index, byte) in (0..13).map(|index| (index, index as u8 + 1)) {
            twai.write(
                DATA_BASE + u64::try_from(index).unwrap() * 4,
                AccessWidth::Word,
                u64::from(byte),
                SimTime::ZERO,
            )
            .unwrap();
        }
        twai.write(
            COMMAND,
            AccessWidth::Word,
            u64::from(COMMAND_TX_REQUEST | COMMAND_SELF_RX_REQUEST),
            SimTime::ZERO,
        )
        .unwrap();

        assert!(handle.rx_available());
        assert_eq!(handle.take_tx_frames(), vec![(1..=13).collect::<Vec<_>>()]);
        assert_eq!(
            twai.read(STATUS, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(
                STATUS_RECEIVE_BUFFER
                    | STATUS_RECEIVE
                    | STATUS_TRANSMIT
                    | STATUS_TRANSMIT_BUFFER_RELEASED
                    | STATUS_TRANSMISSION_COMPLETE
            )
        );
        assert_eq!(
            twai.read(DATA_BASE + 8, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            3
        );
        twai.write(
            COMMAND,
            AccessWidth::Word,
            u64::from(COMMAND_RELEASE_BUFFER),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.rx_available());
        assert!(
            hub.with_registry(|registry| registry.find("board.twai.tx"))
                .is_some()
        );
    }

    #[test]
    fn host_receive_sets_and_clears_receive_interrupt() {
        let hub = SignalHub::new();
        let (mut twai, handle) = EspTwai::new("twai", "board.twai", hub).unwrap();
        twai.write(
            INTERRUPT_ENABLE,
            AccessWidth::Word,
            u64::from(INTERRUPT_RECEIVE),
            SimTime::ZERO,
        )
        .unwrap();
        handle.queue_rx(&[0x12, 0x34]);
        assert_eq!(
            twai.read(INTERRUPT, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(INTERRUPT_RECEIVE)
        );
        twai.write(
            COMMAND,
            AccessWidth::Word,
            u64::from(COMMAND_RELEASE_BUFFER),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            twai.read(INTERRUPT, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
    }

    #[test]
    fn a_second_host_frame_latches_overrun_until_command_clear() {
        let hub = SignalHub::new();
        let (mut twai, handle) = EspTwai::new("twai", "board.twai", hub).unwrap();
        handle.queue_rx(&[0x12]);
        handle.queue_rx(&[0x34]);
        assert_eq!(
            twai.read(STATUS, AccessWidth::Word, SimTime::ZERO).unwrap()
                & u64::from(STATUS_DATA_OVERRUN),
            u64::from(STATUS_DATA_OVERRUN)
        );
        assert_eq!(
            twai.read(INTERRUPT, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(INTERRUPT_DATA_OVERRUN),
            u64::from(INTERRUPT_DATA_OVERRUN)
        );
        twai.write(
            COMMAND,
            AccessWidth::Word,
            u64::from(COMMAND_CLEAR_DATA_OVERRUN),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            twai.read(STATUS, AccessWidth::Word, SimTime::ZERO).unwrap()
                & u64::from(STATUS_DATA_OVERRUN),
            0
        );
        assert!(handle.rx_available());
    }
}
