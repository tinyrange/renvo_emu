use super::*;

/// Host-visible summary of one functional ESP32-S3 I2S transfer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Esp32s3I2sTransfer {
    /// Number of completed TX frames.
    pub tx_frames: u64,
    /// Number of completed RX frames.
    pub rx_frames: u64,
    /// Most recent deterministic TX sample.
    pub last_tx: u32,
    /// Most recent deterministic RX sample.
    pub last_rx: u32,
}

/// Functional ESP32-S3 I2S0/I2S1 controller model.
///
/// The model covers the register and signal path useful for deterministic
/// firmware and board tests: standard-mode single-data TX/RX starts, the
/// controller idle state, and TX/RX interrupt raw/status/clear bits. A TX
/// start emits one stereo frame using `CONF_SIGLE_DATA`, while loopback mode
/// makes that sample available to a subsequent RX start. DMA descriptors,
/// PDM/TDM transforms, and clock-frequency fidelity are intentionally outside
/// this functional slice.
pub struct Esp32s3I2s {
    name: String,
    registers: [u32; Self::REGISTER_WORDS],
    interrupt_raw: u32,
    interrupt_enable: u32,
    transfer: Esp32s3I2sTransfer,
    hub: SignalHub,
    mclk: SignalId,
    bclk: SignalId,
    ws: SignalId,
    dout: SignalId,
    din: SignalId,
    tx_done: SignalId,
    rx_done: SignalId,
    mclk_level: bool,
}

impl Esp32s3I2s {
    const REGISTER_BYTES: u64 = 0x100;
    const REGISTER_WORDS: usize = (Self::REGISTER_BYTES / 4) as usize;

    /// `I2S_INT_RAW_REG`: raw TX/RX completion status.
    pub const INT_RAW: u64 = 0x0c;
    /// `I2S_INT_ST_REG`: enabled TX/RX completion status.
    pub const INT_ST: u64 = 0x10;
    /// `I2S_INT_ENA_REG`: TX/RX completion interrupt enables.
    pub const INT_ENA: u64 = 0x14;
    /// `I2S_INT_CLR_REG`: write-one-to-clear TX/RX completion status.
    pub const INT_CLR: u64 = 0x18;
    /// `I2S_RX_CONF_REG`: receiver control and start/reset bits.
    pub const RX_CONF: u64 = 0x20;
    /// `I2S_TX_CONF_REG`: transmitter control and start/reset bits.
    pub const TX_CONF: u64 = 0x24;
    /// `I2S_RX_CONF1_REG`: receiver sample-width configuration.
    pub const RX_CONF1: u64 = 0x28;
    /// `I2S_TX_CONF1_REG`: transmitter sample-width configuration.
    pub const TX_CONF1: u64 = 0x2c;
    /// `I2S_RX_CLKM_CONF_REG`: receiver clock source/divider.
    pub const RX_CLKM_CONF: u64 = 0x30;
    /// `I2S_TX_CLKM_CONF_REG`: transmitter clock source/divider.
    pub const TX_CLKM_CONF: u64 = 0x34;
    /// `I2S_RXEOF_NUM_REG`: DMA receive frame length configuration.
    pub const RXEOF_NUM: u64 = 0x64;
    /// `I2S_CONF_SIGLE_DATA_REG`: deterministic single-data TX sample.
    pub const SINGLE_DATA: u64 = 0x68;
    /// `I2S_STATE_REG`: controller state, including TX idle.
    pub const STATE: u64 = 0x6c;
    /// `I2S_DATE_REG`: hardware version register.
    pub const DATE: u64 = 0x80;

    const RX_DONE_INT: u32 = 1;
    const TX_DONE_INT: u32 = 1 << 1;
    const RX_START: u32 = 1 << 2;
    const RX_FIFO_RESET: u32 = 1 << 1;
    const RX_RESET: u32 = 1;
    const TX_START: u32 = 1 << 2;
    const TX_FIFO_RESET: u32 = 1 << 1;
    const TX_RESET: u32 = 1;
    const TX_LOOPBACK: u32 = 1 << 27;
    const SAMPLE_BITS_SHIFT: u32 = 13;
    const SAMPLE_BITS_MASK: u32 = 0x1f;
    const TX_IDLE: u32 = 1;

