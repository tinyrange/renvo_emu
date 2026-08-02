use super::*;

const CONTROL: u64 = 0x00;
const STATUS: u64 = 0x04;
const CHANNEL: u64 = 0x08;
const TX_LENGTH: u64 = 0x0c;
const RX_LENGTH: u64 = 0x10;
const INTERRUPT_ENABLE: u64 = 0x14;
const INTERRUPT_RAW: u64 = 0x18;
const INTERRUPT_STATUS: u64 = 0x1c;
const INTERRUPT_CLEAR: u64 = 0x20;
const TX_COMMIT: u64 = 0x24;
const TX_FRAMES: u64 = 0x28;
const RX_FRAMES: u64 = 0x2c;
const TX_DATA: u64 = 0x100;
const RX_DATA: u64 = 0x200;

const CONTROL_ENABLE: u32 = 1 << 0;
const CONTROL_LOOPBACK: u32 = 1 << 1;
const CONTROL_RESET: u32 = 1 << 2;
const STATUS_READY: u32 = 1 << 0;
const STATUS_TX_PENDING: u32 = 1 << 1;
const STATUS_RX_PENDING: u32 = 1 << 2;
const INTERRUPT_FRAME: u32 = 1 << 0;
const INTERRUPT_ERROR: u32 = 1 << 1;
const FIFO_CAPACITY: usize = 2048;

/// Functional ESP32-C6 IEEE 802.15.4 frame endpoint.
///
/// The model deliberately stops at deterministic MAC-side frame behavior:
/// firmware can enable the controller, select a channel, submit a bounded
/// frame, and receive it through a local loopback. It does not model RF,
/// carrier sense, association, encryption, coexistence, or timing. This is
/// enough for compiler and driver smoke tests without implying a PHY model.
pub struct EspC6Ieee802154 {
    name: String,
    registers: Vec<u32>,
    tx_fifo: VecDeque<u8>,
    rx_fifo: VecDeque<u8>,
    tx_frames: u32,
    rx_frames: u32,
}

impl EspC6Ieee802154 {
    /// Creates an idle controller with the reset channel selected.
    pub fn new(name: impl Into<String>) -> Self {
        let mut device = Self {
            name: name.into(),
            registers: vec![0; 0x1000 / 4],
            tx_fifo: VecDeque::new(),
            rx_fifo: VecDeque::new(),
            tx_frames: 0,
            rx_frames: 0,
        };
        device.reset_state();
        device
    }

    fn reset_state(&mut self) {
        self.registers.fill(0);
        self.tx_fifo.clear();
        self.rx_fifo.clear();
        self.tx_frames = 0;
        self.rx_frames = 0;
        self.registers[(CHANNEL / 4) as usize] = 11;
    }

    fn enabled(&self) -> bool {
        self.registers[(CONTROL / 4) as usize] & CONTROL_ENABLE != 0
    }

    fn loopback(&self) -> bool {
        self.registers[(CONTROL / 4) as usize] & CONTROL_LOOPBACK != 0
    }

    fn status(&self) -> u32 {
        let mut status = 0;
        if self.enabled() {
            status |= STATUS_READY;
        }
        if !self.tx_fifo.is_empty() {
            status |= STATUS_TX_PENDING;
        }
        if !self.rx_fifo.is_empty() {
            status |= STATUS_RX_PENDING;
        }
        status
    }

    fn interrupt_status(&self) -> u32 {
        self.registers[(INTERRUPT_RAW / 4) as usize]
            & self.registers[(INTERRUPT_ENABLE / 4) as usize]
    }

    fn commit(&mut self) {
        let requested = usize::try_from(self.registers[(TX_LENGTH / 4) as usize])
            .unwrap_or(0)
            .min(self.tx_fifo.len());
        if !self.enabled() || !self.loopback() || requested == 0 {
            self.registers[(INTERRUPT_RAW / 4) as usize] |= INTERRUPT_ERROR;
            self.tx_fifo.clear();
            return;
        }
        for _ in 0..requested {
            if let Some(byte) = self.tx_fifo.pop_front()
                && self.rx_fifo.len() < FIFO_CAPACITY
            {
                self.rx_fifo.push_back(byte);
            }
        }
        self.tx_frames = self.tx_frames.saturating_add(1);
        self.rx_frames = self.rx_frames.saturating_add(1);
        self.registers[(INTERRUPT_RAW / 4) as usize] |= INTERRUPT_FRAME;
    }

