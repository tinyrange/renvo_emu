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
    const INT_ENABLE: usize = 0x24;
    const INT_STATUS: usize = 0x28;
    const INT_CLEAR: usize = 0x30;
    const START: u32 = 1;
    const STOP: u32 = 1 << 1;
    const TRANSFER_COMPLETE: u32 = 1 << 7;

    fn new() -> Self {
        Self {
            registers: vec![0; 0x1000 / 4],
            tx: Vec::new(),
            rx: VecDeque::new(),
            busy: false,
        }
    }

    fn status(&self) -> u32 {
        // The functional FIFO is never full; bit 0 is the active master state.
        u32::from(self.busy)
    }
}

/// Functional ESP32-C6 I2C0 master FIFO/controller slice.
///
/// Register writes retain the native control/status/FIFO/interrupt windows.
/// START and STOP are deterministic abstract actions; bytes written to the
/// FIFO are exposed through the host handle, and queued host bytes are returned
/// by FIFO reads. Command-link timing, arbitration, clock stretching, and DMA
/// remain outside this functional slice.
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
                if value & EspI2cState::START != 0 {
                    state.busy = true;
                }
                if value & EspI2cState::STOP != 0 {
                    state.busy = false;
                    state.registers[EspI2cState::INT_RAW / 4] |= EspI2cState::TRANSFER_COMPLETE;
                }
            }
            EspI2cState::INT_CLEAR => state.registers[EspI2cState::INT_RAW / 4] &= !value,
            EspI2cState::STATUS | EspI2cState::INT_STATUS => {}
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
