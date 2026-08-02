use super::*;

const IC_INTR_MASKED_BITS: u32 = (1 << 15) - 1;

/// RP2350 DW_apb_i2c register identifiers shared by I2C0 and I2C1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum RpI2cRegister {
    /// I²C control register.
    Control = 0x00,
    /// Target address register.
    TargetAddress = 0x04,
    /// Slave address register.
    SlaveAddress = 0x08,
    /// Combined transmit/receive command register.
    DataCommand = 0x10,
    /// Masked interrupt status register.
    InterruptStatus = 0x2c,
    /// Interrupt mask register; one bits mask their corresponding source.
    InterruptMask = 0x30,
    /// Raw interrupt status register.
    RawInterruptStatus = 0x34,
    /// Receive FIFO threshold register.
    ReceiveThreshold = 0x38,
    /// Transmit FIFO threshold register.
    TransmitThreshold = 0x3c,
    /// Read-clear aggregate interrupt register.
    ClearInterrupt = 0x40,
    /// Read-clear receive-underflow register.
    ClearReceiveUnderflow = 0x44,
    /// Read-clear receive-overflow register.
    ClearReceiveOverflow = 0x48,
    /// Read-clear transmit-overflow register.
    ClearTransmitOverflow = 0x4c,
    /// Read-clear read-request register.
    ClearReadRequest = 0x50,
    /// Read-clear transmit-abort register.
    ClearTransmitAbort = 0x54,
    /// Read-clear receive-done register.
    ClearReceiveDone = 0x58,
    /// Read-clear activity register.
    ClearActivity = 0x5c,
    /// Read-clear stop-detect register.
    ClearStopDetected = 0x60,
    /// Read-clear start-detect register.
    ClearStartDetected = 0x64,
    /// Read-clear general-call register.
    ClearGeneralCall = 0x68,
    /// Enable and abort control register.
    Enable = 0x6c,
    /// Read-only controller status register.
    Status = 0x70,
    /// Transmit FIFO level register.
    TransmitFifoLevel = 0x74,
    /// Receive FIFO level register.
    ReceiveFifoLevel = 0x78,
    /// Read-only enable status register.
    EnableStatus = 0x9c,
    /// Read-clear restart-detect register.
    ClearRestartDetected = 0xa8,
}

impl RpI2cRegister {
    /// Converts a native register offset to a typed identifier.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::Control,
            0x04 => Self::TargetAddress,
            0x08 => Self::SlaveAddress,
            0x10 => Self::DataCommand,
            0x2c => Self::InterruptStatus,
            0x30 => Self::InterruptMask,
            0x34 => Self::RawInterruptStatus,
            0x38 => Self::ReceiveThreshold,
            0x3c => Self::TransmitThreshold,
            0x40 => Self::ClearInterrupt,
            0x44 => Self::ClearReceiveUnderflow,
            0x48 => Self::ClearReceiveOverflow,
            0x4c => Self::ClearTransmitOverflow,
            0x50 => Self::ClearReadRequest,
            0x54 => Self::ClearTransmitAbort,
            0x58 => Self::ClearReceiveDone,
            0x5c => Self::ClearActivity,
            0x60 => Self::ClearStopDetected,
            0x64 => Self::ClearStartDetected,
            0x68 => Self::ClearGeneralCall,
            0x6c => Self::Enable,
            0x70 => Self::Status,
            0x74 => Self::TransmitFifoLevel,
            0x78 => Self::ReceiveFifoLevel,
            0x9c => Self::EnableStatus,
            0xa8 => Self::ClearRestartDetected,
            _ => return None,
        })
    }

    /// Returns the native byte offset represented by this identifier.
    pub const fn offset(self) -> u64 {
        self as u64
    }
}

/// deterministic while making the byte stream visible to VCD consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpI2cEvent {
    /// A transaction started while the controller was idle.
    Start,
    /// A repeated-start command was issued during an active transaction.
    RepeatedStart,
    /// A byte was written to the selected seven-bit target.
    Write {
        /// Seven-bit target address (or the low ten bits when configured).
        address: u16,
        /// Data byte written to the target.
        value: u8,
    },
    /// A byte was returned to the controller's receive FIFO.
    Read {
        /// Seven-bit target address (or the low ten bits when configured).
        address: u16,
        /// Data byte returned by the deterministic host target.
        value: u8,
    },
    /// The command stream requested a STOP condition.
    Stop,
}

