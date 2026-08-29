//! Functional WCH `CH32V00x` I2C1 master peripheral.
//!
//! The CH32V003 and CH32V006 expose an STM32F1-shaped I2C controller.  This
//! model keeps the register protocol and transaction state deterministic while
//! leaving electrical bus arbitration and clock stretching to a future signal
//! backend.  A host can acknowledge addresses, queue bytes for reads, and
//! inspect bytes written by firmware.

use super::{
    AccessWidth, Arc, BTreeMap, BTreeSet, Device, DeviceError, Mutex, ResetKind, SimTime, VecDeque,
};

/// Register identifiers for the WCH `I2C1` block.
///
/// The hardware exposes these as 16-bit registers on four-byte boundaries.
/// Keeping the identifiers named prevents the register protocol from being
/// spread through the device implementation as unannotated offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum WchI2cRegister {
    /// Control register 1.
    Ctlr1 = 0x00,
    /// Control register 2.
    Ctlr2 = 0x04,
    /// Own address register 1.
    Oaddr1 = 0x08,
    /// Own address register 2.
    Oaddr2 = 0x0c,
    /// Data register.
    Datar = 0x10,
    /// Status register 1.
    Star1 = 0x14,
    /// Status register 2.
    Star2 = 0x18,
    /// Clock configuration register.
    Ckcfgr = 0x1c,
}

impl TryFrom<u64> for WchI2cRegister {
    type Error = DeviceError;

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x00 => Ok(Self::Ctlr1),
            0x04 => Ok(Self::Ctlr2),
            0x08 => Ok(Self::Oaddr1),
            0x0c => Ok(Self::Oaddr2),
            0x10 => Ok(Self::Datar),
            0x14 => Ok(Self::Star1),
            0x18 => Ok(Self::Star2),
            0x1c => Ok(Self::Ckcfgr),
            _ => Err(DeviceError::new(format!(
                "unmodeled WCH I2C register at offset {offset:#x}"
            ))),
        }
    }
}

const CTLR1_PE: u16 = 1 << 0;
const CTLR1_ENPEC: u16 = 1 << 5;
const CTLR1_ENGC: u16 = 1 << 6;
const CTLR1_NOSTRETCH: u16 = 1 << 7;
const CTLR1_START: u16 = 1 << 8;
const CTLR1_STOP: u16 = 1 << 9;
const CTLR1_ACK: u16 = 1 << 10;
const CTLR1_POS: u16 = 1 << 11;
const CTLR1_PEC: u16 = 1 << 12;
const CTLR1_SWRST: u16 = 1 << 15;
const CTLR1_SUPPORTED: u16 = CTLR1_PE
    | CTLR1_ENPEC
    | CTLR1_ENGC
    | CTLR1_NOSTRETCH
    | CTLR1_START
    | CTLR1_STOP
    | CTLR1_ACK
    | CTLR1_POS
    | CTLR1_PEC
    | CTLR1_SWRST;
const CTLR2_ITERREN: u16 = 1 << 8;
const CTLR2_ITEVTEN: u16 = 1 << 9;
const CTLR2_ITBUFEN: u16 = 1 << 10;
const CTLR2_DMAEN: u16 = 1 << 11;
const CTLR2_LAST: u16 = 1 << 12;
const CTLR2_SUPPORTED: u16 =
    CTLR2_ITERREN | CTLR2_ITEVTEN | CTLR2_ITBUFEN | CTLR2_DMAEN | CTLR2_LAST | 0x003f;
const OADDR1_SUPPORTED: u16 = 0x83ff;
const OADDR2_SUPPORTED: u16 = 0x00ff;
const CKCFGR_SUPPORTED: u16 = 0xcfff;

