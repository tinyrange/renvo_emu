use super::*;

/// Host-facing state for an ESP I2C controller.
#[derive(Clone)]
pub struct EspI2cHandle {
    state: Rc<RefCell<EspI2cState>>,
}

impl EspI2cHandle {
    /// Queues bytes returned by the next controller FIFO read.
    pub fn queue_rx(&self, bytes: &[u8]) {
        self.state.borrow_mut().rx.extend(bytes.iter().copied());
    }

    /// Returns and clears bytes written to the controller FIFO.
    pub fn take_tx(&self) -> Vec<u8> {
        std::mem::take(&mut self.state.borrow_mut().tx)
    }

    /// Reports whether a START has left the controller busy.
    pub fn busy(&self) -> bool {
        self.state.borrow().busy
    }
}

struct EspI2cState {
    registers: Vec<u32>,
    tx: Vec<u8>,
    rx: VecDeque<u8>,
    busy: bool,
}

impl EspI2cState {
    const CTR: usize = 0x04;
    const STATUS: usize = 0x08;
    const FIFO_DATA: usize = 0x1c;
    const INT_RAW: usize = 0x20;
    const INT_CLEAR: usize = 0x24;
    const INT_ENABLE: usize = 0x28;
    const INT_STATUS: usize = 0x2c;
    const COMMAND_BASE: usize = 0x58;
    const COMMAND_COUNT: usize = 8;
    const BUS_BUSY: u32 = 1 << 4;
    const TRANS_START: u32 = 1 << 5;
    const COMMAND_OPCODE_SHIFT: u32 = 11;
    const COMMAND_OPCODE_MASK: u32 = 0x7;
    const COMMAND_RESTART: u32 = 6;
    const COMMAND_WRITE: u32 = 1;
    const COMMAND_READ: u32 = 3;
    const COMMAND_STOP: u32 = 2;
    const COMMAND_END: u32 = 4;
    const TRANSFER_COMPLETE: u32 = 1 << 7;
    const TRANSFER_START: u32 = 1 << 9;

    fn new() -> Self {
        Self {
            registers: vec![0; 0x1000 / 4],
            tx: Vec::new(),
            rx: VecDeque::new(),
            busy: false,
        }
    }

    fn status(&self) -> u32 {
        // The native BUS_BUSY status is bit 4. FIFO watermark and detailed
        // state-machine fields remain outside this bounded model.
        u32::from(self.busy) * Self::BUS_BUSY
    }

    fn execute_commands(&mut self) {
        self.busy = true;
        self.registers[Self::INT_RAW / 4] |= Self::TRANSFER_START;
        for command_index in 0..Self::COMMAND_COUNT {
            let index = (Self::COMMAND_BASE + command_index * 4) / 4;
            let command = self.registers[index];
            let opcode = (command >> Self::COMMAND_OPCODE_SHIFT) & Self::COMMAND_OPCODE_MASK;
            self.registers[index] = command | (1 << 31);
            match opcode {
                Self::COMMAND_RESTART | Self::COMMAND_WRITE | Self::COMMAND_READ => {}
                Self::COMMAND_STOP => {
                    self.busy = false;
                    self.registers[Self::INT_RAW / 4] |= Self::TRANSFER_COMPLETE;
                    return;
                }
                Self::COMMAND_END => return,
                _ => return,
            }
        }
    }
}

/// Functional ESP32-C6 I2C0 master FIFO/controller slice.
///
/// Register writes retain the native control/status/FIFO/interrupt and command
/// windows. A write to `CTR.TRANS_START` executes the programmed command link;
/// command opcodes provide restart, write, read, stop, and end actions. Bytes
/// written to the FIFO are exposed through the host handle, and queued host
/// bytes are returned by FIFO reads. Command-link timing, arbitration, clock
/// stretching, and DMA remain outside this functional slice.
pub struct EspI2c {
    name: String,
    state: Rc<RefCell<EspI2cState>>,
}

impl EspI2c {
    /// Creates a reset I2C controller and host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, EspI2cHandle) {
        let state = Rc::new(RefCell::new(EspI2cState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspI2cHandle { state },
        )
    }
}

impl Device for EspI2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP I2C requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("ESP I2C offset fits");
        let mut state = self.state.borrow_mut();
        let value = match offset as usize {
            EspI2cState::STATUS => state.status(),
            EspI2cState::FIFO_DATA => u32::from(state.rx.pop_front().unwrap_or(0)),
            EspI2cState::INT_STATUS => {
                state.registers[EspI2cState::INT_RAW / 4]
                    & state.registers[EspI2cState::INT_ENABLE / 4]
            }
            _ => *state
                .registers
                .get(index)
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?,
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
            return Err(DeviceError::new("ESP I2C requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("ESP I2C offset fits");
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked I2C value fits");
        let mut state = self.state.borrow_mut();
        match offset as usize {
            EspI2cState::FIFO_DATA => state.tx.push(value as u8),
            EspI2cState::CTR => {
                state.registers[index] = value;
                if value & EspI2cState::TRANS_START != 0 {
                    state.execute_commands();
                }
            }
            EspI2cState::INT_CLEAR => state.registers[EspI2cState::INT_RAW / 4] &= !value,
            EspI2cState::STATUS | EspI2cState::INT_STATUS => {}
            offset
                if offset >= EspI2cState::COMMAND_BASE
                    && offset < EspI2cState::COMMAND_BASE + EspI2cState::COMMAND_COUNT * 4
                    && offset & 3 == 0 =>
            {
                state.registers[index] = value & !(1 << 31);
            }
            _ => {
                let register = state.registers.get_mut(index).ok_or_else(|| {
                    DeviceError::new(format!("{} write at {offset:#x}", self.name))
                })?;
                *register = value;
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = EspI2cState::new();
    }
}
