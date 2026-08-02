use super::*;

/// Shared terminal UART output.
#[derive(Clone, Default)]
pub struct UartHandle {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl UartHandle {
    /// Returns all transmitted bytes.
    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.lock().expect("UART lock poisoned").clone()
    }

    /// Returns lossy UTF-8 terminal output.
    pub fn text_lossy(&self) -> String {
        String::from_utf8_lossy(&self.bytes()).into_owned()
    }

    /// Clears captured output.
    pub fn clear(&self) {
        self.bytes.lock().expect("UART lock poisoned").clear();
    }

    /// Appends bytes transmitted by a functional ROM or peripheral service.
    pub fn transmit(&self, bytes: &[u8]) {
        self.bytes
            .lock()
            .expect("UART lock poisoned")
            .extend_from_slice(bytes);
    }
}

/// Configurable byte-oriented UART facade.
pub struct FunctionalUart {
    name: String,
    data_offset: u64,
    status_offset: u64,
    tx_ready_mask: u32,
    lenient_registers: bool,
    handle: UartHandle,
}

impl FunctionalUart {
    /// Creates a UART and a host handle.
    pub fn new(
        name: impl Into<String>,
        data_offset: u64,
        status_offset: u64,
        tx_ready_mask: u32,
    ) -> (Self, UartHandle) {
        let handle = UartHandle::default();
        (
            Self {
                name: name.into(),
                data_offset,
                status_offset,
                tx_ready_mask,
                lenient_registers: false,
                handle: handle.clone(),
            },
            handle,
        )
    }

    /// Creates a UART that stores bytes at `data_offset` and tolerates other
    /// control-register accesses. This is useful for bounded vendor facades.
    pub fn new_lenient(
        name: impl Into<String>,
        data_offset: u64,
        status_offset: u64,
        tx_ready_mask: u32,
    ) -> (Self, UartHandle) {
        let (mut device, handle) = Self::new(name, data_offset, status_offset, tx_ready_mask);
        device.lenient_registers = true;
        (device, handle)
    }
}

impl Device for FunctionalUart {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, _width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if offset == self.status_offset {
            Ok(u64::from(self.tx_ready_mask))
        } else if offset == self.data_offset || self.lenient_registers {
            Ok(0)
        } else {
            Err(DeviceError::new(format!(
                "unmodeled UART read at offset {offset:#x}"
            )))
        }
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if offset != self.data_offset && !self.lenient_registers {
            return Err(DeviceError::new(format!(
                "unmodeled UART write at offset {offset:#x}"
            )));
        }
        if offset == self.data_offset {
            self.handle
                .bytes
                .lock()
                .expect("UART lock poisoned")
                .push(value.to_le_bytes()[0]);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.handle.clear();
    }
}

/// WCH CH32V00x USART register slice.
///
/// Transmission is functional and immediate: once USART and transmitter
/// enable are set in `CTLR1`, writing `DATAR` appends one byte to the host
/// terminal while `TXE` and `TC` remain asserted. Receive and line timing are
/// deliberately outside the six-chip baseline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WchUsartRegister {
    /// Status register (`STATR`).
    Status = 0x00,
    /// Data register (`DATAR`).
    Data = 0x04,
    /// Baud-rate register (`BRR`).
    BaudRate = 0x08,
    /// Control register 1 (`CTLR1`).
    Control1 = 0x0c,
    /// Control register 2 (`CTLR2`).
    Control2 = 0x10,
    /// Control register 3 (`CTLR3`).
    Control3 = 0x14,
    /// Guard-time and prescaler register (`GPR`).
    GuardPrescaler = 0x18,
}

impl WchUsartRegister {
    /// Converts a native byte offset to a named USART register.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::Status,
            0x04 => Self::Data,
            0x08 => Self::BaudRate,
            0x0c => Self::Control1,
            0x10 => Self::Control2,
            0x14 => Self::Control3,
            0x18 => Self::GuardPrescaler,
            _ => return None,
        })
    }

    /// Returns the native byte offset of a named USART register.
    pub const fn offset(self) -> u64 {
        self as u64
    }
}

