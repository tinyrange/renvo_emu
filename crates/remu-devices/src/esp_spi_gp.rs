use super::*;

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
    pub const CMD: u64 = 0x00;
    /// `SPI_USER_REG`: user transaction feature enables.
    pub const USER: u64 = 0x10;
    /// `SPI_MS_DLEN_REG`: master data length, encoded as bit count minus one.
    pub const MS_DLEN: u64 = 0x1c;
    /// `SPI_DMA_INT_ENA_REG`: DMA interrupt enable bits.
    pub const DMA_INT_ENA: u64 = 0x34;
    /// `SPI_DMA_INT_CLR_REG`: DMA interrupt clear bits.
    pub const DMA_INT_CLR: u64 = 0x38;
    /// `SPI_DMA_INT_RAW_REG`: raw DMA interrupt bits.
    pub const DMA_INT_RAW: u64 = 0x3c;
    /// `SPI_DMA_INT_ST_REG`: enabled DMA interrupt status bits.
    pub const DMA_INT_ST: u64 = 0x40;
    /// First data buffer register (`SPI_W0_REG`).
    pub const W0: u64 = 0x98;
    /// Last data buffer register (`SPI_W15_REG`).
    pub const W15: u64 = 0xd4;
    /// `SPI_DATE_REG`.
    pub const DATE: u64 = 0xf0;

    const USER_TRANSACTION: u32 = 1 << 24;
    const TRANS_DONE_INT: u32 = 1 << 12;
    const DATA_BIT_LENGTH_MASK: u32 = 0x3ffff;
    const USER_MISO: u32 = 1 << 28;
    const USER_MOSI: u32 = 1 << 27;
    const USER_DOUTDIN: u32 = 1 << 0;

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
        spi.reset_signals(SimTime::ZERO);
        spi.registers[Self::DATE as usize / 4] = 0x0210_1190;
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
        (Self::W0..=Self::W15)
            .contains(&offset)
            .then(|| usize::try_from((offset - Self::W0) / 4).ok())
            .flatten()
    }

    fn set_signal(&self, signal: SignalId, value: u64, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, 1).expect("one-bit SPI signal is valid"),
                at,
            )
            .expect("SPI signal remains declared");
    }

    fn reset_signals(&self, at: SimTime) {
        self.set_signal(self.mosi, 0, at);
        self.set_signal(self.miso, 0, at);
        self.set_signal(self.sclk, 0, at);
        self.set_signal(self.cs0, 1, at);
        self.set_signal(self.transfer_done, 0, at);
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

    fn execute_transfer(&mut self, at: SimTime) {
        let user = self.registers[Self::USER as usize / 4];
        let bit_length = self.data_bit_length();
        let bytes = self.tx_bytes(bit_length);
        let mut bit_index = 0;

        self.set_signal(self.cs0, 0, at);
        for byte in &bytes {
            for bit in (0..8).rev() {
                if bit_index >= bit_length {
                    break;
                }
                let value = u64::from((byte >> bit) & 1);
                if user & Self::USER_MOSI != 0 || user & Self::USER_DOUTDIN != 0 {
                    self.set_signal(self.mosi, value, at);
                }
                if user & Self::USER_MISO != 0 || user & Self::USER_DOUTDIN != 0 {
                    // The functional endpoint is a deterministic loopback.
                    self.set_signal(self.miso, value, at);
                }
                self.set_signal(self.sclk, 0, at);
                self.set_signal(self.sclk, 1, at);
                bit_index += 1;
            }
        }
        self.set_signal(self.sclk, 0, at);
        self.set_signal(self.cs0, 1, at);

        // DOUTDIN is the useful baseline for compiler and board tests. Keep
        // the FIFO contents unchanged: firmware can inspect its TX/RX bytes
        // without a second host-side buffer API.
        self.transfer.count = self.transfer.count.saturating_add(1);
        self.transfer.bytes = bytes;
        self.dma_raw |= Self::TRANS_DONE_INT;
        self.set_signal(self.transfer_done, self.transfer.count & 1, at);
    }
}

impl Device for Esp32s3Spi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("ESP32-S3 SPI requires word access"));
        }
        if let Some(index) = Self::buffer_index(offset) {
            return Ok(u64::from(self.buffers[index]));
        }
        match offset {
            Self::DMA_INT_RAW => Ok(u64::from(self.dma_raw)),
            Self::DMA_INT_ST => Ok(u64::from(self.dma_raw & self.dma_enable)),
            Self::DMA_INT_ENA => Ok(u64::from(self.dma_enable)),
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
        if width != AccessWidth::Word {
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
                self.registers[index] = value & !Self::USER_TRANSACTION;
                if value & Self::USER_TRANSACTION != 0 {
                    self.execute_transfer(at);
                }
            }
            Self::DMA_INT_CLR => self.dma_raw &= !value,
            Self::DMA_INT_ENA => self.dma_enable = value,
            Self::DMA_INT_RAW | Self::DMA_INT_ST => {
                return Err(DeviceError::new(format!(
                    "{} register {offset:#x} is read-only",
                    self.name
                )));
            }
            _ => {
                let index = Self::register_index(offset)?;
                self.registers[index] = value;
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
        self.registers[Self::DATE as usize / 4] = 0x0210_1190;
        self.buffers.fill(0);
        self.dma_raw = 0;
        self.dma_enable = 0;
        self.transfer = Esp32s3SpiTransfer::default();
        self.reset_signals(SimTime::ZERO);
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
}
