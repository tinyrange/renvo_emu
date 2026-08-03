use super::*;

/// Host-facing state for the ESP32-C6 low-power I2C controller.
#[derive(Clone)]
pub struct EspLpI2cHandle {
    state: Rc<RefCell<EspLpI2cState>>,
}

impl EspLpI2cHandle {
    /// Queues bytes returned by a subsequent native FIFO read command.
    pub fn queue_rx(&self, bytes: &[u8]) {
        self.state
            .borrow_mut()
            .rx_host
            .extend(bytes.iter().copied());
    }

    /// Returns and clears bytes consumed by a native FIFO write command.
    pub fn take_tx(&self) -> Vec<u8> {
        std::mem::take(&mut self.state.borrow_mut().tx_host)
    }

    /// Reports whether a START command is still active.
    pub fn busy(&self) -> bool {
        self.state.borrow().busy
    }
}

struct EspLpI2cState {
    registers: Vec<u32>,
    tx_fifo: VecDeque<u8>,
    rx_fifo: VecDeque<u8>,
    rx_host: VecDeque<u8>,
    tx_host: Vec<u8>,
    busy: bool,
    rx_read_addr: u8,
    rx_write_addr: u8,
    tx_read_addr: u8,
    tx_write_addr: u8,
}

impl EspLpI2cState {
    const FIFO_DEPTH: usize = 16;
    const REGISTER_BYTES: usize = 0x200;
    const CTR: usize = 0x04;
    const SCL_LOW_PERIOD: usize = 0x00;
    const STATUS: usize = 0x08;
    const TIMEOUT: usize = 0x0c;
    const FIFO_STATUS: usize = 0x14;
    const FIFO_CONFIG: usize = 0x18;
    const FIFO_DATA: usize = 0x1c;
    const INT_RAW: usize = 0x20;
    const INT_CLEAR: usize = 0x24;
    const INT_ENABLE: usize = 0x28;
    const INT_STATUS: usize = 0x2c;
    const SDA_HOLD: usize = 0x30;
    const SDA_SAMPLE: usize = 0x34;
    const SCL_HIGH_PERIOD: usize = 0x38;
    const SCL_START_HOLD: usize = 0x40;
    const SCL_RSTART_SETUP: usize = 0x44;
    const SCL_STOP_HOLD: usize = 0x48;
    const SCL_STOP_SETUP: usize = 0x4c;
    const FILTER_CONFIG: usize = 0x50;
    const CLOCK_CONFIG: usize = 0x54;
    const COMMAND_BASE: usize = 0x58;
    const COMMAND_COUNT: usize = 8;
    const SCL_STATE_TIMEOUT: usize = 0x78;
    const SCL_MAIN_STATE_TIMEOUT: usize = 0x7c;
    const SCL_SPECIAL_CONFIG: usize = 0x80;
    const DATE: usize = 0xf8;
    const TX_FIFO_START: usize = 0x100;
    const RX_FIFO_START: usize = 0x180;

    const CTR_RW_MASK: u32 = 0x0000_03cf;
    const CTR_TRANS_START: u32 = 1 << 5;
    const CTR_FSM_RESET: u32 = 1 << 10;
    const CTR_RX_FULL_ACK_LEVEL: u32 = 1 << 3;
    const CTR_ARBITRATION_ENABLE: u32 = 1 << 9;
    const FIFO_CONFIG_MASK: u32 =
        (0x0f) | (0x0f << 5) | (1 << 10) | (1 << 12) | (1 << 13) | (1 << 14);
    const INTERRUPT_MASK: u32 = 0xffff;
    const INT_TRANS_START: u32 = 1 << 9;
    const INT_TRANS_COMPLETE: u32 = 1 << 7;
    const INT_END_DETECT: u32 = 1 << 3;
    const INT_RX_WATERMARK: u32 = 1;
    const INT_TX_WATERMARK: u32 = 1 << 1;
    const INT_TX_FIFO_UNDERFLOW: u32 = 1 << 6;
    const INT_TX_FIFO_OVERFLOW: u32 = 1 << 11;
    const INT_RX_FIFO_OVERFLOW: u32 = 1 << 2;
    const INT_RX_FIFO_UNDERFLOW: u32 = 1 << 12;
    const CMD_RESTART: u32 = 6;
    const CMD_WRITE: u32 = 1;
    const CMD_READ: u32 = 3;
    const CMD_STOP: u32 = 2;
    const CMD_END: u32 = 4;

