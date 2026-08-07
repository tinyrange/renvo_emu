use super::*;

/// Scheduler-facing state for an ESP general-purpose SPI controller.
#[derive(Clone)]
pub struct EspSpiHandle {
    state: Rc<RefCell<EspSpiState>>,
}

impl EspSpiHandle {
    /// Queues deterministic bytes for the next master MISO transfer.
    pub fn queue_rx(&self, bytes: &[u8]) {
        self.state.borrow_mut().rx.extend(bytes.iter().copied());
    }

    /// Returns and clears bytes transmitted by the SPI master.
    pub fn take_tx(&self) -> Vec<u8> {
        std::mem::take(&mut self.state.borrow_mut().tx)
    }
}

struct EspSpiState {
    registers: Vec<u32>,
    tx: Vec<u8>,
    rx: VecDeque<u8>,
}

impl EspSpiState {
    const CMD: usize = 0x00;
    const USER: usize = 0x1c;
    const MOSI_DLEN: usize = 0x28;
    const MISO_DLEN: usize = 0x2c;
    const INT_RAW: usize = 0x54;
    const W0: usize = 0x80;
    const CMD_USR: u32 = 1 << 24;
    const USER_MOSI: u32 = 1 << 27;
    const USER_MISO: u32 = 1 << 28;

    fn new() -> Self {
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            tx: Vec::new(),
            rx: VecDeque::new(),
        };
        state.registers[0xf0 / 4] = 35_656_448;
        state
    }

    fn words_for_bits(bits_minus_one: u32) -> usize {
        usize::try_from((bits_minus_one & 0x3ffff).saturating_add(1).div_ceil(8))
            .expect("ESP SPI byte count fits usize")
    }

    fn transfer(&mut self) {
        let user = self.registers[Self::USER / 4];
        let mosi_len = Self::words_for_bits(self.registers[Self::MOSI_DLEN / 4]);
        let miso_len = Self::words_for_bits(self.registers[Self::MISO_DLEN / 4]);
        let mut transmitted = Vec::new();
        if user & Self::USER_MOSI != 0 {
            for index in 0..mosi_len {
                let word = Self::W0 / 4 + index / 4;
                let byte = (self.registers[word] >> ((index % 4) * 8)) as u8;
                transmitted.push(byte);
            }
            self.tx.extend_from_slice(&transmitted);
        }
        if user & Self::USER_MISO != 0 {
            let received = (0..miso_len)
                .map(|index| {
                    self.rx
                        .pop_front()
                        .unwrap_or_else(|| transmitted.get(index).copied().unwrap_or(0))
                })
                .collect::<Vec<_>>();
            for (index, byte) in received.into_iter().enumerate() {
                let word = Self::W0 / 4 + index / 4;
                let shift = (index % 4) * 8;
                self.registers[word] =
                    (self.registers[word] & !(0xff << shift)) | (u32::from(byte) << shift);
            }
        }
        self.registers[Self::INT_RAW / 4] |= 1;
        self.registers[Self::CMD / 4] &= !Self::CMD_USR;
    }
}

/// Functional ESP32-C6 general-purpose SPI2 master.
///
/// The model implements the native command/user/data windows used by simple
/// master-mode drivers. Transfers are synchronous on the abstract timeline;
/// queued host bytes provide MISO and absent bytes deterministically echo MOSI.
pub struct EspSpi {
    name: String,
    state: Rc<RefCell<EspSpiState>>,
}

impl EspSpi {
    /// Creates a reset SPI controller and host-facing transfer handle.
    pub fn new(name: impl Into<String>) -> (Self, EspSpiHandle) {
        let state = Rc::new(RefCell::new(EspSpiState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspSpiHandle { state },
        )
    }
}

impl Device for EspSpi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP SPI requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("ESP SPI offset fits");
        self.state
            .borrow()
            .registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP SPI requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("ESP SPI offset fits");
        let mut state = self.state.borrow_mut();
        let register = state
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        *register = u32::try_from(value & u64::from(u32::MAX)).expect("masked SPI value fits");
        if index == EspSpiState::CMD / 4 && *register & EspSpiState::CMD_USR != 0 {
            state.transfer();
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = EspSpiState::new();
    }
}
