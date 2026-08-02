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
            RpPl011Register::Control => self.control = value & 0xff87,
            RpPl011Register::InterruptFifoLevel => self.interrupt_fifo_level = value & 0x3f,
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
pub struct WchUsart {
    name: String,
    baud_rate: u32,
    control: [u32; 3],
    guard_prescaler: u32,
    handle: UartHandle,
}

impl WchUsart {
    const TXE: u32 = 1 << 7;
    const TC: u32 = 1 << 6;
    const UE: u32 = 1 << 13;
    const TE: u32 = 1 << 3;

    /// Creates a reset USART and its host-facing terminal handle.
    pub fn new(name: impl Into<String>) -> (Self, UartHandle) {
        let handle = UartHandle::default();
        (
            Self {
                name: name.into(),
                baud_rate: 0,
                control: [0; 3],
                guard_prescaler: 0,
                handle: handle.clone(),
            },
            handle,
        )
    }

    fn reset_registers(&mut self) {
        self.baud_rate = 0;
        self.control = [0; 3];
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
        let value = match offset {
            0x00 => Self::TXE | Self::TC,
            0x04 => 0,
            0x08 => self.baud_rate,
            0x0c => self.control[0],
            0x10 => self.control[1],
            0x14 => self.control[2],
            0x18 => self.guard_prescaler,
            _ => {
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
        match offset {
            0x00 => {}
            0x04 => {
                if self.control[0] & (Self::UE | Self::TE) == (Self::UE | Self::TE) {
                    self.handle.transmit(&[value as u8]);
                }
            }
            0x08 => self.baud_rate = value & 0x0000_ffff,
            0x0c => self.control[0] = value,
            0x10 => self.control[1] = value,
            0x14 => self.control[2] = value,
            0x18 => self.guard_prescaler = value,
            _ => {
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
