use super::*;
use crate::RpI2cRegister;

const IC_INTR_BITS: u32 = 0x1fff;
const IC_INTR_MASK_RESET: u32 = 0x08ff;
const IC_FIFO_DEPTH: usize = 16;
// IC_CON[10] is the read-only STOP_DET_IF_MASTER_ACTIVE status bit.  The
// remaining low ten bits are writable while the controller is disabled.
const IC_CON_WRITABLE_BITS: u32 = 0x03ff;
const IC_MAX_SPEED_MODE: u32 = 2;
const IC_INTR_RX_UNDER: u32 = 1 << 0;
const IC_INTR_RX_OVER: u32 = 1 << 1;
const IC_INTR_RX_FULL: u32 = 1 << 2;
const IC_INTR_TX_OVER: u32 = 1 << 3;
const IC_INTR_TX_EMPTY: u32 = 1 << 4;
const IC_INTR_RX_DONE: u32 = 1 << 7;
const IC_INTR_ACTIVITY: u32 = 1 << 8;
const IC_INTR_STOP_DET: u32 = 1 << 9;
const IC_INTR_START_DET: u32 = 1 << 10;
const IC_INTR_RESTART_DET: u32 = 1 << 12;
const IC_INTR_FIFO_BITS: u32 = IC_INTR_RX_FULL | IC_INTR_TX_EMPTY;
const IC_INTR_SOFTWARE_CLEARABLE: u32 = IC_INTR_BITS & !IC_INTR_FIFO_BITS;

/// Alias for the native DW_apb_i2c register IDs used by RP2040.
pub type Rp2040I2cRegister = RpI2cRegister;

/// One byte observed on a functional I²C bus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum I2cEvent {
    /// A byte written to the selected target address.
    Write {
        /// Seven-bit target address in the low bits.
        address: u16,
        /// Data byte written on the bus.
        value: u8,
    },
    /// A byte returned by a queued target response.
    Read {
        /// Seven-bit target address in the low bits.
        address: u16,
        /// Data byte returned by the target.
        value: u8,
    },
}

/// Host-facing state for a functional RP2040 I²C controller.
#[derive(Clone, Default)]
pub struct I2cHandle {
    events: Arc<Mutex<Vec<I2cEvent>>>,
    responses: Arc<Mutex<BTreeMap<u16, VecDeque<u8>>>>,
}

impl I2cHandle {
    /// Queues response bytes for reads addressed to `address`.
    pub fn queue_read(&self, address: u16, bytes: &[u8]) {
        self.responses
            .lock()
            .expect("I2C response lock poisoned")
            .entry(address & 0x03ff)
            .or_default()
            .extend(bytes.iter().copied());
    }

    /// Returns the deterministic byte-level bus trace.
    pub fn events(&self) -> Vec<I2cEvent> {
        self.events.lock().expect("I2C event lock poisoned").clone()
    }

    /// Clears captured events and queued response bytes.
    pub fn clear(&self) {
        self.events.lock().expect("I2C event lock poisoned").clear();
        self.responses
            .lock()
            .expect("I2C response lock poisoned")
            .clear();
    }

    fn record(&self, event: I2cEvent) {
        self.events
            .lock()
            .expect("I2C event lock poisoned")
            .push(event);
    }

    fn response(&self, address: u16) -> u8 {
        let value = self
            .responses
            .lock()
            .expect("I2C response lock poisoned")
            .get_mut(&address)
            .and_then(VecDeque::pop_front)
            .unwrap_or(0xff);
        self.record(I2cEvent::Read { address, value });
        value
    }
}

struct FunctionalI2cState {
    registers: [u32; 0x100 / 4],
    raw_intr: u32,
    tx_fifo: VecDeque<u32>,
    rx_fifo: VecDeque<u8>,
    active: bool,
    handle: I2cHandle,
}

