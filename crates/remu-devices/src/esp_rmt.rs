//! Functional ESP32-S3 remote-control (RMT) transmitter channels.
//!
//! The model deliberately stops at the useful firmware boundary: a program can
//! fill a channel FIFO, start a transfer, observe completion, and inspect the
//! resulting pulse waveform.  It does not claim DMA, carrier modulation, RX,
//! or source-clock accuracy.
//!
//! Register offsets and reset values follow Espressif's official
//! [ESP32-S3 RMT register definitions](https://raw.githubusercontent.com/espressif/esp-idf/master/components/soc/esp32s3/register/soc/rmt_reg.h).

use super::*;

const TX_CHANNELS: usize = 4;

/// Named ESP32-S3 RMT register offsets covered by the functional model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum Esp32s3RmtRegister {
    /// Channel 0 APB FIFO data register.
    Ch0Data = 0x00,
    /// Channel 1 APB FIFO data register.
    Ch1Data = 0x04,
    /// Channel 2 APB FIFO data register.
    Ch2Data = 0x08,
    /// Channel 3 APB FIFO data register.
    Ch3Data = 0x0c,
    /// Channel 0 configuration register.
    Ch0Conf0 = 0x20,
    /// Channel 1 configuration register.
    Ch1Conf0 = 0x24,
    /// Channel 2 configuration register.
    Ch2Conf0 = 0x28,
    /// Channel 3 configuration register.
    Ch3Conf0 = 0x2c,
    /// Channel 0 status register.
    Ch0Status = 0x50,
    /// Channel 1 status register.
    Ch1Status = 0x54,
    /// Channel 2 status register.
    Ch2Status = 0x58,
    /// Channel 3 status register.
    Ch3Status = 0x5c,
    /// Raw interrupt status.
    IntRaw = 0x70,
    /// Masked interrupt status.
    IntSt = 0x74,
    /// Interrupt enables.
    IntEna = 0x78,
    /// Write-one-to-clear interrupt bits.
    IntClr = 0x7c,
    /// APB and memory clock configuration.
    SysConf = 0xc0,
    /// Synchronous transmit trigger.
    TxSim = 0xc4,
    /// RMT version register.
    Date = 0xcc,
}

impl Esp32s3RmtRegister {
    /// Returns the native byte offset of this register.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    /// Converts a modeled native byte offset into a named register.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::Ch0Data,
            0x04 => Self::Ch1Data,
            0x08 => Self::Ch2Data,
            0x0c => Self::Ch3Data,
            0x20 => Self::Ch0Conf0,
            0x24 => Self::Ch1Conf0,
            0x28 => Self::Ch2Conf0,
            0x2c => Self::Ch3Conf0,
            0x50 => Self::Ch0Status,
            0x54 => Self::Ch1Status,
            0x58 => Self::Ch2Status,
            0x5c => Self::Ch3Status,
            0x70 => Self::IntRaw,
            0x74 => Self::IntSt,
            0x78 => Self::IntEna,
            0x7c => Self::IntClr,
            0xc0 => Self::SysConf,
            0xc4 => Self::TxSim,
            0xcc => Self::Date,
            _ => return None,
        })
    }
}

const DATA_BASE: u64 = Esp32s3RmtRegister::Ch0Data.offset();
const CONF0_BASE: u64 = Esp32s3RmtRegister::Ch0Conf0.offset();
const STATUS_BASE: u64 = Esp32s3RmtRegister::Ch0Status.offset();
const INT_RAW: u64 = Esp32s3RmtRegister::IntRaw.offset();
const INT_ST: u64 = Esp32s3RmtRegister::IntSt.offset();
const INT_ENA: u64 = Esp32s3RmtRegister::IntEna.offset();
const INT_CLR: u64 = Esp32s3RmtRegister::IntClr.offset();
const SYS_CONF: u64 = Esp32s3RmtRegister::SysConf.offset();
const TX_SIM: u64 = Esp32s3RmtRegister::TxSim.offset();
const DATE: u64 = Esp32s3RmtRegister::Date.offset();
const TX_START: u32 = 1 << 0;
const MEM_RD_RST: u32 = 1 << 1;
const APB_MEM_RST: u32 = 1 << 2;
const TX_STOP: u32 = 1 << 7;
const CARRIER_EFF_EN: u32 = 1 << 20;
const CARRIER_EN: u32 = 1 << 21;
const CARRIER_OUT_LV: u32 = 1 << 22;
const CONF_UPDATE: u32 = 1 << 24;
const IDLE_OUT_LV: u32 = 1 << 5;
const IDLE_OUT_EN: u32 = 1 << 6;
const CONF_MASK: u32 = 0x007f_ffff | (1 << 24);
const CONF_STROBES: u32 = TX_START | MEM_RD_RST | APB_MEM_RST | TX_STOP | CONF_UPDATE;
const INT_MASK: u32 = (1 << 30) - 1;
const SYS_CONF_MASK: u32 =
    (0xff << 4) | (0x3f << 12) | (0x3f << 18) | (0x3 << 24) | (1 << 26) | (1 << 31);
