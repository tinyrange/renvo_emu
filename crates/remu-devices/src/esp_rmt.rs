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
const DATA_BASE: u64 = 0x00;
const CONF0_BASE: u64 = 0x20;
const STATUS_BASE: u64 = 0x50;
const INT_RAW: u64 = 0x70;
const INT_ST: u64 = 0x74;
const INT_ENA: u64 = 0x78;
const INT_CLR: u64 = 0x7c;
const SYS_CONF: u64 = 0xc0;
const TX_SIM: u64 = 0xc4;
const DATE: u64 = 0xcc;
const TX_START: u32 = 1 << 0;
const MEM_RD_RST: u32 = 1 << 1;
const APB_MEM_RST: u32 = 1 << 2;
const TX_STOP: u32 = 1 << 7;
const IDLE_OUT_LV: u32 = 1 << 5;
const IDLE_OUT_EN: u32 = 1 << 6;
const DATE_RESET: u32 = 34_607_489;
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
        Ok(Self {
            name: name.into(),
            registers: [0; 0x100 / 4],
            status: [0; TX_CHANNELS],
            fifos: std::array::from_fn(|_| Vec::new()),
            transfers: std::array::from_fn(|_| Esp32s3RmtTransfer::default()),
            interrupt_raw: 0,
            interrupt_enable: 0,
            signals,
            outputs,
        })
    }

    /// Returns the completed-transfer evidence for one transmitter channel.
    pub fn transfer(&self, channel: usize) -> Option<&Esp32s3RmtTransfer> {
        self.transfers.get(channel)
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
        let mut value = self.status[channel] & ((1 << 26) | (1 << 27));
        let count = u32::try_from(self.fifos[channel].len().min(0x3ff))
            .expect("bounded RMT FIFO count fits u32");
        value |= count;
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
            DATE => Ok(DATE_RESET),
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
        if width != AccessWidth::Word {
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("ESP32-S3 RMT requires word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("RMT value is 32-bit");
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
            self.registers[(CONF0_BASE as usize / 4) + channel] =
                value & !(TX_START | TX_STOP | MEM_RD_RST | APB_MEM_RST);
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
            INT_ENA => self.interrupt_enable = value,
            INT_CLR => self.interrupt_raw &= !value,
            INT_RAW => self.interrupt_raw &= !value,
            SYS_CONF | TX_SIM => {
                self.registers[offset as usize / 4] = value;
                if offset == TX_SIM {
                    for channel in 0..TX_CHANNELS {
                        if value & (1 << channel) != 0 {
                            self.execute_channel(channel, at)?;
                        }
                    }
                }
            }
            STATUS_BASE..=0x5c => {
                return Err(DeviceError::new(
                    "ESP32-S3 RMT status registers are read-only",
                ));
            }
            DATE => return Err(DeviceError::new("ESP32-S3 RMT DATE is read-only")),
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled ESP32-S3 RMT write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers = [0; 0x100 / 4];
        self.status = [0; TX_CHANNELS];
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
}
