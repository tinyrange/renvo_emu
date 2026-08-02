use super::*;

const RADIO_CONTROL: u64 = 0x00;
const RADIO_STATUS: u64 = 0x04;
const RADIO_CHANNEL: u64 = 0x08;
const RADIO_TX_LENGTH: u64 = 0x0c;
const RADIO_RX_LENGTH: u64 = 0x10;
const RADIO_INTERRUPT_ENABLE: u64 = 0x14;
const RADIO_INTERRUPT_RAW: u64 = 0x18;
const RADIO_INTERRUPT_STATUS: u64 = 0x1c;
const RADIO_INTERRUPT_CLEAR: u64 = 0x20;
const RADIO_TX_COMMIT: u64 = 0x24;
const RADIO_ID: u64 = 0x28;
const RADIO_TX_PACKETS: u64 = 0x2c;
const RADIO_RX_PACKETS: u64 = 0x30;
const RADIO_TX_DATA: u64 = 0x100;
const RADIO_RX_DATA: u64 = 0x200;
const RADIO_CONTROL_ENABLE: u32 = 1 << 0;
const RADIO_CONTROL_LOOPBACK: u32 = 1 << 1;
const RADIO_CONTROL_RESET: u32 = 1 << 2;
const RADIO_STATUS_READY: u32 = 1 << 0;
const RADIO_STATUS_TX_PENDING: u32 = 1 << 1;
const RADIO_STATUS_RX_PENDING: u32 = 1 << 2;
const RADIO_INTERRUPT_PACKET: u32 = 1 << 0;
const RADIO_INTERRUPT_ERROR: u32 = 1 << 1;
const RADIO_FIFO_CAPACITY: usize = 4096;

/// Identifies the deterministic radio endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EspRadioKind {
    /// Wi-Fi control/loopback endpoint.
    Wifi,
    /// Bluetooth LE control/loopback endpoint.
    BluetoothLe,
}

struct EspRadioState {
    registers: Vec<u32>,
    tx_fifo: VecDeque<u8>,
    rx_fifo: VecDeque<u8>,
    kind: EspRadioKind,
    tx_packets: u64,
    rx_packets: u64,
}

impl EspRadioState {
    fn new(kind: EspRadioKind) -> Self {
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            tx_fifo: VecDeque::new(),
            rx_fifo: VecDeque::new(),
            kind,
            tx_packets: 0,
            rx_packets: 0,
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.tx_fifo.clear();
        self.rx_fifo.clear();
        self.tx_packets = 0;
        self.rx_packets = 0;
        self.registers[(RADIO_CHANNEL / 4) as usize] = 1;
        self.registers[(RADIO_ID / 4) as usize] = match self.kind {
            EspRadioKind::Wifi => u32::from_le_bytes(*b"WIFI"),
            EspRadioKind::BluetoothLe => u32::from_le_bytes(*b"BTLE"),
        };
    }

    fn enabled(&self) -> bool {
        self.registers[(RADIO_CONTROL / 4) as usize] & RADIO_CONTROL_ENABLE != 0
    }

    fn loopback(&self) -> bool {
        self.registers[(RADIO_CONTROL / 4) as usize] & RADIO_CONTROL_LOOPBACK != 0
    }

    fn status(&self) -> u32 {
        let mut status = 0;
        if self.enabled() {
            status |= RADIO_STATUS_READY;
        }
        if !self.tx_fifo.is_empty() {
            status |= RADIO_STATUS_TX_PENDING;
        }
        if !self.rx_fifo.is_empty() {
            status |= RADIO_STATUS_RX_PENDING;
        }
        status
    }

    fn interrupt_status(&self) -> u32 {
        self.registers[(RADIO_INTERRUPT_RAW / 4) as usize]
            & self.registers[(RADIO_INTERRUPT_ENABLE / 4) as usize]
    }

    fn commit(&mut self) {
        let requested = usize::try_from(self.registers[(RADIO_TX_LENGTH / 4) as usize])
            .unwrap_or(0)
            .min(self.tx_fifo.len());
        if !self.enabled() || !self.loopback() {
            self.registers[(RADIO_INTERRUPT_RAW / 4) as usize] |= RADIO_INTERRUPT_ERROR;
            self.tx_fifo.clear();
            return;
        }
        let length = if requested == 0 {
            self.tx_fifo.len()
        } else {
            requested
        };
        for _ in 0..length {
            if let Some(byte) = self.tx_fifo.pop_front() {
                if self.rx_fifo.len() < RADIO_FIFO_CAPACITY {
                    self.rx_fifo.push_back(byte);
                }
            }
        }
        self.tx_packets = self.tx_packets.saturating_add(1);
        self.rx_packets = self.rx_packets.saturating_add(1);
        self.registers[(RADIO_INTERRUPT_RAW / 4) as usize] |= RADIO_INTERRUPT_PACKET;
    }

    fn push_tx_word(&mut self, value: u32) {
        for byte in value.to_le_bytes() {
            if self.tx_fifo.len() < RADIO_FIFO_CAPACITY {
                self.tx_fifo.push_back(byte);
            }
        }
    }