    /// Creates an I2S controller and declares its digital waveform signals.
    pub fn new(name: impl Into<String>, hub: SignalHub) -> Result<Self, SignalError> {
        let name = name.into();
        let mclk = hub.declare(
            format!("{name}.mclk"),
            SignalValue::from_u64(0, 1)?,
            Some("I2S master clock".to_owned()),
        )?;
        let bclk = hub.declare(
            format!("{name}.bclk"),
            SignalValue::from_u64(0, 1)?,
            Some("I2S bit clock".to_owned()),
        )?;
        let ws = hub.declare(
            format!("{name}.ws"),
            SignalValue::from_u64(0, 1)?,
            Some("I2S word-select/channel clock".to_owned()),
        )?;
        let dout = hub.declare(
            format!("{name}.dout"),
            SignalValue::from_u64(0, 1)?,
            Some("I2S serial data output".to_owned()),
        )?;
        let din = hub.declare(
            format!("{name}.din"),
            SignalValue::from_u64(0, 1)?,
            Some("I2S serial data input".to_owned()),
        )?;
        let tx_done = hub.declare(
            format!("{name}.tx_done"),
            SignalValue::from_u64(0, 1)?,
            Some("I2S TX completion strobe".to_owned()),
        )?;
        let rx_done = hub.declare(
            format!("{name}.rx_done"),
            SignalValue::from_u64(0, 1)?,
            Some("I2S RX completion strobe".to_owned()),
        )?;

        let mut i2s = Self {
            name,
            registers: [0; Self::REGISTER_WORDS],
            interrupt_raw: 0,
            interrupt_enable: 0,
            transfer: Esp32s3I2sTransfer::default(),
            hub,
            mclk,
            bclk,
            ws,
            dout,
            din,
            tx_done,
            rx_done,
            mclk_level: false,
        };
        i2s.reset_signals(SimTime::ZERO);
        i2s.registers[Self::TX_CONF1 as usize / 4] = (15 << Self::SAMPLE_BITS_SHIFT) | (1 << 29);
        i2s.registers[Self::RX_CONF1 as usize / 4] = (15 << Self::SAMPLE_BITS_SHIFT) | (1 << 29);
        i2s.registers[Self::TX_CONF as usize / 4] = (1 << 13) | (1 << 12);
        i2s.registers[Self::RX_CONF as usize / 4] = 1 << 12;
        i2s.registers[Self::STATE as usize / 4] = Self::TX_IDLE;
        i2s.registers[Self::DATE as usize / 4] = 0x0200_9070;
        Ok(i2s)
    }

    /// Returns a copy of the most recent deterministic transfer summary.
    pub fn transfer(&self) -> Esp32s3I2sTransfer {
        self.transfer
    }

    fn register_index(offset: u64) -> Result<usize, DeviceError> {
        if offset >= Self::REGISTER_BYTES || offset & 3 != 0 {
            return Err(DeviceError::new(format!(
                "ESP32-S3 I2S requires an aligned register offset, got {offset:#x}"
            )));
        }
        usize::try_from(offset / 4).map_err(|_| DeviceError::new("I2S register offset overflow"))
    }

