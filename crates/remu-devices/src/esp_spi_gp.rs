use super::*;

/// Named ESP32-S3 general-purpose SPI register offsets covered by this model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum Esp32s3SpiRegister {
    /// User transaction command register.
    Cmd = 0x00,
    /// User transaction feature-enable register.
    User = 0x10,
    /// Master data bit length register.
    MsDlen = 0x1c,
    /// DMA interrupt enable register.
    DmaIntEna = 0x34,
    /// DMA interrupt clear register.
    DmaIntClr = 0x38,
    /// DMA raw interrupt register.
    DmaIntRaw = 0x3c,
    /// DMA masked interrupt status register.
    DmaIntSt = 0x40,
    /// DMA interrupt set register.
    DmaIntSet = 0x44,
    /// First CPU data buffer register.
    W0 = 0x98,
    /// CPU data buffer register 1.
    W1 = 0x9c,
    /// CPU data buffer register 2.
    W2 = 0xa0,
    /// CPU data buffer register 3.
    W3 = 0xa4,
    /// CPU data buffer register 4.
    W4 = 0xa8,
    /// CPU data buffer register 5.
    W5 = 0xac,
    /// CPU data buffer register 6.
    W6 = 0xb0,
    /// CPU data buffer register 7.
    W7 = 0xb4,
    /// CPU data buffer register 8.
    W8 = 0xb8,
    /// CPU data buffer register 9.
    W9 = 0xbc,
    /// CPU data buffer register 10.
    W10 = 0xc0,
    /// CPU data buffer register 11.
    W11 = 0xc4,
    /// CPU data buffer register 12.
    W12 = 0xc8,
    /// CPU data buffer register 13.
    W13 = 0xcc,
    /// CPU data buffer register 14.
    W14 = 0xd0,
    /// Last CPU data buffer register.
    W15 = 0xd4,
    /// SPI version register.
    Date = 0xf0,
}

impl Esp32s3SpiRegister {
    /// Returns the register's native byte offset.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    /// Converts a native byte offset into a named register when modeled.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::Cmd,
            0x10 => Self::User,
            0x1c => Self::MsDlen,
            0x34 => Self::DmaIntEna,
            0x38 => Self::DmaIntClr,
            0x3c => Self::DmaIntRaw,
            0x40 => Self::DmaIntSt,
            0x44 => Self::DmaIntSet,
            0x98 => Self::W0,
            0x9c => Self::W1,
            0xa0 => Self::W2,
            0xa4 => Self::W3,
            0xa8 => Self::W4,
            0xac => Self::W5,
            0xb0 => Self::W6,
            0xb4 => Self::W7,
            0xb8 => Self::W8,
            0xbc => Self::W9,
            0xc0 => Self::W10,
            0xc4 => Self::W11,
            0xc8 => Self::W12,
            0xcc => Self::W13,
            0xd0 => Self::W14,
            0xd4 => Self::W15,
            0xf0 => Self::Date,
            _ => return None,
        })
    }
}

/// Host-observable state for one ESP32-S3 general-purpose SPI controller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Esp32s3SpiTransfer {
    /// Number of functional transfers completed by the controller.
    pub count: u64,
    /// Most recent MOSI byte stream, packed in the same MSB-first order used
    /// by the ESP32-S3 user transaction FIFO.
    pub bytes: Vec<u8>,
}

/// Functional ESP32-S3 SPI2/SPI3 controller model.
///
/// The model deliberately implements the host-visible user transaction path:
/// firmware fills the W0-W15 FIFO, configures the user bit length, and writes
/// `CMD.USR`. The transfer completes synchronously, loops the transmitted
/// stream back into the receive buffer, and exposes deterministic digital
/// MOSI/MISO/SCLK/CS0 activity. DMA, clock dividers, additional chip selects,
/// and electrical contention are left to later device slices.
pub struct Esp32s3Spi {
    name: String,
    registers: [u32; Self::REGISTER_WORDS],
    buffers: [u32; 16],
    dma_raw: u32,
    dma_enable: u32,
    transfer: Esp32s3SpiTransfer,
    hub: SignalHub,
    mosi: SignalId,
    miso: SignalId,
    sclk: SignalId,
    cs0: SignalId,
    transfer_done: SignalId,
}