const STAR1_SB: u16 = 1 << 0;
const STAR1_ADDR: u16 = 1 << 1;
const STAR1_BTF: u16 = 1 << 2;
const STAR1_STOPF: u16 = 1 << 4;
const STAR1_RXNE: u16 = 1 << 6;
const STAR1_TXE: u16 = 1 << 7;
const STAR1_ERRORS: u16 = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 11) | (1 << 12);
const STAR2_MSL: u16 = 1 << 0;
const STAR2_BUSY: u16 = 1 << 1;
const STAR2_TRA: u16 = 1 << 2;

/// Bytes emitted by one functional I2C master transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WchI2cWrite {
    /// Seven-bit slave address selected by the address byte.
    pub address: u8,
    /// Data byte written after the address was acknowledged.
    pub value: u8,
}

/// Host-facing state for a WCH I2C controller.
#[derive(Clone)]
pub struct WchI2cHandle {
    state: Arc<Mutex<WchI2cState>>,
}

impl WchI2cHandle {
    /// Queues bytes returned when firmware addresses `address` for reading.
    ///
    /// Bytes are consumed in order.  Supplying another queue for the same
    /// address appends to the existing response, which makes repeated-start
    /// register reads straightforward to script.
    pub fn queue_read(&self, address: u8, bytes: &[u8]) {
        let mut state = self.state.lock().expect("WCH I2C lock poisoned");
        state
            .queued_reads
            .entry(address & 0x7f)
            .or_default()
            .extend(bytes.iter().copied());
    }

    /// Controls whether a seven-bit address receives an ACK.
    ///
    /// Addresses ACK by default.  Disabling an address lets tests exercise the
    /// controller's `AF` error path without modelling an analogue bus.
    pub fn set_address_ack(&self, address: u8, acknowledge: bool) {
        let mut state = self.state.lock().expect("WCH I2C lock poisoned");
        if acknowledge {
            state.nack_addresses.remove(&(address & 0x7f));
        } else {
            state.nack_addresses.insert(address & 0x7f);
        }
    }

    /// Returns and clears bytes written by firmware since the previous call.
    pub fn take_transmitted(&self) -> Vec<WchI2cWrite> {
        let mut state = self.state.lock().expect("WCH I2C lock poisoned");
        std::mem::take(&mut state.transmitted)
    }

    /// Returns `(event_pending, error_pending)` for PFIC routing.
    pub fn interrupt_pending(&self) -> (bool, bool) {
        let state = self.state.lock().expect("WCH I2C lock poisoned");
        state.interrupt_pending()
    }
}

struct WchI2cState {
    ctlr1: u16,
    ctlr2: u16,
    oaddr1: u16,
    oaddr2: u16,
    datar: u8,
    star1: u16,
    star2: u16,
    ckcfgr: u16,
    address: Option<u8>,
    receiving: bool,
    star1_read_for_addr: bool,
    queued_reads: BTreeMap<u8, VecDeque<u8>>,
    rx_pending: VecDeque<u8>,
    nack_addresses: BTreeSet<u8>,
    transmitted: Vec<WchI2cWrite>,
}

impl WchI2cState {
    fn reset() -> Self {
        Self {
            ctlr1: 0,
            ctlr2: 0,
            oaddr1: 0,
            oaddr2: 0,
            datar: 0,
            star1: 0,
            star2: 0,
            ckcfgr: 0,
            address: None,
            receiving: false,
            star1_read_for_addr: false,
            queued_reads: BTreeMap::new(),
            rx_pending: VecDeque::new(),
            nack_addresses: BTreeSet::new(),
            transmitted: Vec::new(),
        }
    }