/// Functional WCH USART register bank shared by USART1 and USART2.
pub struct WchUsart {
    name: String,
    status: u32,
    baud_rate: u32,
    control1: u32,
    control2: u32,
    control3: u32,
    guard_prescaler: u32,
    handle: UartHandle,
}

impl WchUsart {
    const TXE: u32 = 1 << 7;
    const TC: u32 = 1 << 6;
    const UE: u32 = 1 << 13;
    const TE: u32 = 1 << 3;
    const RXNE: u32 = 1 << 5;
    const STATUS_RW0_MASK: u32 = (1 << 9) | (1 << 8) | Self::TC | Self::RXNE;
    const CONTROL1_MASK: u32 = 0x0000_3fff;
    const CONTROL2_MASK: u32 = 0x0000_706f;
    const CONTROL3_MASK: u32 = 0x0000_07cf;
    const BAUD_RATE_MASK: u32 = 0x0000_ffff;
    const GUARD_PRESCALER_MASK: u32 = 0x0000_00ff;

    /// Creates a reset USART and its host-facing terminal handle.
    pub fn new(name: impl Into<String>) -> (Self, UartHandle) {
        let handle = UartHandle::default();
        (
            Self {
                name: name.into(),
                status: Self::TXE | Self::TC,
                baud_rate: 0,
                control1: 0,
                control2: 0,
                control3: 0,
                guard_prescaler: 0,
                handle: handle.clone(),
            },
            handle,
        )
    }

    fn reset_registers(&mut self) {
        self.status = Self::TXE | Self::TC;
        self.baud_rate = 0;
        self.control1 = 0;
        self.control2 = 0;
        self.control3 = 0;
        self.guard_prescaler = 0;
    }
}

impl Device for WchUsart {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("WCH USART requires word access"));
        }
        let value = match WchUsartRegister::from_offset(offset) {
            Some(WchUsartRegister::Status) => self.status,
            Some(WchUsartRegister::Data) => {
                self.status &= !Self::RXNE;
                0
            }
            Some(WchUsartRegister::BaudRate) => self.baud_rate,
            Some(WchUsartRegister::Control1) => self.control1,
            Some(WchUsartRegister::Control2) => self.control2,
            Some(WchUsartRegister::Control3) => self.control3,
            Some(WchUsartRegister::GuardPrescaler) => self.guard_prescaler,
            None => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH USART read at offset {offset:#x}"
                )));
            }
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("WCH USART requires word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX))
            .expect("masked USART register value fits u32");
        match WchUsartRegister::from_offset(offset) {
            Some(WchUsartRegister::Status) => {
                // CTS, LBD, TC, and RXNE are documented RW0 flags: writing
                // zero clears them while writing one leaves them unchanged.
                self.status &= (value & Self::STATUS_RW0_MASK) | !Self::STATUS_RW0_MASK;
                self.status |= Self::TXE;
            }
            Some(WchUsartRegister::Data) => {
                if self.control1 & (Self::UE | Self::TE) == (Self::UE | Self::TE) {
                    // The native DR is nine bits wide. UartHandle is
                    // intentionally byte-oriented, so the low byte is the
                    // observable baseline and the ninth bit remains a
                    // documented fidelity gap.
                    self.status &= !Self::TXE;
                    self.handle.transmit(&[(value & 0xff) as u8]);
                    // Functional transmission completes in the same
                    // abstract step, restoring the ready/completed flags.
                    self.status |= Self::TXE | Self::TC;
                }
            }
            Some(WchUsartRegister::BaudRate) => self.baud_rate = value & Self::BAUD_RATE_MASK,
            Some(WchUsartRegister::Control1) => self.control1 = value & Self::CONTROL1_MASK,
            Some(WchUsartRegister::Control2) => self.control2 = value & Self::CONTROL2_MASK,
            Some(WchUsartRegister::Control3) => self.control3 = value & Self::CONTROL3_MASK,
            Some(WchUsartRegister::GuardPrescaler) => {
                self.guard_prescaler = value & Self::GUARD_PRESCALER_MASK
            }
            None => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH USART write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset_registers();
        self.handle.clear();
    }
}