struct RpI2cState {
    con: u32,
    tar: u32,
    sar: u32,
    intr_mask: u32,
    rx_tl: u32,
    tx_tl: u32,
    enable: bool,
    raw_intr: u32,
    rx_fifo: VecDeque<u8>,
    queued_reads: BTreeMap<u16, VecDeque<u8>>,
    events: Vec<RpI2cEvent>,
    active: bool,
    byte_signal: SignalId,
    strobe_signal: SignalId,
    strobe: bool,
}

impl RpI2cState {
    fn new(byte_signal: SignalId, strobe_signal: SignalId) -> Self {
        Self {
            con: 0x65,
            tar: 0x55,
            sar: 0,
            intr_mask: IC_INTR_MASKED_BITS,
            rx_tl: 0,
            tx_tl: 0,
            enable: false,
            raw_intr: 0,
            rx_fifo: VecDeque::new(),
            queued_reads: BTreeMap::new(),
            events: Vec::new(),
            active: false,
            byte_signal,
            strobe_signal,
            strobe: false,
        }
    }

    fn reset(&mut self) {
        self.con = 0x65;
        self.tar = 0x55;
        self.sar = 0;
        self.intr_mask = IC_INTR_MASKED_BITS;
        self.rx_tl = 0;
        self.tx_tl = 0;
        self.enable = false;
        self.raw_intr = 0;
        self.rx_fifo.clear();
        self.queued_reads.clear();
        self.events.clear();
        self.active = false;
        self.strobe = false;
    }

    fn pending(&self) -> bool {
        self.raw_intr & !self.intr_mask != 0
    }
}

/// Scheduler and test-facing view of an RP2350 I²C controller.
#[derive(Clone)]
pub struct RpI2cHandle {
    state: Rc<RefCell<RpI2cState>>,
}

impl RpI2cHandle {
    /// Queues bytes returned by a deterministic host-side seven-bit target.
    pub fn queue_read(&self, address: u16, bytes: impl IntoIterator<Item = u8>) {
        self.state
            .borrow_mut()
            .queued_reads
            .entry(address & 0x03ff)
            .or_default()
            .extend(bytes);
    }

    /// Returns the logical transactions observed since reset or the last clear.
    pub fn events(&self) -> Vec<RpI2cEvent> {
        self.state.borrow().events.clone()
    }

    /// Clears the transaction history without disturbing controller state.
    pub fn clear_events(&self) {
        self.state.borrow_mut().events.clear();
    }

    /// Returns whether an enabled I²C interrupt is pending.
    pub fn pending(&self) -> bool {
        self.state.borrow().pending()
    }
}

/// Functional RP2350 I²C0/I²C1 host controller.
///
/// This models the RP2350's Synopsys DW_apb_i2c register contract closely
/// enough for SDK initialization and bounded firmware tests: controller
/// configuration, target selection, DATA_CMD read/write/STOP commands, RX
/// FIFO status, interrupt latches, and deterministic host responses.  It does
/// not yet model pin-level arbitration, slave mode, DMA, or clock-accurate
/// SCL/SDA timing.
pub struct RpI2c {
    name: String,
    state: Rc<RefCell<RpI2cState>>,
    hub: SignalHub,
}

