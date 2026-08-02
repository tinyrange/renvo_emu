use super::*;

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
            intr_mask: 0,
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
        self.intr_mask = 0;
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
        self.raw_intr & self.intr_mask != 0
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
    const IC_CON: u64 = 0x00;
    const IC_TAR: u64 = 0x04;
    const IC_SAR: u64 = 0x08;
    const IC_DATA_CMD: u64 = 0x10;
    const IC_INTR_STAT: u64 = 0x2c;
    const IC_INTR_MASK: u64 = 0x30;
    const IC_RAW_INTR_STAT: u64 = 0x34;
    const IC_RX_TL: u64 = 0x38;
    const IC_TX_TL: u64 = 0x3c;
    const IC_CLR_INTR: u64 = 0x40;
    const IC_CLR_RX_UNDER: u64 = 0x44;
    const IC_CLR_RX_OVER: u64 = 0x48;
    const IC_CLR_TX_OVER: u64 = 0x4c;
    const IC_CLR_RD_REQ: u64 = 0x50;
    const IC_CLR_TX_ABRT: u64 = 0x54;
    const IC_CLR_RX_DONE: u64 = 0x58;
    const IC_CLR_ACTIVITY: u64 = 0x5c;
    const IC_CLR_STOP_DET: u64 = 0x60;
    const IC_CLR_START_DET: u64 = 0x64;
    const IC_CLR_GEN_CALL: u64 = 0x68;
    const IC_ENABLE: u64 = 0x6c;
    const IC_STATUS: u64 = 0x70;
    const IC_TXFLR: u64 = 0x74;
    const IC_RXFLR: u64 = 0x78;
    const IC_ENABLE_STATUS: u64 = 0x9c;
    const IC_CLR_RESTART_DET: u64 = 0xa8;
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
    const IC_INTR_MASKED_BITS: u32 = (1 << 15) - 1;

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
        let register = offset & 0x0fff;
        let mut state = self.state.borrow_mut();
        let value = match register {
            Self::IC_CON => state.con,
            Self::IC_TAR => state.tar,
            Self::IC_SAR => state.sar,
            Self::IC_DATA_CMD => {
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
            Self::IC_INTR_STAT => state.raw_intr & state.intr_mask,
            Self::IC_INTR_MASK => state.intr_mask,
            Self::IC_RAW_INTR_STAT => state.raw_intr,
            Self::IC_RX_TL => state.rx_tl,
            Self::IC_TX_TL => state.tx_tl,
            Self::IC_CLR_INTR => {
                let value = state.raw_intr;
                state.raw_intr = 0;
                value
            }
            Self::IC_CLR_RX_UNDER => {
                let value = state.raw_intr & Self::IC_INTR_RX_UNDER;
                Self::clear_interrupt(&mut state, Self::IC_INTR_RX_UNDER);
                value
            }
            Self::IC_CLR_RX_OVER => {
                let value = state.raw_intr & Self::IC_INTR_RX_OVER;
                Self::clear_interrupt(&mut state, Self::IC_INTR_RX_OVER);
                value
            }
            Self::IC_CLR_TX_OVER => {
                let value = state.raw_intr & Self::IC_INTR_TX_OVER;
                Self::clear_interrupt(&mut state, Self::IC_INTR_TX_OVER);
                value
            }
            Self::IC_CLR_RD_REQ => {
                let value = state.raw_intr & Self::IC_INTR_RD_REQ;
                Self::clear_interrupt(&mut state, Self::IC_INTR_RD_REQ);
                value
            }
            Self::IC_CLR_TX_ABRT => {
                let value = state.raw_intr & Self::IC_INTR_TX_ABRT;
                Self::clear_interrupt(&mut state, Self::IC_INTR_TX_ABRT);
                value
            }
            Self::IC_CLR_RX_DONE => {
                let value = state.raw_intr & Self::IC_INTR_RX_DONE;
                Self::clear_interrupt(&mut state, Self::IC_INTR_RX_DONE);
                value
            }
            Self::IC_CLR_ACTIVITY => {
                let value = state.raw_intr & Self::IC_INTR_ACTIVITY;
                Self::clear_interrupt(&mut state, Self::IC_INTR_ACTIVITY);
                value
            }
            Self::IC_CLR_STOP_DET => {
                let value = state.raw_intr & Self::IC_INTR_STOP_DET;
                Self::clear_interrupt(&mut state, Self::IC_INTR_STOP_DET);
                value
            }
            Self::IC_CLR_START_DET => {
                let value = state.raw_intr & Self::IC_INTR_START_DET;
                Self::clear_interrupt(&mut state, Self::IC_INTR_START_DET);
                value
            }
            Self::IC_CLR_GEN_CALL => {
                let value = state.raw_intr & Self::IC_INTR_GEN_CALL;
                Self::clear_interrupt(&mut state, Self::IC_INTR_GEN_CALL);
                value
            }
            Self::IC_ENABLE => u32::from(state.enable),
            Self::IC_STATUS => {
                let mut status = 0;
                if state.active {
                    status |= 1 << 0;
                }
                if state.enable {
                    status |= 1 << 1;
                }
                status |= 1 << 2;
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
            Self::IC_TXFLR => 0,
            Self::IC_RXFLR => u32::try_from(state.rx_fifo.len()).expect("I²C FIFO length fits"),
            Self::IC_ENABLE_STATUS => u32::from(state.enable),
            Self::IC_CLR_RESTART_DET => {
                let value = state.raw_intr & Self::IC_INTR_RESTART_DET;
                Self::clear_interrupt(&mut state, Self::IC_INTR_RESTART_DET);
                value
            }
            _ => 0,
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
        let register = offset & 0x0fff;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("I²C register fits u32");
        let mut state = self.state.borrow_mut();
        match register {
            Self::IC_CON if !state.enable => state.con = value & 0x7f,
            Self::IC_TAR => state.tar = value & 0x03ff,
            Self::IC_SAR => state.sar = value & 0x03ff,
            Self::IC_DATA_CMD if state.enable => self.complete_command(&mut state, value, at)?,
            Self::IC_INTR_MASK => state.intr_mask = value & Self::IC_INTR_MASKED_BITS,
            Self::IC_RX_TL => state.rx_tl = value & 0xff,
            Self::IC_TX_TL => state.tx_tl = value & 0xff,
            Self::IC_CLR_INTR => Self::clear_interrupt(&mut state, value),
            Self::IC_ENABLE => {
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