impl FunctionalI2cState {
    fn reset(handle: I2cHandle) -> Self {
        let mut registers = [0; 0x100 / 4];
        registers[RpI2cRegister::Control.offset() as usize / 4] = 0x65;
        registers[RpI2cRegister::TargetAddress.offset() as usize / 4] = 0x55;
        registers[RpI2cRegister::SlaveAddress.offset() as usize / 4] = 0x55;
        registers[RpI2cRegister::StandardSpeedHighCount.offset() as usize / 4] = 0x28;
        registers[RpI2cRegister::StandardSpeedLowCount.offset() as usize / 4] = 0x2f;
        registers[RpI2cRegister::FastSpeedHighCount.offset() as usize / 4] = 0x06;
        registers[RpI2cRegister::FastSpeedLowCount.offset() as usize / 4] = 0x0d;
        registers[RpI2cRegister::InterruptMask.offset() as usize / 4] = IC_INTR_MASK_RESET;
        registers[RpI2cRegister::SdaHold.offset() as usize / 4] = 1;
        registers[RpI2cRegister::SdaSetup.offset() as usize / 4] = 0x64;
        registers[RpI2cRegister::FastSpeedSpikeLength.offset() as usize / 4] = 7;
        registers[RpI2cRegister::AckGeneralCall.offset() as usize / 4] = 1;
        registers[RpI2cRegister::ComponentVersion.offset() as usize / 4] = 0x3230_312a;
        registers[RpI2cRegister::ComponentType.offset() as usize / 4] = 0x4457_0140;
        Self {
            registers,
            raw_intr: 0,
            tx_fifo: VecDeque::new(),
            rx_fifo: VecDeque::new(),
            active: false,
            handle,
        }
    }

    fn enabled(&self) -> bool {
        self.registers[RpI2cRegister::Enable.offset() as usize / 4] & 1 != 0
    }

    fn tx_cmd_blocked(&self) -> bool {
        self.registers[RpI2cRegister::Enable.offset() as usize / 4] & (1 << 2) != 0
    }

    fn update_fifo_interrupts(&mut self) {
        let rx_threshold = usize::try_from(
            self.registers[RpI2cRegister::ReceiveThreshold.offset() as usize / 4] & 0xff,
        )
        .expect("I2C threshold fits usize")
            + 1;
        let tx_threshold = usize::try_from(
            self.registers[RpI2cRegister::TransmitThreshold.offset() as usize / 4] & 0xff,
        )
        .expect("I2C threshold fits usize");
        if self.enabled() && self.rx_fifo.len() >= rx_threshold {
            self.raw_intr |= IC_INTR_RX_FULL;
        } else {
            self.raw_intr &= !IC_INTR_RX_FULL;
        }
        if self.enabled() && self.tx_fifo.len() <= tx_threshold {
            self.raw_intr |= IC_INTR_TX_EMPTY;
        } else {
            self.raw_intr &= !IC_INTR_TX_EMPTY;
        }
    }

    fn status(&self) -> u32 {
        let mut status = 0;
        if self.active {
            status |= 1;
        }
        if self.tx_fifo.len() < IC_FIFO_DEPTH {
            status |= 1 << 1;
        }
        if self.tx_fifo.is_empty() {
            status |= 1 << 2;
        }
        if !self.rx_fifo.is_empty() {
            status |= 1 << 3;
        }
        if self.rx_fifo.len() >= IC_FIFO_DEPTH {
            status |= 1 << 4;
        }
        // Functional execution drains the state machines immediately, so master/slave activity
        // remains low after each command has been observed.
        status
    }

    fn clear_software_interrupts(&mut self) {
        self.raw_intr &= !IC_INTR_SOFTWARE_CLEARABLE;
        self.update_fifo_interrupts();
    }

    fn reset_after_disable(&mut self) {
        self.tx_fifo.clear();
        self.rx_fifo.clear();
        self.active = false;
        self.raw_intr &= !IC_INTR_FIFO_BITS;
    }
}