impl RpI2c {
    const IC_DATA_CMD_READ: u32 = 1 << 8;
    const IC_DATA_CMD_STOP: u32 = 1 << 9;
    const IC_DATA_CMD_RESTART: u32 = 1 << 10;
    const IC_INTR_RX_UNDER: u32 = 1 << 0;
    const IC_INTR_RX_OVER: u32 = 1 << 1;
    const IC_INTR_RX_FULL: u32 = 1 << 2;
    const IC_INTR_TX_OVER: u32 = 1 << 3;
    const IC_INTR_TX_EMPTY: u32 = 1 << 4;
    const IC_INTR_RD_REQ: u32 = 1 << 5;
    const IC_INTR_TX_ABRT: u32 = 1 << 6;
    const IC_INTR_RX_DONE: u32 = 1 << 7;
    const IC_INTR_ACTIVITY: u32 = 1 << 8;
    const IC_INTR_STOP_DET: u32 = 1 << 9;
    const IC_INTR_START_DET: u32 = 1 << 10;
    const IC_INTR_GEN_CALL: u32 = 1 << 11;
    const IC_INTR_RESTART_DET: u32 = 1 << 12;
    /// Creates a reset controller and scheduler-facing handle.
    pub fn new(
        name: impl Into<String>,
        signal_path: &str,
        hub: SignalHub,
    ) -> Result<(Self, RpI2cHandle), SignalError> {
        let byte_signal = hub.declare(
            format!("{signal_path}.byte"),
            SignalValue::from_u64(0, 8)?,
            Some("Functional I²C transaction byte".to_owned()),
        )?;
        let strobe_signal = hub.declare(
            format!("{signal_path}.strobe"),
            SignalValue::from_u64(0, 1)?,
            Some("I²C transaction strobe".to_owned()),
        )?;
        let state = Rc::new(RefCell::new(RpI2cState::new(byte_signal, strobe_signal)));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
                hub,
            },
            RpI2cHandle { state },
        ))
    }

    fn publish_byte(
        &self,
        state: &mut RpI2cState,
        value: u8,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        state.strobe = !state.strobe;
        self.hub
            .set(
                state.byte_signal,
                SignalValue::from_u64(u64::from(value), 8)
                    .map_err(|error| DeviceError::new(error.to_string()))?,
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))?;
        self.hub
            .set(
                state.strobe_signal,
                SignalValue::from_u64(u64::from(state.strobe), 1)
                    .map_err(|error| DeviceError::new(error.to_string()))?,
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn clear_interrupt(state: &mut RpI2cState, mask: u32) {
        state.raw_intr &= !mask;
    }

    fn complete_command(
        &self,
        state: &mut RpI2cState,
        command: u32,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let address = (state.tar & 0x03ff) as u16;
        if !state.active {
            state.active = true;
            state.events.push(RpI2cEvent::Start);
            state.raw_intr |= Self::IC_INTR_START_DET | Self::IC_INTR_ACTIVITY;
        } else if command & Self::IC_DATA_CMD_RESTART != 0 {
            state.events.push(RpI2cEvent::RepeatedStart);
            state.raw_intr |= Self::IC_INTR_RESTART_DET | Self::IC_INTR_ACTIVITY;
        }
        let value = (command & 0xff) as u8;
        if command & Self::IC_DATA_CMD_READ != 0 {
            let value = state
                .queued_reads
                .get_mut(&address)
                .and_then(VecDeque::pop_front)
                .unwrap_or(0xff);
            if state.rx_fifo.len() >= 16 {
                state.raw_intr |= Self::IC_INTR_RX_OVER;
            } else {
                state.rx_fifo.push_back(value);
                state.events.push(RpI2cEvent::Read { address, value });
                state.raw_intr |= Self::IC_INTR_RX_DONE;
                if state.rx_fifo.len() > usize::try_from(state.rx_tl).unwrap_or(0) {
                    state.raw_intr |= Self::IC_INTR_RX_FULL;
                }
            }
            self.publish_byte(state, value, at)?;
        } else {
            state.events.push(RpI2cEvent::Write { address, value });
            state.raw_intr |= Self::IC_INTR_TX_EMPTY;
            self.publish_byte(state, value, at)?;
        }
        if command & Self::IC_DATA_CMD_STOP != 0 {
            state.active = false;
            state.events.push(RpI2cEvent::Stop);
            state.raw_intr |= Self::IC_INTR_STOP_DET;
        }
        Ok(())
    }
}

