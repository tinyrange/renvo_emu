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

/// Register identifiers for the PL011-compatible UARTs used by RP2040 and
/// RP2350. Keeping the offsets named makes the device contract readable at
/// call sites and avoids scattering magic register numbers through the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum RpPl011Register {
    /// Data register (DR).
    Data = 0x000,
    /// Receive status / error clear register (RSR/ECR).
    ReceiveStatus = 0x004,
    /// Flag register (FR).
    Flags = 0x018,
    /// IrDA low-power counter register (ILPR).
    IrdaLowPower = 0x020,
    /// Integer baud-rate divisor (IBRD).
    IntegerBaud = 0x024,
    /// Fractional baud-rate divisor (FBRD).
    FractionalBaud = 0x028,
    /// Line-control register (LCR_H).
    LineControl = 0x02c,
    /// Control register (CR).
    Control = 0x030,
    /// Interrupt FIFO-level select register (IFLS).
    InterruptFifoLevel = 0x034,
    /// Interrupt mask set/clear register (IMSC).
    InterruptMask = 0x038,
    /// Raw interrupt status register (RIS).
    RawInterruptStatus = 0x03c,
    /// Masked interrupt status register (MIS).
    MaskedInterruptStatus = 0x040,
    /// Interrupt clear register (ICR).
    InterruptClear = 0x044,
    /// DMA control register (DMACR).
    DmaControl = 0x048,
    /// Peripheral identification byte 0.
    PeripheralId0 = 0xfe0,
    /// Peripheral identification byte 1.
    PeripheralId1 = 0xfe4,
    /// Peripheral identification byte 2.
    PeripheralId2 = 0xfe8,
    /// Peripheral identification byte 3.
    PeripheralId3 = 0xfec,
    /// PrimeCell identification byte 0.
    CellId0 = 0xff0,
    /// PrimeCell identification byte 1.
    CellId1 = 0xff4,
    /// PrimeCell identification byte 2.
    CellId2 = 0xff8,
    /// PrimeCell identification byte 3.
    CellId3 = 0xffc,
}

impl RpPl011Register {
    /// Returns the byte offset of this register.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    /// Converts a documented register offset into its named identifier.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x000 => Self::Data,
            0x004 => Self::ReceiveStatus,
            0x018 => Self::Flags,
            0x020 => Self::IrdaLowPower,
            0x024 => Self::IntegerBaud,
            0x028 => Self::FractionalBaud,
            0x02c => Self::LineControl,
            0x030 => Self::Control,
            0x034 => Self::InterruptFifoLevel,
            0x038 => Self::InterruptMask,
            0x03c => Self::RawInterruptStatus,
            0x040 => Self::MaskedInterruptStatus,
            0x044 => Self::InterruptClear,
            0x048 => Self::DmaControl,
            0xfe0 => Self::PeripheralId0,
            0xfe4 => Self::PeripheralId1,
            0xfe8 => Self::PeripheralId2,
            0xfec => Self::PeripheralId3,
            0xff0 => Self::CellId0,
            0xff4 => Self::CellId1,
            0xff8 => Self::CellId2,
            0xffc => Self::CellId3,
            _ => return None,
        })
    }
}

/// Functional PL011 register slice used by the RP2040 and RP2350 UART1
/// instances.
///
/// The model deliberately covers the register programming contract used by
/// SDK and compiler smoke tests rather than trying to simulate UART bit
/// timing. Transmit is immediate once `UARTEN` and `TXE` are enabled. There
/// is no receive queue, baud-rate timing, FIFO occupancy, modem signalling,
/// DMA engine, or generated interrupt source yet; those omissions are
/// explicit so a firmware test cannot mistake this for a full UART model.
pub struct RpPl011Uart {
    name: String,
    receive_status: u32,
    irda_low_power: u32,
    integer_baud: u32,
    fractional_baud: u32,
    line_control: u32,
    control: u32,
    interrupt_fifo_level: u32,
    interrupt_mask: u32,
    raw_interrupt_status: u32,
    dma_control: u32,
    handle: UartHandle,
}

impl RpPl011Uart {
    const UARTEN: u32 = 1 << 0;
    const TXE: u32 = 1 << 8;
    const RXE: u32 = 1 << 9;
    const FLAG_TXFE: u32 = 1 << 7;
    const FLAG_RXFE: u32 = 1 << 4;
    const REGISTER_MASK: u32 = 0x07ff;
    const CONTROL_MASK: u32 = 0xff87;
    const FIFO_LEVEL_MASK: u32 = 0x3f;

    /// Creates a reset PL011 slice and its host-facing terminal handle.
    pub fn new(name: impl Into<String>) -> (Self, UartHandle) {
        let handle = UartHandle::default();
        (
            Self {
                name: name.into(),
                receive_status: 0,
                irda_low_power: 0,
                integer_baud: 0,
                fractional_baud: 0,
                line_control: 0,
                // PL011 reset leaves TXE and RXE asserted, but UARTEN clear.
                control: Self::TXE | Self::RXE,
                interrupt_fifo_level: 0x12,
                interrupt_mask: 0,
                raw_interrupt_status: 0,
                dma_control: 0,
                handle: handle.clone(),
            },
            handle,
        )
    }