/// Deterministic byte-oriented RP2040 DW_apb_i2c controller.
///
/// The model executes master DATA_CMD entries immediately in abstract time. A host response is
/// consumed for read commands, while writes and returned bytes are exposed through `I2cHandle`.
/// It follows the native reset values, masks, read-clear interrupt registers, sixteen-entry FIFO
/// limits, disabled-only configuration rules, APB narrow access lanes, and RP atomic aliases.
/// Pin-level arbitration, slave mode, DMA handshakes, and exact SCL timing remain outside this
/// functional slice.
///
/// Register names, reset values, and access masks follow the official
/// [RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf)
/// and [Pico SDK register definition](https://raw.githubusercontent.com/raspberrypi/pico-sdk/master/src/rp2040/hardware_regs/include/hardware/regs/i2c.h).
pub struct FunctionalI2c {
    name: String,
    state: Arc<Mutex<FunctionalI2cState>>,
}

impl FunctionalI2c {
    /// Creates a reset RP2040 controller and host handle.
    pub fn new(name: impl Into<String>) -> (Self, I2cHandle) {
        let handle = I2cHandle::default();
        let state = Arc::new(Mutex::new(FunctionalI2cState::reset(handle.clone())));
        (
            Self {
                name: name.into(),
                state,
            },
            handle,
        )
    }

    fn atomic_update(alias: u64, current: u32, value: u32) -> u32 {
        match alias {
            0 => value,
            1 => current ^ value,
            2 => current | value,
            3 => current & !value,
            _ => unreachable!("RP2040 I2C atomic alias is two bits"),
        }
    }

    fn replicated_write(value: u64, width: AccessWidth) -> Result<u32, DeviceError> {
        match width {
            AccessWidth::Byte => {
                let value = u32::try_from(value & 0xff).expect("I2C byte value fits");
                Ok(value | value << 8 | value << 16 | value << 24)
            }
            AccessWidth::HalfWord => {
                let value = u32::try_from(value & 0xffff).expect("I2C halfword value fits");
                Ok(value | value << 16)
            }
            AccessWidth::Word => u32::try_from(value & u64::from(u32::MAX))
                .map_err(|_| DeviceError::new("RP2040 I2C value overflow")),
            AccessWidth::DoubleWord => Err(DeviceError::new(
                "RP2040 I2C does not support 64-bit access",
            )),
        }
    }

    fn read_lane(value: u32, offset: u64, width: AccessWidth) -> Result<u64, DeviceError> {
        match width {
            AccessWidth::Byte => Ok(u64::from((value >> ((offset & 3) * 8)) & 0xff)),
            AccessWidth::HalfWord => Ok(u64::from((value >> ((offset & 2) * 8)) & 0xffff)),
            AccessWidth::Word => Ok(u64::from(value)),
            AccessWidth::DoubleWord => Err(DeviceError::new(
                "RP2040 I2C does not support 64-bit access",
            )),
        }
    }

    fn complete_command(state: &mut FunctionalI2cState, command: u32) -> Result<(), DeviceError> {
        let address =
            (state.registers[RpI2cRegister::TargetAddress.offset() as usize / 4] & 0x03ff) as u16;
        if !state.active {
            state.active = true;
            state.raw_intr |= IC_INTR_START_DET | IC_INTR_ACTIVITY;
        } else if command & (1 << 10) != 0 {
            state.raw_intr |= IC_INTR_RESTART_DET | IC_INTR_ACTIVITY;
        }
        if command & (1 << 8) != 0 {
            if state.rx_fifo.len() >= IC_FIFO_DEPTH {
                if state.registers[RpI2cRegister::Control.offset() as usize / 4] & (1 << 9) == 0 {
                    state.raw_intr |= IC_INTR_RX_OVER;
                }
            } else {
                let value = state.handle.response(address);
                state.rx_fifo.push_back(value);
                state.raw_intr |= IC_INTR_RX_DONE;
            }
        } else {
            state.handle.record(I2cEvent::Write {
                address,
                value: command as u8,
            });
        }
        if command & (1 << 9) != 0 {
            state.active = false;
            state.raw_intr |= IC_INTR_STOP_DET;
        }
        state.update_fifo_interrupts();
        Ok(())
    }