    fn new() -> Self {
        let mut registers = vec![0; Self::REGISTER_BYTES / 4];
        // These reset values are taken from the ESP32-C6 LP-I2C register
        // description. The functional model does not claim electrical timing
        // fidelity, but preserving reset-visible configuration makes SDK
        // startup code deterministic.
        registers[Self::CTR / 4] = Self::CTR_RX_FULL_ACK_LEVEL | Self::CTR_ARBITRATION_ENABLE;
        registers[Self::TIMEOUT / 4] = 16;
        registers[Self::FIFO_CONFIG / 4] = (6 << 0) | (2 << 5) | (1 << 14);
        registers[Self::SCL_START_HOLD / 4] = 8;
        registers[Self::SCL_RSTART_SETUP / 4] = 8;
        registers[Self::SCL_STOP_HOLD / 4] = 8;
        registers[Self::SCL_STOP_SETUP / 4] = 8;
        registers[Self::FILTER_CONFIG / 4] = (1 << 8) | (1 << 9);
        registers[Self::CLOCK_CONFIG / 4] = 1 << 21;
        registers[Self::SCL_STATE_TIMEOUT / 4] = 16;
        registers[Self::SCL_MAIN_STATE_TIMEOUT / 4] = 16;
        registers[Self::DATE / 4] = 35_656_003;
        registers[Self::INT_RAW / 4] = Self::INT_TX_WATERMARK;
        Self {
            registers,
            tx_fifo: VecDeque::new(),
            rx_fifo: VecDeque::new(),
            rx_host: VecDeque::new(),
            tx_host: Vec::new(),
            busy: false,
            rx_read_addr: 0,
            rx_write_addr: 0,
            tx_read_addr: 0,
            tx_write_addr: 0,
        }
    }

    fn status(&self) -> u32 {
        let mut value = u32::from(self.busy) << 4;
        value |= u32::try_from(self.rx_fifo.len().min(0x1f)).unwrap_or(0) << 8;
        value |= u32::try_from(self.tx_fifo.len().min(0x1f)).unwrap_or(0) << 18;
        value
    }

    fn fifo_status(&self) -> u32 {
        u32::from(self.rx_read_addr & 0x0f)
            | (u32::from(self.rx_write_addr & 0x0f) << 5)
            | (u32::from(self.tx_read_addr & 0x0f) << 10)
            | (u32::from(self.tx_write_addr & 0x0f) << 15)
    }

    fn interrupt_status(&self) -> u32 {
        self.registers[Self::INT_RAW / 4] & self.registers[Self::INT_ENABLE / 4]
    }

    fn command_value(&self, index: usize) -> u32 {
        self.registers[Self::COMMAND_BASE / 4 + index]
    }

    fn push_tx(&mut self, byte: u8) -> bool {
        if self.tx_fifo.len() >= Self::FIFO_DEPTH {
            return false;
        }
        self.tx_fifo.push_back(byte);
        self.tx_write_addr = (self.tx_write_addr + 1) & 0x0f;
        true
    }

    fn pop_tx(&mut self) -> Option<u8> {
        let byte = self.tx_fifo.pop_front()?;
        self.tx_read_addr = (self.tx_read_addr + 1) & 0x0f;
        Some(byte)
    }

    fn push_rx(&mut self, byte: u8) -> bool {
        if self.rx_fifo.len() >= Self::FIFO_DEPTH {
            return false;
        }
        self.rx_fifo.push_back(byte);
        self.rx_write_addr = (self.rx_write_addr + 1) & 0x0f;
        true
    }

    fn pop_rx(&mut self) -> Option<u8> {
        let byte = self.rx_fifo.pop_front()?;
        self.rx_read_addr = (self.rx_read_addr + 1) & 0x0f;
        Some(byte)
    }