    fn interrupt_pending(&self) -> (bool, bool) {
        // BTF is an event interrupt only when neither buffer event is active;
        // the WCH manual lists TXE/RXNE under ITBUFEN in that case.
        let event_flags = self.star1 & (STAR1_SB | STAR1_ADDR | STAR1_STOPF) != 0
            || self.star1 & STAR1_BTF != 0 && self.star1 & (STAR1_TXE | STAR1_RXNE) == 0;
        let buffer_flags = self.star1 & (STAR1_RXNE | STAR1_TXE) != 0;
        let event = self.ctlr2 & CTLR2_ITEVTEN != 0 && event_flags
            || self.ctlr2 & CTLR2_ITBUFEN != 0 && buffer_flags;
        let error = self.ctlr2 & CTLR2_ITERREN != 0 && self.star1 & STAR1_ERRORS != 0;
        (event, error)
    }

    fn clear_transaction(&mut self, preserve_errors: bool) {
        self.address = None;
        self.receiving = false;
        self.star1 = if preserve_errors {
            self.star1 & STAR1_ERRORS
        } else {
            0
        };
        self.star2 = 0;
        self.star1_read_for_addr = false;
        self.rx_pending.clear();
    }

    fn start(&mut self) {
        self.address = None;
        self.receiving = false;
        self.star1 &= STAR1_ERRORS;
        self.star1 |= STAR1_SB;
        self.star2 = STAR2_MSL | STAR2_BUSY;
        self.star1_read_for_addr = false;
        self.rx_pending.clear();
    }

    fn address_byte(&mut self, value: u8) {
        let address = (value >> 1) & 0x7f;
        let receiving = value & 1 != 0;
        self.star1 &= STAR1_ERRORS;
        self.star1 &= !STAR1_SB;
        self.star1_read_for_addr = false;
        self.address = Some(address);
        self.receiving = receiving;
        if self.nack_addresses.contains(&address) {
            self.star1 |= 1 << 10; // AF: acknowledge failure.
            self.star2 = STAR2_MSL | STAR2_BUSY;
            self.address = None;
            return;
        }
        self.star1 |= STAR1_ADDR;
        if receiving {
            self.star2 = STAR2_MSL | STAR2_BUSY;
            self.rx_pending = self.queued_reads.remove(&address).unwrap_or_default();
        } else {
            self.star2 = STAR2_MSL | STAR2_BUSY | STAR2_TRA;
            // TXE becomes actionable after the firmware clears ADDR by the
            // required STAR1-then-STAR2 read sequence.
        }
    }

    fn clear_address(&mut self) {
        if self.star1 & STAR1_ADDR == 0 {
            return;
        }
        self.star1 &= !STAR1_ADDR;
        self.star1_read_for_addr = false;
        if self.receiving {
            self.load_received_byte();
        } else {
            self.star1 |= STAR1_TXE;
        }
    }

    fn load_received_byte(&mut self) {
        self.star1 &= !STAR1_TXE;
        if let Some(value) = self.rx_pending.front().copied() {
            self.datar = value;
            self.star1 |= STAR1_RXNE;
            if self.rx_pending.len() > 1 {
                self.star1 |= STAR1_BTF;
            }
        } else {
            self.star1 &= !(STAR1_RXNE | STAR1_BTF);
        }
    }

    fn read_data(&mut self) -> u8 {
        let value = self.datar;
        if self.receiving && self.star1 & STAR1_RXNE != 0 {
            let _ = self.rx_pending.pop_front();
            self.star1 &= !(STAR1_RXNE | STAR1_BTF);
            self.load_received_byte();
        }
        value
    }

    fn write_data(&mut self, value: u8) {
        if self.star1 & STAR1_SB != 0 {
            self.address_byte(value);
            return;
        }
        self.datar = value;
        if let Some(address) = self.address.filter(|_| !self.receiving) {
            // Writing DATAR clears TXE/BTF.  The functional backend completes
            // the byte immediately, so TXE is raised again before the next
            // guest instruction while BTF remains clear until a later byte
            // boundary is observable.
            self.star1 &= !(STAR1_TXE | STAR1_BTF);
            self.transmitted.push(WchI2cWrite { address, value });
            self.star1 |= STAR1_TXE;
        }
    }