    fn drain_tx_fifo(state: &mut FunctionalI2cState) -> Result<(), DeviceError> {
        if !state.enabled() || state.tx_cmd_blocked() {
            state.update_fifo_interrupts();
            return Ok(());
        }
        while let Some(command) = state.tx_fifo.pop_front() {
            Self::complete_command(state, command)?;
        }
        state.update_fifo_interrupts();
        Ok(())
    }
}

impl Device for FunctionalI2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width == AccessWidth::DoubleWord {
            return Err(DeviceError::new(
                "RP2040 I2C does not support 64-bit access",
            ));
        }
        let register_offset = (offset & 0x0fff) & !3;
        let register = RpI2cRegister::from_offset(register_offset)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?;
        let mut state = self.state.lock().expect("I2C lock poisoned");
        let value = match register {
            RpI2cRegister::DataCommand => {
                let value = state.rx_fifo.pop_front().map_or_else(
                    || {
                        state.raw_intr |= IC_INTR_RX_UNDER;
                        0
                    },
                    u32::from,
                );
                state.update_fifo_interrupts();
                value
            }
            RpI2cRegister::InterruptStatus => {
                state.raw_intr & state.registers[RpI2cRegister::InterruptMask.offset() as usize / 4]
            }
            RpI2cRegister::RawInterruptStatus => state.raw_intr,
            RpI2cRegister::ClearInterrupt => {
                let value = u32::from(state.raw_intr != 0);
                state.clear_software_interrupts();
                value
            }
            RpI2cRegister::ClearReceiveUnderflow => {
                let value = u32::from(state.raw_intr & IC_INTR_RX_UNDER != 0);
                state.raw_intr &= !IC_INTR_RX_UNDER;
                value
            }
            RpI2cRegister::ClearReceiveOverflow => {
                let value = u32::from(state.raw_intr & IC_INTR_RX_OVER != 0);
                state.raw_intr &= !IC_INTR_RX_OVER;
                value
            }
            RpI2cRegister::ClearTransmitOverflow => {
                let value = u32::from(state.raw_intr & IC_INTR_TX_OVER != 0);
                state.raw_intr &= !IC_INTR_TX_OVER;
                value
            }
            RpI2cRegister::ClearReadRequest => 0,
            RpI2cRegister::ClearTransmitAbort => 0,
            RpI2cRegister::ClearReceiveDone => {
                let value = u32::from(state.raw_intr & IC_INTR_RX_DONE != 0);
                state.raw_intr &= !IC_INTR_RX_DONE;
                value
            }
            RpI2cRegister::ClearActivity => {
                let value = u32::from(state.raw_intr & IC_INTR_ACTIVITY != 0);
                if !state.active {
                    state.raw_intr &= !IC_INTR_ACTIVITY;
                }
                value
            }
            RpI2cRegister::ClearStopDetected => {
                let value = u32::from(state.raw_intr & IC_INTR_STOP_DET != 0);
                state.raw_intr &= !IC_INTR_STOP_DET;
                value
            }
            RpI2cRegister::ClearStartDetected => {
                let value = u32::from(state.raw_intr & IC_INTR_START_DET != 0);
                state.raw_intr &= !IC_INTR_START_DET;
                value
            }
            RpI2cRegister::ClearGeneralCall => 0,
            RpI2cRegister::ClearRestartDetected => {
                let value = u32::from(state.raw_intr & IC_INTR_RESTART_DET != 0);
                state.raw_intr &= !IC_INTR_RESTART_DET;
                value
            }
            RpI2cRegister::Status => state.status(),
            RpI2cRegister::TransmitFifoLevel => state.tx_fifo.len() as u32,
            RpI2cRegister::ReceiveFifoLevel => state.rx_fifo.len() as u32,
            RpI2cRegister::EnableStatus => {
                state.registers[RpI2cRegister::Enable.offset() as usize / 4] & 1
            }
            RpI2cRegister::Control
            | RpI2cRegister::TargetAddress
            | RpI2cRegister::SlaveAddress
            | RpI2cRegister::StandardSpeedHighCount
            | RpI2cRegister::StandardSpeedLowCount
            | RpI2cRegister::FastSpeedHighCount
            | RpI2cRegister::FastSpeedLowCount
            | RpI2cRegister::FastSpeedSpikeLength
            | RpI2cRegister::InterruptMask
            | RpI2cRegister::ReceiveThreshold
            | RpI2cRegister::TransmitThreshold
            | RpI2cRegister::SdaHold
            | RpI2cRegister::TransmitAbortSource
            | RpI2cRegister::SlaveDataNackOnly
            | RpI2cRegister::DmaControl
            | RpI2cRegister::DmaTransmitLevel
            | RpI2cRegister::DmaReceiveLevel
            | RpI2cRegister::SdaSetup
            | RpI2cRegister::AckGeneralCall
            | RpI2cRegister::Enable
            | RpI2cRegister::ComponentParameter
            | RpI2cRegister::ComponentVersion
            | RpI2cRegister::ComponentType => state.registers[register_offset as usize / 4],
        };
        Self::read_lane(value, offset, width)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let register_offset = (offset & 0x0fff) & !3;
        let alias = (offset >> 12) & 3;
        let register = RpI2cRegister::from_offset(register_offset)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        let value = Self::replicated_write(value, width)?;
        let mut state = self.state.lock().expect("I2C lock poisoned");
        let index = register_offset as usize / 4;
        match register {
            RpI2cRegister::Control => {
                if !state.enabled() {
                    let value = Self::atomic_update(alias, state.registers[index], value);
                    let speed = match (value >> 1) & 0x3 {
                        1 => 1,
                        2 => 2,
                        // RP2040 is configured for standard and fast modes;
                        // DW_apb_i2c saturates an illegal speed to the
                        // maximum supported mode.
                        _ => IC_MAX_SPEED_MODE,
                    };
                    state.registers[index] = (value & IC_CON_WRITABLE_BITS & !0x6) | (speed << 1);
                }
            }
            RpI2cRegister::TargetAddress => {
                if !state.enabled() {
                    state.registers[index] =
                        Self::atomic_update(alias, state.registers[index], value) & 0x0fff;
                }
            }
            RpI2cRegister::SlaveAddress => {
                if !state.enabled() {
                    state.registers[index] =
                        Self::atomic_update(alias, state.registers[index], value) & 0x03ff;
                }
            }
            RpI2cRegister::DataCommand => {
                if alias != 0 {
                    return Err(DeviceError::new(
                        "RP2040 I2C atomic aliases are not valid for DATA_CMD",
                    ));
                }
                if state.tx_fifo.len() >= IC_FIFO_DEPTH {
                    state.raw_intr |= IC_INTR_TX_OVER;
                    return Ok(());
                }
                state.tx_fifo.push_back(value & 0x7ff);
                Self::drain_tx_fifo(&mut state)?;
            }
            RpI2cRegister::InterruptMask => {
                state.registers[index] =
                    Self::atomic_update(alias, state.registers[index], value) & IC_INTR_BITS;
            }
            RpI2cRegister::ReceiveThreshold | RpI2cRegister::TransmitThreshold => {
                state.registers[index] =
                    Self::atomic_update(alias, state.registers[index], value) & 0xff;
                state.update_fifo_interrupts();
            }
            RpI2cRegister::Enable => {
                let old = state.registers[index];
                let next = Self::atomic_update(alias, old, value) & 0x07;
                state.registers[index] = next;
                if next & 1 == 0 {
                    state.reset_after_disable();
                } else {
                    Self::drain_tx_fifo(&mut state)?;
                }
            }
            RpI2cRegister::StandardSpeedHighCount
            | RpI2cRegister::StandardSpeedLowCount
            | RpI2cRegister::FastSpeedHighCount
            | RpI2cRegister::FastSpeedLowCount => {
                if !state.enabled() {
                    let minimum = match register {
                        RpI2cRegister::StandardSpeedHighCount
                        | RpI2cRegister::FastSpeedHighCount => 6,
                        RpI2cRegister::StandardSpeedLowCount | RpI2cRegister::FastSpeedLowCount => {
                            8
                        }
                        _ => unreachable!(),
                    };
                    state.registers[index] =
                        Self::atomic_update(alias, state.registers[index], value)
                            .min(u32::from(u16::MAX))
                            .max(minimum);
                }
            }
            RpI2cRegister::SdaHold if !state.enabled() => {
                state.registers[index] =
                    Self::atomic_update(alias, state.registers[index], value) & 0x00ff_ffff;
            }
            RpI2cRegister::SlaveDataNackOnly => {
                if !state.enabled() {
                    state.registers[index] =
                        Self::atomic_update(alias, state.registers[index], value) & 1;
                }
            }
            RpI2cRegister::DmaControl | RpI2cRegister::AckGeneralCall => {
                let mask = match register {
                    RpI2cRegister::DmaControl => 3,
                    RpI2cRegister::AckGeneralCall => 1,
                    _ => unreachable!(),
                };
                state.registers[index] =
                    Self::atomic_update(alias, state.registers[index], value) & mask;
            }
            RpI2cRegister::DmaTransmitLevel | RpI2cRegister::DmaReceiveLevel => {
                state.registers[index] =
                    Self::atomic_update(alias, state.registers[index], value) & 0x0f;
            }
            RpI2cRegister::SdaSetup if !state.enabled() => {
                state.registers[index] = Self::atomic_update(alias, state.registers[index], value)
                    .min(u32::from(u8::MAX))
                    .max(2);
            }
            RpI2cRegister::FastSpeedSpikeLength if !state.enabled() => {
                state.registers[index] = Self::atomic_update(alias, state.registers[index], value)
                    .min(u32::from(u8::MAX))
                    .max(1);
            }
            // These configuration registers ignore writes while enabled.
            RpI2cRegister::SdaHold
            | RpI2cRegister::SdaSetup
            | RpI2cRegister::FastSpeedSpikeLength => {}
            RpI2cRegister::ClearInterrupt
            | RpI2cRegister::ClearReceiveUnderflow
            | RpI2cRegister::ClearReceiveOverflow
            | RpI2cRegister::ClearTransmitOverflow
            | RpI2cRegister::ClearReadRequest
            | RpI2cRegister::ClearTransmitAbort
            | RpI2cRegister::ClearReceiveDone
            | RpI2cRegister::ClearActivity
            | RpI2cRegister::ClearStopDetected
            | RpI2cRegister::ClearStartDetected
            | RpI2cRegister::ClearGeneralCall
            | RpI2cRegister::ClearRestartDetected
            | RpI2cRegister::InterruptStatus
            | RpI2cRegister::RawInterruptStatus
            | RpI2cRegister::Status
            | RpI2cRegister::TransmitFifoLevel
            | RpI2cRegister::ReceiveFifoLevel
            | RpI2cRegister::EnableStatus
            | RpI2cRegister::TransmitAbortSource
            | RpI2cRegister::ComponentParameter
            | RpI2cRegister::ComponentVersion
            | RpI2cRegister::ComponentType => {
                return Err(DeviceError::new("RP2040 I2C register is read-only"));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("I2C lock poisoned");
        let handle = state.handle.clone();
        *state = FunctionalI2cState::reset(handle);
    }
}