    fn set_signal(&self, signal: SignalId, value: u64, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, 1).expect("one-bit I2S signal is valid"),
                at,
            )
            .expect("I2S signal remains declared");
    }

    fn reset_signals(&self, at: SimTime) {
        self.set_signal(self.mclk, 0, at);
        self.set_signal(self.bclk, 0, at);
        self.set_signal(self.ws, 0, at);
        self.set_signal(self.dout, 0, at);
        self.set_signal(self.din, 0, at);
        self.set_signal(self.tx_done, 0, at);
        self.set_signal(self.rx_done, 0, at);
    }

    fn pulse(&self, signal: SignalId, at: SimTime) {
        self.set_signal(signal, 1, at);
        self.set_signal(signal, 0, at);
    }

    fn sample_bits(&self, tx: bool) -> u32 {
        let register = if tx { Self::TX_CONF1 } else { Self::RX_CONF1 };
        ((self.registers[register as usize / 4] >> Self::SAMPLE_BITS_SHIFT)
            & Self::SAMPLE_BITS_MASK)
            .saturating_add(1)
            .min(32)
    }

    fn emit_frame(&mut self, sample: u32, bits: u32, at: SimTime) {
        self.registers[Self::STATE as usize / 4] &= !Self::TX_IDLE;
        for channel in 0_u32..2 {
            self.set_signal(self.ws, u64::from(channel), at);
            for bit in (0..bits).rev() {
                let value = u64::from((sample >> bit) & 1);
                self.set_signal(self.dout, value, at);
                self.set_signal(self.din, value, at);
                self.set_signal(self.bclk, 0, at);
                self.mclk_level = !self.mclk_level;
                self.set_signal(self.mclk, u64::from(self.mclk_level), at);
                self.set_signal(self.bclk, 1, at);
                self.mclk_level = !self.mclk_level;
                self.set_signal(self.mclk, u64::from(self.mclk_level), at);
            }
        }
        self.set_signal(self.bclk, 0, at);
        self.set_signal(self.ws, 0, at);
        self.set_signal(self.dout, 0, at);
        self.set_signal(self.din, 0, at);
        self.set_signal(self.mclk, 0, at);
        self.mclk_level = false;
        self.registers[Self::STATE as usize / 4] |= Self::TX_IDLE;
    }

    fn execute_tx(&mut self, at: SimTime) {
        let sample = self.registers[Self::SINGLE_DATA as usize / 4];
        self.emit_frame(sample, self.sample_bits(true), at);
        self.transfer.tx_frames = self.transfer.tx_frames.saturating_add(1);
        self.transfer.last_tx = sample;
        self.interrupt_raw |= Self::TX_DONE_INT;
        self.pulse(self.tx_done, at);
    }

    fn execute_rx(&mut self, at: SimTime) {
        let sample = if self.registers[Self::TX_CONF as usize / 4] & Self::TX_LOOPBACK != 0 {
            self.transfer.last_tx
        } else {
            0
        };
        self.emit_rx_frame(sample, self.sample_bits(false), at);
        self.transfer.rx_frames = self.transfer.rx_frames.saturating_add(1);
        self.transfer.last_rx = sample;
        self.interrupt_raw |= Self::RX_DONE_INT;
        self.pulse(self.rx_done, at);
    }

    fn emit_rx_frame(&mut self, sample: u32, bits: u32, at: SimTime) {
        for channel in 0_u32..2 {
            self.set_signal(self.ws, u64::from(channel), at);
            for bit in (0..bits).rev() {
                let value = u64::from((sample >> bit) & 1);
                self.set_signal(self.din, value, at);
                self.set_signal(self.bclk, 0, at);
                self.mclk_level = !self.mclk_level;
                self.set_signal(self.mclk, u64::from(self.mclk_level), at);
                self.set_signal(self.bclk, 1, at);
                self.mclk_level = !self.mclk_level;
                self.set_signal(self.mclk, u64::from(self.mclk_level), at);
            }
        }
        self.set_signal(self.bclk, 0, at);
        self.set_signal(self.ws, 0, at);
        self.set_signal(self.din, 0, at);
        self.set_signal(self.mclk, 0, at);
        self.mclk_level = false;
    }

    fn reset_tx(&mut self, at: SimTime) {
        self.registers[Self::TX_CONF as usize / 4] &=
            !(Self::TX_START | Self::TX_FIFO_RESET | Self::TX_RESET);
        self.registers[Self::STATE as usize / 4] |= Self::TX_IDLE;
        self.set_signal(self.dout, 0, at);
        self.set_signal(self.bclk, 0, at);
    }

    fn reset_rx(&mut self, at: SimTime) {
        self.registers[Self::RX_CONF as usize / 4] &=
            !(Self::RX_START | Self::RX_FIFO_RESET | Self::RX_RESET);
        self.set_signal(self.din, 0, at);
    }
}

