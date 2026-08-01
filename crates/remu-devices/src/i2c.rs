use super::*;

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

/// Host-facing state for a functional I²C controller.
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
            .entry(address)
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

/// Deterministic byte-oriented DesignWare-style I²C controller.
///
/// The register view matches the controller integrated by the RP2040 and RP2350. Writes to
/// `DATA_CMD` execute immediately, recording write bytes and consuming host-queued response bytes
/// for read commands. FIFO levels, status, interrupt latches, and the common timing/configuration
/// registers are modeled sufficiently for bounded SDK and board tests; electrical line timing,
/// arbitration, and DMA are deliberately outside this functional slice.
pub struct FunctionalI2c {
    name: String,
    con: u32,
    tar: u32,
    intr_mask: u32,
    raw_intr: u32,
    rx_tl: u32,
    tx_tl: u32,
    enable: u32,
    registers: BTreeMap<u64, u32>,
    rx_fifo: VecDeque<u8>,
    handle: I2cHandle,
}

impl FunctionalI2c {
    const DATA_CMD: u64 = 0x10;
    const INTR_STAT: u64 = 0x2c;
    const INTR_MASK: u64 = 0x30;
    const RAW_INTR: u64 = 0x34;
    const RX_TL: u64 = 0x38;
    const TX_TL: u64 = 0x3c;
    const CLR_INTR: u64 = 0x40;
    const ENABLE: u64 = 0x6c;
    const STATUS: u64 = 0x70;
    const TXFLR: u64 = 0x74;
    const RXFLR: u64 = 0x78;
    const TX_ABRT_SOURCE: u64 = 0x80;
    const COMP_PARAM: u64 = 0xf4;
    const COMP_VERSION: u64 = 0xf8;
    const COMP_TYPE: u64 = 0xfc;
    const READ_CMD: u64 = 1 << 8;
    const STOP: u64 = 1 << 9;
    const TX_EMPTY: u32 = 1 << 4;
    const RX_FULL: u32 = 1 << 2;
    const STOP_DET: u32 = 1 << 9;

    /// Creates a reset I²C controller and host handle.
    pub fn new(name: impl Into<String>) -> (Self, I2cHandle) {
        let handle = I2cHandle::default();
        (
            Self {
                name: name.into(),
                con: 0,
                tar: 0,
                intr_mask: 0,
                raw_intr: Self::TX_EMPTY,
                rx_tl: 0,
                tx_tl: 0,
                enable: 0,
                registers: BTreeMap::new(),
                rx_fifo: VecDeque::new(),
                handle: handle.clone(),
            },
            handle,
        )
    }

    fn check_access(offset: u64, width: AccessWidth) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("I2C requires aligned word access"));
        }
        Ok(offset & 0x0fff)
    }

    fn status(&self) -> u32 {
        // TFNF and TFE are always asserted because writes complete immediately.
        0x06 | u32::from(!self.rx_fifo.is_empty()) << 3
    }

    fn read_register(&mut self, offset: u64) -> Result<u32, DeviceError> {
        match offset {
            0x00 => Ok(self.con),
            0x04 => Ok(self.tar),
            Self::DATA_CMD => Ok(u32::from(self.rx_fifo.pop_front().unwrap_or(0))),
            Self::INTR_STAT => Ok(self.raw_intr & self.intr_mask),
            Self::INTR_MASK => Ok(self.intr_mask),
            Self::RAW_INTR => Ok(self.raw_intr),
            Self::RX_TL => Ok(self.rx_tl),
            Self::TX_TL => Ok(self.tx_tl),
            Self::CLR_INTR => {
                let value = self.raw_intr;
                self.raw_intr = 0;
                Ok(value)
            }
            Self::ENABLE => Ok(self.enable),
            0x68 => Ok(0),
            Self::STATUS => Ok(self.status()),
            Self::TXFLR => Ok(0),
            Self::RXFLR => Ok(self.rx_fifo.len() as u32),
            Self::TX_ABRT_SOURCE => Ok(0),
            Self::COMP_PARAM => Ok(0x00ff_3fff),
            Self::COMP_VERSION => Ok(0x3131_312a),
            Self::COMP_TYPE => Ok(0x4457_0110),
            _ => Ok(*self.registers.get(&offset).unwrap_or(&0)),
        }
    }
}

impl Device for FunctionalI2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Ok(u64::from(
            self.read_register(Self::check_access(offset, width)?)?,
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let offset = Self::check_access(offset, width)?;
        match offset {
            0x00 => self.con = value as u32,
            0x04 => self.tar = value as u32 & 0x3ff,
            Self::DATA_CMD => {
                if value & Self::READ_CMD != 0 {
                    self.rx_fifo
                        .push_back(self.handle.response(self.tar as u16));
                    if self.rx_fifo.len() > self.rx_tl as usize {
                        self.raw_intr |= Self::RX_FULL;
                    }
                } else {
                    self.handle.record(I2cEvent::Write {
                        address: self.tar as u16,
                        value: value as u8,
                    });
                }
                self.raw_intr |= Self::TX_EMPTY;
                if value & Self::STOP != 0 {
                    self.raw_intr |= Self::STOP_DET;
                }
            }
            Self::INTR_MASK => self.intr_mask = value as u32,
            Self::RX_TL => self.rx_tl = value as u32,
            Self::TX_TL => self.tx_tl = value as u32,
            Self::CLR_INTR => self.raw_intr = 0,
            Self::ENABLE => self.enable = value as u32 & 1,
            Self::TX_ABRT_SOURCE => {}
            _ => {
                self.registers.insert(offset, value as u32);
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.con = 0;
        self.tar = 0;
        self.intr_mask = 0;
        self.raw_intr = Self::TX_EMPTY;
        self.rx_tl = 0;
        self.tx_tl = 0;
        self.enable = 0;
        self.registers.clear();
        self.rx_fifo.clear();
        self.handle.clear();
    }
}