impl Device for RpI2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RP2350 I²C requires aligned word access"));
        }
        let register = RpI2cRegister::from_offset(offset & 0x0fff);
        let mut state = self.state.borrow_mut();
        let value = match register {
            Some(RpI2cRegister::Control) => state.con,
            Some(RpI2cRegister::TargetAddress) => state.tar,
            Some(RpI2cRegister::SlaveAddress) => state.sar,
            Some(RpI2cRegister::DataCommand) => {
                let value = state.rx_fifo.pop_front().map_or_else(
                    || {
                        state.raw_intr |= Self::IC_INTR_RX_UNDER;
                        0
                    },
                    u32::from,
                );
                if state.rx_fifo.len() <= usize::try_from(state.rx_tl).unwrap_or(0) {
                    state.raw_intr &= !Self::IC_INTR_RX_FULL;
                }
                value
            }
            Some(RpI2cRegister::InterruptStatus) => state.raw_intr & !state.intr_mask,
            Some(RpI2cRegister::InterruptMask) => state.intr_mask,
            Some(RpI2cRegister::RawInterruptStatus) => state.raw_intr,
            Some(RpI2cRegister::ReceiveThreshold) => state.rx_tl,
            Some(RpI2cRegister::TransmitThreshold) => state.tx_tl,
            Some(RpI2cRegister::ClearInterrupt) => {
                let value = state.raw_intr;
                state.raw_intr = 0;
                value
            }
            Some(RpI2cRegister::ClearReceiveUnderflow) => {
                let value = state.raw_intr & Self::IC_INTR_RX_UNDER;
                Self::clear_interrupt(&mut state, Self::IC_INTR_RX_UNDER);
                value
            }
            Some(RpI2cRegister::ClearReceiveOverflow) => {
                let value = state.raw_intr & Self::IC_INTR_RX_OVER;
                Self::clear_interrupt(&mut state, Self::IC_INTR_RX_OVER);
                value
            }
            Some(RpI2cRegister::ClearTransmitOverflow) => {
                let value = state.raw_intr & Self::IC_INTR_TX_OVER;
                Self::clear_interrupt(&mut state, Self::IC_INTR_TX_OVER);
                value
            }
            Some(RpI2cRegister::ClearReadRequest) => {
                let value = state.raw_intr & Self::IC_INTR_RD_REQ;
                Self::clear_interrupt(&mut state, Self::IC_INTR_RD_REQ);
                value
            }
            Some(RpI2cRegister::ClearTransmitAbort) => {
                let value = state.raw_intr & Self::IC_INTR_TX_ABRT;
                Self::clear_interrupt(&mut state, Self::IC_INTR_TX_ABRT);
                value
            }
            Some(RpI2cRegister::ClearReceiveDone) => {
                let value = state.raw_intr & Self::IC_INTR_RX_DONE;
                Self::clear_interrupt(&mut state, Self::IC_INTR_RX_DONE);
                value
            }
            Some(RpI2cRegister::ClearActivity) => {
                let value = state.raw_intr & Self::IC_INTR_ACTIVITY;
                if !state.active {
                    Self::clear_interrupt(&mut state, Self::IC_INTR_ACTIVITY);
                }
                value
            }
            Some(RpI2cRegister::ClearStopDetected) => {
                let value = state.raw_intr & Self::IC_INTR_STOP_DET;
                Self::clear_interrupt(&mut state, Self::IC_INTR_STOP_DET);
                value
            }
            Some(RpI2cRegister::ClearStartDetected) => {
                let value = state.raw_intr & Self::IC_INTR_START_DET;
                Self::clear_interrupt(&mut state, Self::IC_INTR_START_DET);
                value
            }
            Some(RpI2cRegister::ClearGeneralCall) => {
                let value = state.raw_intr & Self::IC_INTR_GEN_CALL;
                Self::clear_interrupt(&mut state, Self::IC_INTR_GEN_CALL);
                value
            }
            Some(RpI2cRegister::Enable) => u32::from(state.enable),
            Some(RpI2cRegister::Status) => {
                let mut status = 0;
                if state.active {
                    status |= 1 << 0;
                }
                // The model has no transmit FIFO backlog, so TFNF/TFE are
                // asserted both while enabled and in the documented reset/
                // disabled state.
                status |= (1 << 1) | (1 << 2);
                if !state.rx_fifo.is_empty() {
                    status |= 1 << 3;
                }
                if state.rx_fifo.len() >= 16 {
                    status |= 1 << 4;
                }
                if state.active {
                    status |= 1 << 5;
                }
                status
            }
            Some(RpI2cRegister::TransmitFifoLevel) => 0,
            Some(RpI2cRegister::ReceiveFifoLevel) => {
                u32::try_from(state.rx_fifo.len()).expect("I²C FIFO length fits")
            }
            Some(RpI2cRegister::EnableStatus) => u32::from(state.enable),
            Some(RpI2cRegister::ClearRestartDetected) => {
                let value = state.raw_intr & Self::IC_INTR_RESTART_DET;
                Self::clear_interrupt(&mut state, Self::IC_INTR_RESTART_DET);
                value
            }
            None => 0,
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
            return Err(DeviceError::new("RP2350 I²C requires aligned word access"));
        }
        let register = RpI2cRegister::from_offset(offset & 0x0fff);
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("I²C register fits u32");
        let mut state = self.state.borrow_mut();
        match register {
            Some(RpI2cRegister::Control) if !state.enable => state.con = value & 0x7f,
            Some(RpI2cRegister::TargetAddress) => state.tar = value & 0x03ff,
            Some(RpI2cRegister::SlaveAddress) => state.sar = value & 0x03ff,
            Some(RpI2cRegister::DataCommand) if state.enable => {
                self.complete_command(&mut state, value, at)?
            }
            Some(RpI2cRegister::InterruptMask) => state.intr_mask = value & IC_INTR_MASKED_BITS,
            Some(RpI2cRegister::ReceiveThreshold) => state.rx_tl = value & 0xff,
            Some(RpI2cRegister::TransmitThreshold) => state.tx_tl = value & 0xff,
            Some(RpI2cRegister::ClearInterrupt) => Self::clear_interrupt(&mut state, value),
            Some(RpI2cRegister::Enable) => {
                let enabled = value & 1 != 0;
                if !enabled {
                    state.active = false;
                    state.rx_fifo.clear();
                    state.raw_intr = 0;
                }
                state.enable = enabled;
            }
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}