impl Esp32s3Spi {
    const REGISTER_BYTES: u64 = 0x100;
    const REGISTER_WORDS: usize = (Self::REGISTER_BYTES / 4) as usize;

    /// `SPI_CMD_REG`: user transaction command register.
    pub const CMD: u64 = Esp32s3SpiRegister::Cmd.offset();
    /// `SPI_USER_REG`: user transaction feature enables.
    pub const USER: u64 = Esp32s3SpiRegister::User.offset();
    /// `SPI_MS_DLEN_REG`: master data length, encoded as bit count minus one.
    pub const MS_DLEN: u64 = Esp32s3SpiRegister::MsDlen.offset();
    /// `SPI_DMA_INT_ENA_REG`: DMA interrupt enable bits.
    pub const DMA_INT_ENA: u64 = Esp32s3SpiRegister::DmaIntEna.offset();
    /// `SPI_DMA_INT_CLR_REG`: DMA interrupt clear bits.
    pub const DMA_INT_CLR: u64 = Esp32s3SpiRegister::DmaIntClr.offset();
    /// `SPI_DMA_INT_RAW_REG`: raw DMA interrupt bits.
    pub const DMA_INT_RAW: u64 = Esp32s3SpiRegister::DmaIntRaw.offset();
    /// `SPI_DMA_INT_ST_REG`: enabled DMA interrupt status bits.
    pub const DMA_INT_ST: u64 = Esp32s3SpiRegister::DmaIntSt.offset();
    /// `SPI_DMA_INT_SET_REG`: software-set DMA interrupt bits.
    pub const DMA_INT_SET: u64 = Esp32s3SpiRegister::DmaIntSet.offset();
    /// First data buffer register (`SPI_W0_REG`).
    pub const W0: u64 = Esp32s3SpiRegister::W0.offset();
    /// Last data buffer register (`SPI_W15_REG`).
    pub const W15: u64 = Esp32s3SpiRegister::W15.offset();
    /// `SPI_DATE_REG`.
    pub const DATE: u64 = Esp32s3SpiRegister::Date.offset();

    const USER_TRANSACTION: u32 = 1 << 24;
    const USER_COMMAND: u32 = 1 << 31;
    const TRANS_DONE_INT: u32 = 1 << 12;
    const DATA_BIT_LENGTH_MASK: u32 = 0x3ffff;
    const USER_MISO: u32 = 1 << 28;
    const USER_MOSI: u32 = 1 << 27;
    const USER_DOUTDIN: u32 = 1 << 0;
    const USER_MASK: u32 = Self::USER_COMMAND
        | (1 << 30)
        | (1 << 29)
        | Self::USER_MISO
        | Self::USER_MOSI
        | (1 << 26)
        | (1 << 25)
        | (1 << 24)
        | (1 << 17)
        | (1 << 15)
        | (1 << 14)
        | (1 << 13)
        | (1 << 12)
        | (1 << 9)
        | (1 << 8)
        | (1 << 7)
        | (1 << 6)
        | (1 << 5)
        | (1 << 4)
        | (1 << 3)
        | Self::USER_DOUTDIN;
    const DMA_INT_MASK: u32 = 0x001f_ffff;
    const DATE_MASK: u32 = 0x0fff_ffff;

    /// Creates a reset general-purpose SPI controller and its waveform
    /// signals. `name` should be `esp32s3.spi2` or `esp32s3.spi3`.
    pub fn new(name: impl Into<String>, hub: SignalHub) -> Result<Self, SignalError> {
        let name = name.into();
        let mosi = hub.declare(
            format!("{name}.mosi"),
            SignalValue::from_u64(0, 1)?,
            Some("SPI master-out/slave-in data".to_owned()),
        )?;
        let miso = hub.declare(
            format!("{name}.miso"),
            SignalValue::from_u64(0, 1)?,
            Some("SPI master-in/slave-out data".to_owned()),
        )?;
        let sclk = hub.declare(
            format!("{name}.sclk"),
            SignalValue::from_u64(0, 1)?,
            Some("SPI serial clock".to_owned()),
        )?;
        let cs0 = hub.declare(
            format!("{name}.cs0"),
            SignalValue::from_u64(1, 1)?,
            Some("SPI chip-select zero, active low".to_owned()),
        )?;
        let transfer_done = hub.declare(
            format!("{name}.transfer_done"),
            SignalValue::from_u64(0, 1)?,
            Some("Functional transfer completion strobe".to_owned()),
        )?;

        let mut spi = Self {
            name,
            registers: [0; Self::REGISTER_WORDS],
            buffers: [0; 16],
            dma_raw: 0,
            dma_enable: 0,
            transfer: Esp32s3SpiTransfer::default(),
            hub,
            mosi,
            miso,
            sclk,
            cs0,
            transfer_done,
        };
        spi.reset_registers();
        spi.reset_signals(SimTime::ZERO)?;
        Ok(spi)
    }