    fn execute_commands(&mut self) {
        self.busy = true;
        self.registers[Self::INT_RAW / 4] |= Self::INT_TRANS_START;
        for index in 0..Self::COMMAND_COUNT {
            let command = self.command_value(index);
            let byte_count = usize::try_from(command & 0xff).unwrap_or(0);
            let opcode = (command >> 11) & 0x7;
            let mut finish = false;
            match opcode {
                Self::CMD_WRITE => {
                    for _ in 0..byte_count {
                        if let Some(byte) = self.pop_tx() {
                            self.tx_host.push(byte);
                        } else {
                            self.registers[Self::INT_RAW / 4] |= Self::INT_TX_FIFO_UNDERFLOW;
                            break;
                        }
                    }
                }
                Self::CMD_READ => {
                    for _ in 0..byte_count {
                        let byte = self.rx_host.pop_front().unwrap_or_default();
                        if !self.push_rx(byte) {
                            self.registers[Self::INT_RAW / 4] |= Self::INT_RX_FIFO_OVERFLOW;
                            break;
                        }
                    }
                }
                Self::CMD_STOP => {
                    self.registers[Self::INT_RAW / 4] |= Self::INT_TRANS_COMPLETE;
                    finish = true;
                }
                Self::CMD_END => {
                    self.registers[Self::INT_RAW / 4] |= Self::INT_END_DETECT;
                    finish = true;
                }
                Self::CMD_RESTART => {}
                _ => break,
            }
            self.registers[Self::COMMAND_BASE / 4 + index] = command | (1 << 31);
            if finish {
                break;
            }
        }
        self.busy = false;
        self.refresh_watermarks();
    }

    fn refresh_watermarks(&mut self) {
        let config = self.registers[Self::FIFO_CONFIG / 4];
        if config & (1 << 14) == 0 {
            return;
        }
        let rx_threshold = usize::try_from(config & 0x0f).unwrap_or(0);
        let tx_threshold = usize::try_from((config >> 5) & 0x0f).unwrap_or(0);
        if self.rx_fifo.len() > rx_threshold {
            self.registers[Self::INT_RAW / 4] |= Self::INT_RX_WATERMARK;
        }
        if self.tx_fifo.len() < tx_threshold {
            self.registers[Self::INT_RAW / 4] |= Self::INT_TX_WATERMARK;
        }
    }
}

/// Functional ESP32-C6 LP-I2C master register and FIFO slice.
///
/// Native control, status, FIFO, command-link, interrupt and reset-visible
/// version registers are retained. A START write executes the programmed
/// command list immediately on the abstract simulation timeline: WRITE drains
/// the TX FIFO into the host transcript, READ fills the RX FIFO from queued
/// host bytes, and STOP/END completes the transfer. Bus arbitration, clock
/// stretching, GPIO pad ownership and LP-domain power sequencing are outside
/// this functional baseline.
pub struct EspLpI2c {
    name: String,
    state: Rc<RefCell<EspLpI2cState>>,
}