    fn push_tx_word(&mut self, value: u32) {
        for byte in value.to_le_bytes() {
            if self.tx_fifo.len() < FIFO_CAPACITY {
                self.tx_fifo.push_back(byte);
            }
        }
    }

    fn pop_rx_word(&mut self) -> u32 {
        let mut bytes = [0_u8; 4];
        for byte in &mut bytes {
            *byte = self.rx_fifo.pop_front().unwrap_or_default();
        }
        u32::from_le_bytes(bytes)
    }

    fn index(offset: u64) -> Result<usize, DeviceError> {
        if offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-C6 IEEE 802.15.4 requires aligned access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("radio offset fits usize");
        (index < 0x1000 / 4)
            .then_some(index)
            .ok_or_else(|| DeviceError::new(format!("IEEE 802.15.4 access at {offset:#x}")))
    }
}

impl Device for EspC6Ieee802154 {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new(
                "ESP32-C6 IEEE 802.15.4 requires word access",
            ));
        }
        let index = Self::index(offset)?;
        let value = match offset {
            STATUS => self.status(),
            RX_LENGTH => self.rx_fifo.len() as u32,
            INTERRUPT_STATUS => self.interrupt_status(),
            RX_DATA => self.pop_rx_word(),
            TX_FRAMES => self.tx_frames,
            RX_FRAMES => self.rx_frames,
            _ => self.registers[index],
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
            return Err(DeviceError::new(
                "ESP32-C6 IEEE 802.15.4 requires word access",
            ));
        }
        let index = Self::index(offset)?;
        let value = value as u32;
        match offset {
            CONTROL => {
                self.registers[index] = value & 0x7;
                if value & CONTROL_RESET != 0 {
                    self.reset_state();
                }
            }
            CHANNEL => self.registers[index] = value & 0x1f,
            TX_LENGTH => self.registers[index] = value & 0x7ff,
            INTERRUPT_ENABLE => self.registers[index] = value & 0x3,
            INTERRUPT_CLEAR => {
                self.registers[(INTERRUPT_RAW / 4) as usize] &= !(value & 0x3);
                self.registers[index] = 0;
            }
            TX_DATA => self.push_tx_word(value),
            TX_COMMIT => self.commit(),
            STATUS | RX_LENGTH | INTERRUPT_STATUS | RX_DATA | TX_FRAMES | RX_FRAMES => {}
            _ => self.registers[index] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset_state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_loopback_returns_a_frame_and_latches_interrupt() {
        let mut radio = EspC6Ieee802154::new("ieee802154");
        radio
            .write(CONTROL, AccessWidth::Word, 0b11, SimTime::ZERO)
            .unwrap();
        radio
            .write(CHANNEL, AccessWidth::Word, 20, SimTime::ZERO)
            .unwrap();
        radio
            .write(
                INTERRUPT_ENABLE,
                AccessWidth::Word,
                INTERRUPT_FRAME as u64,
                SimTime::ZERO,
            )
            .unwrap();
        radio
            .write(TX_LENGTH, AccessWidth::Word, 8, SimTime::ZERO)
            .unwrap();
        radio
            .write(TX_DATA, AccessWidth::Word, 0x4433_2211, SimTime::ZERO)
            .unwrap();
        radio
            .write(TX_DATA, AccessWidth::Word, 0x8877_6655, SimTime::ZERO)
            .unwrap();
        radio
            .write(TX_COMMIT, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();

        assert_eq!(radio.read(STATUS, AccessWidth::Word, SimTime::ZERO), Ok(5));
        assert_eq!(
            radio.read(INTERRUPT_STATUS, AccessWidth::Word, SimTime::ZERO),
            Ok(1)
        );
        assert_eq!(
            radio.read(RX_DATA, AccessWidth::Word, SimTime::ZERO),
            Ok(0x4433_2211)
        );
        assert_eq!(
            radio.read(RX_DATA, AccessWidth::Word, SimTime::ZERO),
            Ok(0x8877_6655)
        );
        radio
            .write(INTERRUPT_CLEAR, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            radio.read(INTERRUPT_STATUS, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
    }

    #[test]
    fn disabled_submission_reports_a_deterministic_error() {
        let mut radio = EspC6Ieee802154::new("ieee802154");
        radio
            .write(TX_DATA, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        radio
            .write(TX_COMMIT, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            radio.read(INTERRUPT_RAW, AccessWidth::Word, SimTime::ZERO),
            Ok(u64::from(INTERRUPT_ERROR))
        );
    }
}