    fn write_control(&mut self, value: u16) {
        let was_enabled = self.ctlr1 & CTLR1_PE != 0;
        let start = value & CTLR1_START != 0;
        let stop = value & CTLR1_STOP != 0;
        self.ctlr1 = value & CTLR1_SUPPORTED & !CTLR1_START & !CTLR1_STOP;
        if value & CTLR1_SWRST != 0 {
            let queued_reads = std::mem::take(&mut self.queued_reads);
            let nack_addresses = std::mem::take(&mut self.nack_addresses);
            *self = Self::reset();
            self.queued_reads = queued_reads;
            self.nack_addresses = nack_addresses;
            return;
        }
        if was_enabled != (self.ctlr1 & CTLR1_PE != 0) {
            self.clear_transaction(false);
        }
        if stop && self.ctlr1 & CTLR1_PE != 0 {
            self.clear_transaction(true);
        }
        if start && self.ctlr1 & CTLR1_PE != 0 {
            self.start();
        }
    }
}

/// Functional WCH `I2C_TypeDef` register block.
pub struct WchI2c {
    name: String,
    state: Arc<Mutex<WchI2cState>>,
}

impl WchI2c {
    /// Creates a reset I2C1 controller and its host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, WchI2cHandle) {
        let state = Arc::new(Mutex::new(WchI2cState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            WchI2cHandle { state },
        )
    }

    fn require_register_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if !matches!(width, AccessWidth::HalfWord | AccessWidth::Word) || offset & 3 != 0 {
            return Err(DeviceError::new(
                "WCH I2C requires halfword or word access at a register boundary",
            ));
        }
        Ok(())
    }
}

impl Device for WchI2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_register_access(offset, width)?;
        let register = WchI2cRegister::try_from(offset)?;
        let mut state = self.state.lock().expect("WCH I2C lock poisoned");
        let value = match register {
            WchI2cRegister::Ctlr1 => state.ctlr1,
            WchI2cRegister::Ctlr2 => state.ctlr2,
            WchI2cRegister::Oaddr1 => state.oaddr1,
            WchI2cRegister::Oaddr2 => state.oaddr2,
            WchI2cRegister::Datar => u16::from(state.read_data()),
            WchI2cRegister::Star1 => {
                state.star1_read_for_addr = state.star1 & STAR1_ADDR != 0;
                state.star1
            }
            WchI2cRegister::Star2 => {
                if state.star1_read_for_addr {
                    state.clear_address();
                }
                state.star2
            }
            WchI2cRegister::Ckcfgr => state.ckcfgr,
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        Self::require_register_access(offset, width)?;
        let register = WchI2cRegister::try_from(offset)?;
        let value = u16::try_from(value & u64::from(u16::MAX))
            .expect("masked WCH I2C register value fits u16");
        let mut state = self.state.lock().expect("WCH I2C lock poisoned");
        match register {
            WchI2cRegister::Ctlr1 => state.write_control(value),
            WchI2cRegister::Ctlr2 => state.ctlr2 = value & CTLR2_SUPPORTED,
            WchI2cRegister::Oaddr1 => state.oaddr1 = value & OADDR1_SUPPORTED,
            WchI2cRegister::Oaddr2 => state.oaddr2 = value & OADDR2_SUPPORTED,
            WchI2cRegister::Datar => state.write_data(
                u8::try_from(value & u16::from(u8::MAX)).expect("I2C DATAR is eight bits"),
            ),
            // STAR1 error flags are RW0: writing zero clears a selected error,
            // while read-only event/status flags are unaffected.  This also
            // matches the vendor SDK's write-the-complement clear helpers.
            WchI2cRegister::Star1 => state.star1 &= value | !STAR1_ERRORS,
            WchI2cRegister::Star2 => {}
            WchI2cRegister::Ckcfgr => state.ckcfgr = value & CKCFGR_SUPPORTED,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("WCH I2C lock poisoned") = WchI2cState::reset();
    }
}