impl EspLpI2c {
    /// Creates a reset LP-I2C block and its deterministic host handle.
    pub fn new(name: impl Into<String>) -> (Self, EspLpI2cHandle) {
        let state = Rc::new(RefCell::new(EspLpI2cState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspLpI2cHandle { state },
        )
    }
}

impl Device for EspLpI2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP LP-I2C requires aligned word access"));
        }
        let offset = usize::try_from(offset).expect("LP-I2C offset fits");
        let mut state = self.state.borrow_mut();
        let value =
            match offset {
                EspLpI2cState::STATUS => state.status(),
                EspLpI2cState::FIFO_STATUS => state.fifo_status(),
                EspLpI2cState::FIFO_DATA => {
                    let value = if let Some(byte) = state.pop_rx() {
                        u32::from(byte)
                    } else {
                        state.registers[EspLpI2cState::INT_RAW / 4] |=
                            EspLpI2cState::INT_RX_FIFO_UNDERFLOW;
                        0
                    };
                    state.refresh_watermarks();
                    value
                }
                EspLpI2cState::INT_STATUS => state.interrupt_status(),
                EspLpI2cState::TX_FIFO_START | EspLpI2cState::RX_FIFO_START => 0,
                _ => state.registers.get(offset / 4).copied().ok_or_else(|| {
                    DeviceError::new(format!("{} read at {offset:#x}", self.name))
                })?,
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
            return Err(DeviceError::new("ESP LP-I2C requires aligned word access"));
        }
        let offset = usize::try_from(offset).expect("LP-I2C offset fits");
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits");
        let mut state = self.state.borrow_mut();
        match offset {
            EspLpI2cState::FIFO_DATA => {
                if state.push_tx(value as u8) {
                    state.refresh_watermarks();
                } else {
                    state.registers[EspLpI2cState::INT_RAW / 4] |=
                        EspLpI2cState::INT_TX_FIFO_OVERFLOW;
                }
            }
            EspLpI2cState::CTR => {
                let stored = value & EspLpI2cState::CTR_RW_MASK;
                state.registers[offset / 4] = stored;
                if value & EspLpI2cState::CTR_FSM_RESET != 0 {
                    state.busy = false;
                }
                if value & EspLpI2cState::CTR_TRANS_START != 0 {
                    state.execute_commands();
                }
            }
            EspLpI2cState::FIFO_CONFIG => {
                state.registers[offset / 4] = value & EspLpI2cState::FIFO_CONFIG_MASK;
                if value & (1 << 12) != 0 {
                    state.rx_fifo.clear();
                }
                if value & (1 << 13) != 0 {
                    state.tx_fifo.clear();
                }
                state.refresh_watermarks();
            }
            EspLpI2cState::INT_CLEAR => {
                state.registers[EspLpI2cState::INT_RAW / 4] &=
                    !(value & EspLpI2cState::INTERRUPT_MASK);
            }
            EspLpI2cState::INT_RAW => {
                // R/SS/WTC: writing one clears a raw interrupt bit.
                state.registers[EspLpI2cState::INT_RAW / 4] &=
                    !(value & EspLpI2cState::INTERRUPT_MASK);
            }
            EspLpI2cState::INT_ENABLE => {
                state.registers[EspLpI2cState::INT_ENABLE / 4] =
                    value & EspLpI2cState::INTERRUPT_MASK;
            }
            EspLpI2cState::STATUS | EspLpI2cState::FIFO_STATUS | EspLpI2cState::INT_STATUS => {}
            EspLpI2cState::TX_FIFO_START | EspLpI2cState::RX_FIFO_START => {}
            offset
                if (EspLpI2cState::COMMAND_BASE
                    ..EspLpI2cState::COMMAND_BASE + EspLpI2cState::COMMAND_COUNT * 4)
                    .contains(&offset) =>
            {
                state.registers[offset / 4] = value & 0x0000_3fff;
            }
            EspLpI2cState::SCL_LOW_PERIOD
            | EspLpI2cState::SDA_HOLD
            | EspLpI2cState::SDA_SAMPLE
            | EspLpI2cState::SCL_START_HOLD
            | EspLpI2cState::SCL_RSTART_SETUP
            | EspLpI2cState::SCL_STOP_HOLD
            | EspLpI2cState::SCL_STOP_SETUP => state.registers[offset / 4] = value & 0x1ff,
            EspLpI2cState::TIMEOUT => state.registers[offset / 4] = value & 0x3f,
            EspLpI2cState::SCL_HIGH_PERIOD => state.registers[offset / 4] = value & 0xffff,
            EspLpI2cState::FILTER_CONFIG => state.registers[offset / 4] = value & 0x3ff,
            EspLpI2cState::CLOCK_CONFIG => state.registers[offset / 4] = value & 0x003f_ffff,
            EspLpI2cState::SCL_STATE_TIMEOUT | EspLpI2cState::SCL_MAIN_STATE_TIMEOUT => {
                state.registers[offset / 4] = value & 0x1f
            }
            EspLpI2cState::SCL_SPECIAL_CONFIG => state.registers[offset / 4] = value & 0xff,
            EspLpI2cState::DATE => state.registers[offset / 4] = value,
            _ => {
                let register = state.registers.get_mut(offset / 4).ok_or_else(|| {
                    DeviceError::new(format!("{} write at {offset:#x}", self.name))
                })?;
                *register = value;
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = EspLpI2cState::new();
    }
}