impl Device for Esp32s3I2s {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("ESP32-S3 I2S requires word access"));
        }
        match offset {
            Self::INT_RAW => Ok(u64::from(self.interrupt_raw)),
            Self::INT_ST => Ok(u64::from(self.interrupt_raw & self.interrupt_enable)),
            Self::INT_ENA => Ok(u64::from(self.interrupt_enable)),
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
            return Err(DeviceError::new("ESP32-S3 I2S requires word access"));
        }
        let value = u32::try_from(value).map_err(|_| DeviceError::new("I2S value exceeds u32"))?;
        match offset {
            Self::INT_RAW | Self::INT_CLR => self.interrupt_raw &= !value,
            Self::INT_ENA => self.interrupt_enable = value & 0x0f,
            Self::TX_CONF => {
                let index = Self::register_index(offset)?;
                self.registers[index] =
                    value & !(Self::TX_START | Self::TX_FIFO_RESET | Self::TX_RESET);
                if value & (Self::TX_FIFO_RESET | Self::TX_RESET) != 0 {
                    self.reset_tx(at);
                }
                if value & Self::TX_START != 0 {
                    self.execute_tx(at);
                }
            }
            Self::RX_CONF => {
                let index = Self::register_index(offset)?;
                self.registers[index] =
                    value & !(Self::RX_START | Self::RX_FIFO_RESET | Self::RX_RESET);
                if value & (Self::RX_FIFO_RESET | Self::RX_RESET) != 0 {
                    self.reset_rx(at);
                }
                if value & Self::RX_START != 0 {
                    self.execute_rx(at);
                }
            }
            Self::INT_ST | Self::STATE => {
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
        self.registers[Self::TX_CONF1 as usize / 4] = (15 << Self::SAMPLE_BITS_SHIFT) | (1 << 29);
        self.registers[Self::RX_CONF1 as usize / 4] = (15 << Self::SAMPLE_BITS_SHIFT) | (1 << 29);
        self.registers[Self::TX_CONF as usize / 4] = (1 << 13) | (1 << 12);
        self.registers[Self::RX_CONF as usize / 4] = 1 << 12;
        self.registers[Self::STATE as usize / 4] = Self::TX_IDLE;
        self.registers[Self::DATE as usize / 4] = 0x0200_9070;
        self.interrupt_raw = 0;
        self.interrupt_enable = 0;
        self.transfer = Esp32s3I2sTransfer::default();
        self.mclk_level = false;
        self.reset_signals(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transmits_single_data_stereo_frame_and_emits_signals() {
        let hub = SignalHub::new();
        let mut i2s = Esp32s3I2s::new("board.esp32s3.i2s0", hub.clone()).unwrap();
        i2s.write(
            Esp32s3I2s::SINGLE_DATA,
            AccessWidth::Word,
            0xa55a_1234,
            SimTime::ZERO,
        )
        .unwrap();
        i2s.write(
            Esp32s3I2s::TX_CONF1,
            AccessWidth::Word,
            15 << 13,
            SimTime::ZERO,
        )
        .unwrap();
        i2s.write(
            Esp32s3I2s::TX_CONF,
            AccessWidth::Word,
            1 << 2,
            SimTime::from_ticks(1),
        )
        .unwrap();

        assert_eq!(i2s.transfer().tx_frames, 1);
        assert_eq!(i2s.transfer().last_tx, 0xa55a_1234);
        assert_eq!(
            i2s.read(Esp32s3I2s::INT_RAW, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1 << 1
        );
        assert_eq!(
            i2s.read(Esp32s3I2s::STATE, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
        assert!(hub.with_registry(|registry| registry.find("board.esp32s3.i2s0.bclk").is_some()));
        assert!(hub.with_registry(|registry| registry.find("board.esp32s3.i2s0.dout").is_some()));
    }

    #[test]
    fn loopback_rx_and_interrupt_clear_are_deterministic() {
        let hub = SignalHub::new();
        let mut i2s = Esp32s3I2s::new("board.esp32s3.i2s1", hub).unwrap();
        i2s.write(
            Esp32s3I2s::SINGLE_DATA,
            AccessWidth::Word,
            0x55aa,
            SimTime::ZERO,
        )
        .unwrap();
        i2s.write(
            Esp32s3I2s::TX_CONF,
            AccessWidth::Word,
            1 << 27 | 1 << 2,
            SimTime::ZERO,
        )
        .unwrap();
        i2s.write(
            Esp32s3I2s::RX_CONF,
            AccessWidth::Word,
            1 << 2,
            SimTime::from_ticks(1),
        )
        .unwrap();
        assert_eq!(i2s.transfer().last_rx, 0x55aa);
        assert_eq!(
            i2s.read(Esp32s3I2s::INT_RAW, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x3
        );
        i2s.write(Esp32s3I2s::INT_CLR, AccessWidth::Word, 0x3, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            i2s.read(Esp32s3I2s::INT_RAW, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
    }
}