const TX_SIM_MASK: u32 = 0x1f;
const TX_SIM_EN: u32 = 1 << 4;
const DATE_MASK: u32 = 0x0fff_ffff;
const DATE_RESET: u32 = 0x0210_1181;
const CONF_RESET: u32 = (2 << 8) | (1 << 16) | CARRIER_EFF_EN | CARRIER_EN | CARRIER_OUT_LV;
const SYS_CONF_RESET: u32 = (1 << 4) | (1 << 24) | (1 << 26);
const FIFO_LIMIT: usize = 256;

/// A completed functional RMT transfer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Esp32s3RmtTransfer {
    /// Number of completed transfers on this channel.
    pub frames: u64,
    /// The most recently transmitted RMT item words.
    pub last_items: Vec<u32>,
}

/// Functional ESP32-S3 RMT transmitter slice.
pub struct Esp32s3Rmt {
    name: String,
    registers: [u32; 0x100 / 4],
    status: [u32; TX_CHANNELS],
    fifos: [Vec<u32>; TX_CHANNELS],
    transfers: [Esp32s3RmtTransfer; TX_CHANNELS],
    interrupt_raw: u32,
    interrupt_enable: u32,
    signals: SignalHub,
    outputs: [SignalId; TX_CHANNELS],
}

impl Esp32s3Rmt {
    /// Creates the ESP32-S3 transmitter channels and their waveform signals.
    pub fn new(name: impl Into<String>, signals: SignalHub) -> Result<Self, SignalError> {
        let mut output_vec = Vec::with_capacity(TX_CHANNELS);
        for channel in 0..TX_CHANNELS {
            output_vec.push(signals.declare(
                format!("board.esp32s3.rmt.ch{channel}"),
                SignalValue::from_u64(0, 1)?,
                Some(format!("ESP32-S3 RMT channel {channel} output")),
            )?);
        }
        let outputs = output_vec
            .try_into()
            .expect("RMT output declaration count is fixed");
        let mut rmt = Self {
            name: name.into(),
            registers: [0; 0x100 / 4],
            status: [0; TX_CHANNELS],
            fifos: std::array::from_fn(|_| Vec::new()),
            transfers: std::array::from_fn(|_| Esp32s3RmtTransfer::default()),
            interrupt_raw: 0,
            interrupt_enable: 0,
            signals,
            outputs,
        };
        rmt.reset_registers();
        Ok(rmt)
    }

    /// Returns the completed-transfer evidence for one transmitter channel.
    pub fn transfer(&self, channel: usize) -> Option<&Esp32s3RmtTransfer> {
        self.transfers.get(channel)
    }

    fn reset_registers(&mut self) {
        self.registers = [0; 0x100 / 4];
        for channel in 0..TX_CHANNELS {
            self.registers[(CONF0_BASE as usize / 4) + channel] = CONF_RESET;
            self.status[channel] = 0;
        }
        self.registers[SYS_CONF as usize / 4] = SYS_CONF_RESET;
        self.registers[TX_SIM as usize / 4] = 0;
        self.registers[DATE as usize / 4] = DATE_RESET;
        for channel in 0..TX_CHANNELS {
            self.refresh_status(channel);
        }
    }