    fn pop_rx_word(&mut self) -> u32 {
        let mut bytes = [0_u8; 4];
        for byte in &mut bytes {
            *byte = self.rx_fifo.pop_front().unwrap_or(0);
        }
        u32::from_le_bytes(bytes)
    }
}

/// Deterministic ESP32-S3 Wi-Fi/Bluetooth control and packet-loopback slice.
///
/// This gives headless firmware tests a bounded packet endpoint at the native
/// radio pages without pretending to model an RF PHY, association, channel
/// timing, coexistence, or network security.
pub struct EspRadio {
    name: String,
    state: EspRadioState,
}

impl EspRadio {
    /// Creates an idle radio endpoint of the requested kind.
    pub fn new(name: impl Into<String>, kind: EspRadioKind) -> Self {
        Self {
            name: name.into(),
            state: EspRadioState::new(kind),
        }
    }
}

impl Device for EspRadio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP radio requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("radio offset fits");
        if index >= self.state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} read at {offset:#x}",
                self.name
            )));
        }
        let value = match offset {
            RADIO_STATUS => self.state.status(),
            RADIO_RX_LENGTH => self.state.rx_fifo.len() as u32,
            RADIO_INTERRUPT_STATUS => self.state.interrupt_status(),
            RADIO_RX_DATA => self.state.pop_rx_word(),
            RADIO_TX_PACKETS => self.state.tx_packets as u32,
            RADIO_RX_PACKETS => self.state.rx_packets as u32,
            _ => self.state.registers[index],
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP radio requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("radio offset fits");
        if index >= self.state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = value as u32;
        match offset {
            RADIO_CONTROL => {
                self.state.registers[index] = value & 0x7;
                if value & RADIO_CONTROL_RESET != 0 {
                    self.state.reset();
                }
            }
            RADIO_CHANNEL => self.state.registers[index] = value & 0x3f,
            RADIO_TX_LENGTH => self.state.registers[index] = value & 0xfff,
            RADIO_INTERRUPT_ENABLE => self.state.registers[index] = value & 0x3,
            RADIO_INTERRUPT_CLEAR => {
                self.state.registers[(RADIO_INTERRUPT_RAW / 4) as usize] &= !(value & 0x3);
                self.state.registers[index] = 0;
            }
            RADIO_TX_DATA => self.state.push_tx_word(value),
            RADIO_TX_COMMIT => self.state.commit(),
            RADIO_STATUS
            | RADIO_RX_LENGTH
            | RADIO_INTERRUPT_STATUS
            | RADIO_RX_DATA
            | RADIO_ID
            | RADIO_TX_PACKETS
            | RADIO_RX_PACKETS => {}
            _ => self.state.registers[index] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_loopback_exposes_packet_status_and_clearable_interrupt() {
        let mut device = EspRadio::new("wifi", EspRadioKind::Wifi);
        device
            .write(RADIO_CONTROL, AccessWidth::Word, 0b11, SimTime::ZERO)
            .unwrap();
        device
            .write(RADIO_CHANNEL, AccessWidth::Word, 6, SimTime::ZERO)
            .unwrap();
        device
            .write(RADIO_INTERRUPT_ENABLE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(RADIO_TX_LENGTH, AccessWidth::Word, 8, SimTime::ZERO)
            .unwrap();
        device
            .write(RADIO_TX_DATA, AccessWidth::Word, 0x4433_2211, SimTime::ZERO)
            .unwrap();
        device
            .write(RADIO_TX_DATA, AccessWidth::Word, 0x8877_6655, SimTime::ZERO)
            .unwrap();
        device
            .write(RADIO_TX_COMMIT, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(RADIO_STATUS, AccessWidth::Word, SimTime::ZERO),
            Ok(u64::from(RADIO_STATUS_READY | RADIO_STATUS_RX_PENDING))
        );
        assert_eq!(
            device.read(RADIO_INTERRUPT_STATUS, AccessWidth::Word, SimTime::ZERO),
            Ok(1)
        );
        assert_eq!(
            device.read(RADIO_RX_DATA, AccessWidth::Word, SimTime::ZERO),
            Ok(0x4433_2211)
        );
        assert_eq!(
            device.read(RADIO_RX_DATA, AccessWidth::Word, SimTime::ZERO),
            Ok(0x8877_6655)
        );
        device
            .write(RADIO_INTERRUPT_CLEAR, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(RADIO_INTERRUPT_STATUS, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
    }

    #[test]
    fn disabled_radio_reports_a_transmit_error() {
        let mut device = EspRadio::new("btle", EspRadioKind::BluetoothLe);
        device
            .write(RADIO_TX_DATA, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(RADIO_TX_COMMIT, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(RADIO_INTERRUPT_RAW, AccessWidth::Word, SimTime::ZERO),
            Ok(u64::from(RADIO_INTERRUPT_ERROR))
        );
    }
}