    /// Returns a copy of the host-observable transfer summary.
    pub fn transfer(&self) -> Esp32s3SpiTransfer {
        self.transfer.clone()
    }

    fn register_index(offset: u64) -> Result<usize, DeviceError> {
        if offset >= Self::REGISTER_BYTES || offset & 3 != 0 {
            return Err(DeviceError::new(format!(
                "ESP32-S3 SPI requires an aligned register offset, got {offset:#x}"
            )));
        }
        usize::try_from(offset / 4).map_err(|_| DeviceError::new("SPI register offset overflow"))
    }

    fn buffer_index(offset: u64) -> Option<usize> {
        if offset & 3 != 0 || !(Self::W0..=Self::W15).contains(&offset) {
            return None;
        }
        Some(((offset - Self::W0) / 4) as usize)
    }

    fn set_signal(&self, signal: SignalId, value: u64, at: SimTime) -> Result<(), SignalError> {
        self.hub.set(
            signal,
            SignalValue::from_u64(value, 1).expect("one-bit SPI signal is valid"),
            at,
        )
    }

    fn signal_error(error: SignalError) -> DeviceError {
        DeviceError::new(error.to_string())
    }

    fn reset_signals(&self, at: SimTime) -> Result<(), SignalError> {
        self.set_signal(self.mosi, 0, at)?;
        self.set_signal(self.miso, 0, at)?;
        self.set_signal(self.sclk, 0, at)?;
        self.set_signal(self.cs0, 1, at)?;
        self.set_signal(self.transfer_done, 0, at)
    }

    fn reset_registers(&mut self) {
        self.registers.fill(0);
        self.buffers.fill(0);
        self.dma_raw = 0;
        self.dma_enable = 0;
        self.transfer = Esp32s3SpiTransfer::default();
        self.registers[Self::USER as usize / 4] = Self::USER_COMMAND;
        self.registers[Self::DATE as usize / 4] = 0x0210_1190;
    }

    fn data_bit_length(&self) -> usize {
        let encoded = self.registers[Self::MS_DLEN as usize / 4] & Self::DATA_BIT_LENGTH_MASK;
        // The hardware encodes the number of bits as bit_count - 1. The
        // register is wider than the 512-bit W0-W15 functional FIFO, so the
        // model bounds an otherwise unrepresentable transaction to that FIFO.
        usize::try_from(encoded + 1)
            .expect("SPI data length fits usize")
            .min(512)
    }

    fn tx_bytes(&self, bit_length: usize) -> Vec<u8> {
        let byte_length = bit_length.div_ceil(8);
        let mut bytes = Vec::with_capacity(byte_length);
        for word in self.buffers {
            bytes.extend_from_slice(&word.to_be_bytes());
            if bytes.len() >= byte_length {
                break;
            }
        }
        bytes.truncate(byte_length);
        bytes
    }

    fn execute_transfer(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let user = self.registers[Self::USER as usize / 4];
        let bit_length = self.data_bit_length();
        let bytes = self.tx_bytes(bit_length);
        let mut bit_index = 0;

        self.set_signal(self.cs0, 0, at)
            .map_err(Self::signal_error)?;
        for byte in &bytes {
            for bit in (0..8).rev() {
                if bit_index >= bit_length {
                    break;
                }
                let value = u64::from((byte >> bit) & 1);
                if user & Self::USER_MOSI != 0 || user & Self::USER_DOUTDIN != 0 {
                    self.set_signal(self.mosi, value, at)
                        .map_err(Self::signal_error)?;
                }
                if user & Self::USER_MISO != 0 || user & Self::USER_DOUTDIN != 0 {
                    // The functional endpoint is a deterministic loopback.
                    self.set_signal(self.miso, value, at)
                        .map_err(Self::signal_error)?;
                }
                self.set_signal(self.sclk, 0, at)
                    .map_err(Self::signal_error)?;
                self.set_signal(self.sclk, 1, at)
                    .map_err(Self::signal_error)?;
                bit_index += 1;
            }
        }
        self.set_signal(self.sclk, 0, at)
            .map_err(Self::signal_error)?;
        self.set_signal(self.cs0, 1, at)
            .map_err(Self::signal_error)?;

        // DOUTDIN is the useful baseline for compiler and board tests. Keep
        // the FIFO contents unchanged: firmware can inspect its TX/RX bytes
        // without a second host-side buffer API.
        self.transfer.count = self.transfer.count.saturating_add(1);
        self.transfer.bytes = bytes;
        self.dma_raw |= Self::TRANS_DONE_INT;
        self.set_signal(self.transfer_done, self.transfer.count & 1, at)
            .map_err(Self::signal_error)?;
        Ok(())
    }
}