    fn require_word(width: AccessWidth) -> Result<(), DeviceError> {
        if width == AccessWidth::Word {
            Ok(())
        } else {
            Err(DeviceError::new("RP PL011 UART requires word access"))
        }
    }

    fn flags(&self) -> u32 {
        // With no receive queue and immediate transmit, both FIFO-empty bits
        // stay asserted. BUSY is not observable between functional writes.
        Self::FLAG_TXFE | Self::FLAG_RXFE
    }

    fn fifo_level(value: u32) -> Result<u32, DeviceError> {
        let rx_level = (value >> 3) & 0x07;
        let tx_level = value & 0x07;
        if rx_level > 4 || tx_level > 4 {
            return Err(DeviceError::new(
                "RP PL011 UART FIFO level must use a documented 1/8..7/8 encoding",
            ));
        }
        Ok(value & Self::FIFO_LEVEL_MASK)
    }
}

impl Device for RpPl011Uart {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_word(width)?;
        let register = RpPl011Register::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP PL011 UART read at offset {offset:#x}"
            ))
        })?;
        let value = match register {
            RpPl011Register::Data => 0,
            RpPl011Register::ReceiveStatus => self.receive_status & 0x0f,
            RpPl011Register::Flags => self.flags(),
            RpPl011Register::IrdaLowPower => self.irda_low_power,
            RpPl011Register::IntegerBaud => self.integer_baud,
            RpPl011Register::FractionalBaud => self.fractional_baud,
            RpPl011Register::LineControl => self.line_control,
            RpPl011Register::Control => self.control,
            RpPl011Register::InterruptFifoLevel => self.interrupt_fifo_level,
            RpPl011Register::InterruptMask => self.interrupt_mask,
            RpPl011Register::RawInterruptStatus => self.raw_interrupt_status,
            RpPl011Register::MaskedInterruptStatus => {
                self.raw_interrupt_status & self.interrupt_mask
            }
            RpPl011Register::InterruptClear => 0,
            RpPl011Register::DmaControl => self.dma_control,
            RpPl011Register::PeripheralId0 => 0x11,
            RpPl011Register::PeripheralId1 => 0x10,
            RpPl011Register::PeripheralId2 => 0x34,
            RpPl011Register::PeripheralId3 => 0x00,
            RpPl011Register::CellId0 => 0x0d,
            RpPl011Register::CellId1 => 0xf0,
            RpPl011Register::CellId2 => 0x05,
            RpPl011Register::CellId3 => 0xb1,
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
        Self::require_word(width)?;
        let register = RpPl011Register::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP PL011 UART write at offset {offset:#x}"
            ))
        })?;
        let value = u32::try_from(value & u64::from(u32::MAX))
            .expect("masked PL011 register value fits u32");
        match register {
            RpPl011Register::Data => {
                if self.control & (Self::UARTEN | Self::TXE) == (Self::UARTEN | Self::TXE) {
                    self.handle.transmit(&[value as u8]);
                }
            }
            RpPl011Register::ReceiveStatus => {
                // UARTECR is write-to-clear: any write clears all four
                // receive error latches (the written value is ignored).
                self.receive_status = 0;
            }
            RpPl011Register::Flags => {}
            RpPl011Register::IrdaLowPower => self.irda_low_power = value & 0xff,
            RpPl011Register::IntegerBaud => self.integer_baud = value & 0xffff,
            RpPl011Register::FractionalBaud => self.fractional_baud = value & 0x3f,
            RpPl011Register::LineControl => self.line_control = value & 0xff,
            RpPl011Register::Control => self.control = value & Self::CONTROL_MASK,
            RpPl011Register::InterruptFifoLevel => {
                self.interrupt_fifo_level = Self::fifo_level(value)?;
            }
            RpPl011Register::InterruptMask => self.interrupt_mask = value & Self::REGISTER_MASK,
            RpPl011Register::RawInterruptStatus => {}
            RpPl011Register::MaskedInterruptStatus => {}
            RpPl011Register::InterruptClear => {
                self.raw_interrupt_status &= !(value & Self::REGISTER_MASK);
            }
            RpPl011Register::DmaControl => self.dma_control = value & 0x07,
            RpPl011Register::PeripheralId0
            | RpPl011Register::PeripheralId1
            | RpPl011Register::PeripheralId2
            | RpPl011Register::PeripheralId3
            | RpPl011Register::CellId0
            | RpPl011Register::CellId1
            | RpPl011Register::CellId2
            | RpPl011Register::CellId3 => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.receive_status = 0;
        self.irda_low_power = 0;
        self.integer_baud = 0;
        self.fractional_baud = 0;
        self.line_control = 0;
        self.control = Self::TXE | Self::RXE;
        self.interrupt_fifo_level = 0x12;
        self.interrupt_mask = 0;
        self.raw_interrupt_status = 0;
        self.dma_control = 0;
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