    fn channel(offset: u64, base: u64, stride: u64) -> Option<usize> {
        if offset < base {
            return None;
        }
        let relative = offset - base;
        if relative % stride != 0 {
            return None;
        }
        let channel = usize::try_from(relative / stride).ok()?;
        (channel < TX_CHANNELS).then_some(channel)
    }

    fn register_index(offset: u64) -> Result<usize, DeviceError> {
        if offset & 3 != 0 || offset >= 0x100 {
            return Err(DeviceError::new(format!(
                "ESP32-S3 RMT offset {offset:#x} is not an aligned register"
            )));
        }
        Ok(usize::try_from(offset / 4).expect("RMT register offset fits usize"))
    }

    fn set_output(&self, channel: usize, level: bool, at: SimTime) -> Result<(), DeviceError> {
        self.signals
            .set(
                self.outputs[channel],
                SignalValue::from_u64(u64::from(level), 1).expect("one-bit RMT output is valid"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn refresh_status(&mut self, channel: usize) {
        // TX status exposes read-only memory addresses, FSM state, and the
        // empty/write-error summary. The functional model uses the number of
        // queued words as the deterministic APB write address and leaves the
        // transmitter FSM in its idle state.
        let mut value = self.status[channel] & (1 << 26);
        let count = u32::try_from(self.fifos[channel].len().min(0x3ff))
            .expect("bounded RMT FIFO count fits u32");
        value |= count << 11;
        if self.fifos[channel].is_empty() {
            value |= 1 << 25;
        }
        self.status[channel] = value;
    }

    fn execute_channel(&mut self, channel: usize, at: SimTime) -> Result<(), DeviceError> {
        let items = std::mem::take(&mut self.fifos[channel]);
        self.refresh_status(channel);
        if items.is_empty() {
            self.status[channel] |= 1 << 25;
            self.interrupt_raw |= 1 << (4 + channel);
            return Ok(());
        }

        let config = self.registers[(CONF0_BASE as usize / 4) + channel];
        let idle_level = config & IDLE_OUT_LV != 0;
        let mut cursor = at;
        for item in &items {
            let duration0 = u64::from(item & 0x7fff).max(1);
            let duration1 = u64::from((item >> 16) & 0x7fff).max(1);
            let level0 = item & (1 << 15) != 0;
            let level1 = item & (1 << 31) != 0;
            self.set_output(channel, level0, cursor)?;
            cursor = SimTime::from_ticks(cursor.ticks().saturating_add(duration0));
            self.set_output(channel, level1, cursor)?;
            cursor = SimTime::from_ticks(cursor.ticks().saturating_add(duration1));
        }
        if config & IDLE_OUT_EN != 0 {
            self.set_output(channel, idle_level, cursor)?;
        }

        self.transfers[channel].frames = self.transfers[channel].frames.saturating_add(1);
        self.transfers[channel].last_items = items;
        self.interrupt_raw |= 1 << channel;
        self.refresh_status(channel);
        Ok(())
    }

    fn reset_channel(&mut self, channel: usize) {
        self.fifos[channel].clear();
        self.status[channel] = 0;
        self.refresh_status(channel);
    }

    fn read_register(&self, offset: u64) -> Result<u32, DeviceError> {
        if let Some(channel) = Self::channel(offset, DATA_BASE, 4) {
            return Ok(self.fifos[channel].last().copied().unwrap_or(0));
        }
        if let Some(channel) = Self::channel(offset, CONF0_BASE, 4) {
            return Ok(self.registers[(CONF0_BASE as usize / 4) + channel]);
        }
        if let Some(channel) = Self::channel(offset, STATUS_BASE, 4) {
            return Ok(self.status[channel]);
        }
        match offset {
            INT_RAW => Ok(self.interrupt_raw),
            INT_ST => Ok(self.interrupt_raw & self.interrupt_enable),
            INT_ENA => Ok(self.interrupt_enable),
            SYS_CONF | TX_SIM => Ok(self.registers[offset as usize / 4]),
            DATE => Ok(self.registers[DATE as usize / 4]),
            _ => Err(DeviceError::new(format!(
                "unmodeled ESP32-S3 RMT read at offset {offset:#x}"
            ))),
        }
    }
}

impl Device for Esp32s3Rmt {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP32-S3 RMT requires word access"));
        }
        Ok(u64::from(self.read_register(offset)?))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP32-S3 RMT requires word access"));
        }
        let value = u32::try_from(value).map_err(|_| DeviceError::new("RMT value exceeds u32"))?;
        let _ = Self::register_index(offset)?;
        if let Some(channel) = Self::channel(offset, DATA_BASE, 4) {
            if self.fifos[channel].len() >= FIFO_LIMIT {
                self.status[channel] |= 1 << 26;
                self.interrupt_raw |= 1 << (4 + channel);
            } else {
                self.fifos[channel].push(value);
                self.refresh_status(channel);
            }
            return Ok(());
        }
        if let Some(channel) = Self::channel(offset, CONF0_BASE, 4) {
            self.registers[(CONF0_BASE as usize / 4) + channel] = value & CONF_MASK & !CONF_STROBES;
            if value & (MEM_RD_RST | APB_MEM_RST) != 0 {
                self.reset_channel(channel);
            }
            if value & TX_STOP != 0 {
                self.set_output(channel, value & IDLE_OUT_LV != 0, at)?;
            }
            if value & TX_START != 0 {
                self.execute_channel(channel, at)?;
            }
            return Ok(());
        }
        match offset {
            INT_ENA => self.interrupt_enable = value & INT_MASK,
            INT_CLR => self.interrupt_raw &= !(value & INT_MASK),
            INT_RAW => self.interrupt_raw = value & INT_MASK,
            SYS_CONF | TX_SIM => {
                let mask = if offset == SYS_CONF {
                    SYS_CONF_MASK
                } else {
                    TX_SIM_MASK
                };
                self.registers[offset as usize / 4] = value & mask;
                if offset == TX_SIM {
                    if value & TX_SIM_EN != 0 {
                        for channel in 0..TX_CHANNELS {
                            if value & (1 << channel) != 0 {
                                self.execute_channel(channel, at)?;
                            }
                        }
                    }
                }
            }
            STATUS_BASE..=0x5c => {
                return Err(DeviceError::new(
                    "ESP32-S3 RMT status registers are read-only",
                ));
            }
            DATE => self.registers[DATE as usize / 4] = value & DATE_MASK,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled ESP32-S3 RMT write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset_registers();
        self.fifos.iter_mut().for_each(Vec::clear);
        self.transfers = std::array::from_fn(|_| Esp32s3RmtTransfer::default());
        self.interrupt_raw = 0;
        self.interrupt_enable = 0;
        for channel in 0..TX_CHANNELS {
            self.refresh_status(channel);
            let _ = self.set_output(channel, false, SimTime::ZERO);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(duration0: u32, level0: bool, duration1: u32, level1: bool) -> u32 {
        duration0 | (u32::from(level0) << 15) | (duration1 << 16) | (u32::from(level1) << 31)
    }

    #[test]
    fn transmits_items_and_records_waveform_evidence() {
        let hub = SignalHub::new();
        let mut rmt = Esp32s3Rmt::new("esp32s3.rmt", hub.clone()).unwrap();
        rmt.write(
            0x00,
            AccessWidth::Word,
            item(2, true, 3, false) as u64,
            SimTime::ZERO,
        )
        .unwrap();
        rmt.write(
            0x00,
            AccessWidth::Word,
            item(1, false, 1, true) as u64,
            SimTime::ZERO,
        )
        .unwrap();
        rmt.write(
            0x20,
            AccessWidth::Word,
            TX_START as u64 | IDLE_OUT_EN as u64,
            SimTime::ZERO,
        )
        .unwrap();

        let transfer = rmt.transfer(0).unwrap();
        assert_eq!(transfer.frames, 1);
        assert_eq!(transfer.last_items.len(), 2);
        assert_eq!(
            rmt.read(INT_RAW, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1
        );
        let changes = hub.drain_changes();
        assert!(
            changes
                .iter()
                .any(|change| change.at == SimTime::from_ticks(2))
        );
        assert!(
            changes
                .iter()
                .any(|change| change.at == SimTime::from_ticks(6))
        );
    }

    #[test]
    fn interrupt_enable_and_empty_fifo_error_are_deterministic() {
        let hub = SignalHub::new();
        let mut rmt = Esp32s3Rmt::new("esp32s3.rmt", hub).unwrap();
        rmt.write(INT_ENA, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        rmt.write(0x20, AccessWidth::Word, TX_START as u64, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rmt.read(INT_RAW, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 4
        );
        assert_eq!(
            rmt.read(INT_ST, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0
        );
        rmt.write(INT_CLR, AccessWidth::Word, 1 << 4, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rmt.read(INT_RAW, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0
        );
    }

    #[test]
    fn register_ids_masks_reset_values_and_write_semantics_match_native_contract() {
        for register in [
            Esp32s3RmtRegister::Ch0Data,
            Esp32s3RmtRegister::Ch1Data,
            Esp32s3RmtRegister::Ch2Data,
            Esp32s3RmtRegister::Ch3Data,
            Esp32s3RmtRegister::Ch0Conf0,
            Esp32s3RmtRegister::Ch1Conf0,
            Esp32s3RmtRegister::Ch2Conf0,
            Esp32s3RmtRegister::Ch3Conf0,
            Esp32s3RmtRegister::Ch0Status,
            Esp32s3RmtRegister::Ch1Status,
            Esp32s3RmtRegister::Ch2Status,
            Esp32s3RmtRegister::Ch3Status,
            Esp32s3RmtRegister::IntRaw,
            Esp32s3RmtRegister::IntSt,
            Esp32s3RmtRegister::IntEna,
            Esp32s3RmtRegister::IntClr,
            Esp32s3RmtRegister::SysConf,
            Esp32s3RmtRegister::TxSim,
            Esp32s3RmtRegister::Date,
        ] {
            assert_eq!(
                Esp32s3RmtRegister::from_offset(register.offset()),
                Some(register)
            );
        }

        let hub = SignalHub::new();
        let mut rmt = Esp32s3Rmt::new("esp32s3.rmt", hub).unwrap();
        assert_eq!(
            rmt.read(
                Esp32s3RmtRegister::Ch0Conf0.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            u64::from(CONF_RESET)
        );
        assert_eq!(
            rmt.read(
                Esp32s3RmtRegister::Ch0Status.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            u64::from(1_u32 << 25)
        );
        assert_eq!(
            rmt.read(SYS_CONF, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(SYS_CONF_RESET)
        );
        assert_eq!(
            rmt.read(DATE, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(DATE_RESET)
        );

        let config = u32::MAX & !CONF_STROBES;
        rmt.write(
            Esp32s3RmtRegister::Ch0Conf0.offset(),
            AccessWidth::Word,
            u64::from(config),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            rmt.read(
                Esp32s3RmtRegister::Ch0Conf0.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            u64::from(config & CONF_MASK)
        );
        rmt.write(
            SYS_CONF,
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            rmt.read(SYS_CONF, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(SYS_CONF_MASK)
        );
        rmt.write(
            TX_SIM,
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            rmt.read(TX_SIM, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(TX_SIM_MASK)
        );

        rmt.write(
            INT_RAW,
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            rmt.read(INT_RAW, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(INT_MASK)
        );
        rmt.write(
            INT_ENA,
            AccessWidth::Word,
            u64::from(u32::MAX),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            rmt.read(INT_ST, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(INT_MASK)
        );
        rmt.write(INT_CLR, AccessWidth::Word, u64::from(1_u32), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rmt.read(INT_RAW, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(INT_MASK & !1)
        );

        rmt.write(DATE, AccessWidth::Word, u64::from(u32::MAX), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rmt.read(DATE, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(DATE_MASK)
        );
        assert!(
            rmt.read(
                Esp32s3RmtRegister::Ch0Data.offset() + 1,
                AccessWidth::Word,
                SimTime::ZERO
            )
            .is_err()
        );
        assert!(
            rmt.write(
                Esp32s3RmtRegister::Ch0Data.offset() + 1,
                AccessWidth::Word,
                0,
                SimTime::ZERO,
            )
            .is_err()
        );
        assert!(
            rmt.write(
                Esp32s3RmtRegister::Ch0Status.offset(),
                AccessWidth::Word,
                0,
                SimTime::ZERO
            )
            .is_err()
        );
    }
}