impl Device for Esp32s3Spi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP32-S3 SPI requires word access"));
        }
        if let Some(index) = Self::buffer_index(offset) {
            return Ok(u64::from(self.buffers[index]));
        }
        match offset {
            Self::DMA_INT_RAW => Ok(u64::from(self.dma_raw)),
            Self::DMA_INT_ST => Ok(u64::from(self.dma_raw & self.dma_enable)),
            Self::DMA_INT_ENA => Ok(u64::from(self.dma_enable)),
            Self::DMA_INT_SET => Ok(0),
            _ => Ok(u64::from(self.registers[Self::register_index(offset)?])),
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
            return Err(DeviceError::new("ESP32-S3 SPI requires word access"));
        }
        let value = u32::try_from(value).map_err(|_| DeviceError::new("SPI value exceeds u32"))?;
        if let Some(index) = Self::buffer_index(offset) {
            self.buffers[index] = value;
            return Ok(());
        }
        match offset {
            Self::CMD => {
                let index = Self::register_index(offset)?;
                self.registers[index] = 0;
                if value & Self::USER_TRANSACTION != 0 {
                    self.execute_transfer(at)?;
                }
            }
            Self::USER => self.registers[Self::USER as usize / 4] = value & Self::USER_MASK,
            Self::MS_DLEN => {
                self.registers[Self::MS_DLEN as usize / 4] = value & Self::DATA_BIT_LENGTH_MASK;
            }
            Self::DMA_INT_CLR => self.dma_raw &= !(value & Self::DMA_INT_MASK),
            Self::DMA_INT_SET => self.dma_raw |= value & Self::DMA_INT_MASK,
            Self::DMA_INT_ENA => self.dma_enable = value & Self::DMA_INT_MASK,
            Self::DMA_INT_RAW => self.dma_raw = value & Self::DMA_INT_MASK,
            Self::DMA_INT_ST => {
                return Err(DeviceError::new(format!(
                    "{} register {offset:#x} is read-only",
                    self.name
                )));
            }
            Self::DATE => self.registers[Self::DATE as usize / 4] = value & Self::DATE_MASK,
            _ => {
                let index = Self::register_index(offset)?;
                self.registers[index] = value;
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset_registers();
        self.reset_signals(SimTime::ZERO)
            .expect("SPI reset signals remain declared");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn executes_loopback_transaction_and_reports_completion() {
        let hub = SignalHub::new();
        let mut spi = Esp32s3Spi::new("board.esp32s3.spi2", hub.clone()).unwrap();
        spi.write(
            Esp32s3Spi::W0,
            AccessWidth::Word,
            0xa55a_0000,
            SimTime::ZERO,
        )
        .unwrap();
        spi.write(Esp32s3Spi::MS_DLEN, AccessWidth::Word, 15, SimTime::ZERO)
            .unwrap();
        spi.write(
            Esp32s3Spi::USER,
            AccessWidth::Word,
            u64::from(Esp32s3Spi::USER_MOSI | Esp32s3Spi::USER_MISO | Esp32s3Spi::USER_DOUTDIN),
            SimTime::ZERO,
        )
        .unwrap();
        spi.write(
            Esp32s3Spi::CMD,
            AccessWidth::Word,
            u64::from(Esp32s3Spi::USER_TRANSACTION),
            SimTime::from_ticks(1),
        )
        .unwrap();

        assert_eq!(
            spi.read(Esp32s3Spi::W0, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0xa55a_0000
        );
        assert_eq!(
            spi.read(Esp32s3Spi::DMA_INT_RAW, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1 << 12
        );
        assert_eq!(
            spi.transfer(),
            Esp32s3SpiTransfer {
                count: 1,
                bytes: vec![0xa5, 0x5a]
            }
        );
        assert!(hub.with_registry(|registry| registry.find("board.esp32s3.spi2.cs0").is_some()));
        assert!(hub.with_registry(|registry| registry.find("board.esp32s3.spi2.sclk").is_some()));
    }

    #[test]
    fn completion_interrupt_can_be_cleared_and_reset_restores_idle_state() {
        let hub = SignalHub::new();
        let mut spi = Esp32s3Spi::new("board.esp32s3.spi3", hub).unwrap();
        spi.write(Esp32s3Spi::CMD, AccessWidth::Word, 1 << 24, SimTime::ZERO)
            .unwrap();
        assert_ne!(
            spi.read(Esp32s3Spi::DMA_INT_RAW, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        spi.write(
            Esp32s3Spi::DMA_INT_CLR,
            AccessWidth::Word,
            1 << 12,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            spi.read(Esp32s3Spi::DMA_INT_RAW, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        spi.reset(ResetKind::External);
        assert_eq!(spi.transfer().count, 0);
        assert_eq!(
            spi.read(Esp32s3Spi::DATE, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x0210_1190
        );
    }

    #[test]
    fn register_ids_masks_and_interrupt_aliases_match_the_native_contract() {
        for register in [
            Esp32s3SpiRegister::Cmd,
            Esp32s3SpiRegister::User,
            Esp32s3SpiRegister::MsDlen,
            Esp32s3SpiRegister::DmaIntEna,
            Esp32s3SpiRegister::DmaIntClr,
            Esp32s3SpiRegister::DmaIntRaw,
            Esp32s3SpiRegister::DmaIntSt,
            Esp32s3SpiRegister::DmaIntSet,
            Esp32s3SpiRegister::W0,
            Esp32s3SpiRegister::W15,
            Esp32s3SpiRegister::Date,
        ] {
            assert_eq!(
                Esp32s3SpiRegister::from_offset(register.offset()),
                Some(register)
            );
        }

        let hub = SignalHub::new();
        let mut spi = Esp32s3Spi::new("board.esp32s3.spi2", hub).unwrap();
        spi.write(
            Esp32s3Spi::USER,
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            spi.read(Esp32s3Spi::USER, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(Esp32s3Spi::USER_MASK)
        );
        spi.write(
            Esp32s3Spi::MS_DLEN,
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            spi.read(Esp32s3Spi::MS_DLEN, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(Esp32s3Spi::DATA_BIT_LENGTH_MASK)
        );

        spi.write(
            Esp32s3Spi::DMA_INT_ENA,
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            spi.read(Esp32s3Spi::DMA_INT_ENA, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(Esp32s3Spi::DMA_INT_MASK)
        );
        spi.write(
            Esp32s3Spi::DMA_INT_RAW,
            AccessWidth::Word,
            u64::from(Esp32s3Spi::TRANS_DONE_INT),
            SimTime::ZERO,
        )
        .unwrap();
        spi.write(
            Esp32s3Spi::DMA_INT_CLR,
            AccessWidth::Word,
            u64::from(Esp32s3Spi::TRANS_DONE_INT),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            spi.read(Esp32s3Spi::DMA_INT_RAW, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
        spi.write(
            Esp32s3Spi::DMA_INT_SET,
            AccessWidth::Word,
            u64::from(Esp32s3Spi::TRANS_DONE_INT),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            spi.read(Esp32s3Spi::DMA_INT_RAW, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(Esp32s3Spi::TRANS_DONE_INT)
        );

        assert!(
            spi.read(Esp32s3Spi::W0 + 1, AccessWidth::Word, SimTime::ZERO)
                .is_err()
        );
        assert!(
            spi.write(Esp32s3Spi::W0 + 1, AccessWidth::Word, 0, SimTime::ZERO)
                .is_err()
        );
        assert!(
            spi.write(
                Esp32s3Spi::DMA_INT_ST,
                AccessWidth::Word,
                u64::from(Esp32s3Spi::TRANS_DONE_INT),
                SimTime::ZERO,
            )
            .is_err()
        );
    }
}
