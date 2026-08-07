use super::*;

const FIFO_CAPACITY: usize = 256;

#[derive(Default)]
struct EspSdioState {
    hinf: Vec<u32>,
    slc: Vec<u32>,
    rx: [VecDeque<u32>; 2],
    tx: [VecDeque<u32>; 2],
}

impl EspSdioState {
    fn new() -> Self {
        let mut state = Self {
            hinf: vec![0; 0x100 / 4],
            slc: vec![0; 0x200 / 4],
            rx: [VecDeque::new(), VecDeque::new()],
            tx: [VecDeque::new(), VecDeque::new()],
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.hinf.fill(0);
        self.slc.fill(0);
        self.rx.iter_mut().for_each(VecDeque::clear);
        self.tx.iter_mut().for_each(VecDeque::clear);
        self.hinf[0x00 / 4] = 0x0092_6666;
        self.hinf[0x04 / 4] = (562 << 12) | (1 << 4) | 1;
        self.hinf[0x08 / 4] = (1 << 28) | (1400 << 16) | (2 << 10) | (2 << 3) | 2;
        self.hinf[0x1c / 4] = 1 << 17;
        for offset in (0x20..=0x3c).step_by(4) {
            self.hinf[offset / 4] = u32::MAX;
        }
        self.hinf[0x40 / 4] = 0x0092_7777;
        self.hinf[0xfc / 4] = 35_664_208;
        self.slc[0x00 / 4] = (1 << 31)
            | (1 << 30)
            | (1 << 29)
            | (1 << 28)
            | (1 << 27)
            | (1 << 26)
            | (1 << 25)
            | (1 << 24)
            | (1 << 13)
            | (1 << 12)
            | (1 << 11)
            | (1 << 10)
            | (1 << 9)
            | (1 << 8);
        self.slc[0x24 / 4] = (1 << 17) | (1 << 1);
        self.slc[0x30 / 4] = (1 << 17) | (1 << 1);
        self.slc[0x1f8 / 4] = 554_182_400;
        self.slc[0x1fc / 4] = 256;
    }

    fn fifo_reset(&mut self, function: usize) {
        self.rx[function].clear();
        self.tx[function].clear();
        let raw = 0x04 + function * 0x10;
        self.slc[raw / 4] &= !(1 << 16);
        self.slc[(raw + 4) / 4] &= !(1 << 16);
    }

    fn update_status(&mut self) {
        let rx0 = self.rx[0].len().min(0x3fff);
        let rx1 = self.rx[1].len().min(0x3fff);
        self.slc[0x24 / 4] = (u32::from(rx0 == 0) << 1)
            | ((rx0 as u32) << 2)
            | (u32::from(rx1 == 0) << 17)
            | ((rx1 as u32) << 18);
        let tx0 = self.tx[0].len();
        let tx1 = self.tx[1].len();
        self.slc[0x30 / 4] = (u32::from(tx0 == 0) << 1)
            | (u32::from(tx0 >= FIFO_CAPACITY) | (u32::from(tx1 >= FIFO_CAPACITY) << 16))
            | (u32::from(tx1 == 0) << 17);
    }

    fn push_rx(&mut self, function: usize, value: u32) {
        if self.rx[function].len() < FIFO_CAPACITY {
            self.rx[function].push_back(value & 0x1ff);
            let raw = 0x04 + function * 0x10;
            self.slc[raw / 4] |= 1 << 16;
        } else {
            let raw = 0x04 + function * 0x10;
            self.slc[raw / 4] |= 1 << 10;
        }
        self.update_status();
    }

    fn pop_tx(&mut self, function: usize) -> u32 {
        let value = self.tx[function].pop_front().unwrap_or(0x400);
        let raw = 0x04 + function * 0x10;
        if self.tx[function].is_empty() {
            self.slc[raw / 4] |= 1 << 14;
        }
        self.update_status();
        value
    }

    fn peek_tx(&self, function: usize) -> u32 {
        self.tx[function].front().copied().unwrap_or(0x400)
    }

    fn read_hinf(&mut self, offset: u64) -> u32 {
        let index = (offset as usize) / 4;
        self.hinf.get(index).copied().unwrap_or_default()
    }

    fn write_hinf(&mut self, offset: u64, value: u32) {
        match offset {
            0x04 => {
                let writable = 0xfee0_7077;
                let old = self.hinf[0x04 / 4];
                self.hinf[0x04 / 4] = (old & !writable) | (value & writable);
            }
            0x0c => {}
            0x1c => {
                if value & (1 << 16) != 0 {
                    self.fifo_reset(0);
                    self.fifo_reset(1);
                }
                self.hinf[0x1c / 4] = value & 0x0003_ffff;
            }
            _ => {
                if let Some(register) = self.hinf.get_mut((offset as usize) / 4) {
                    *register = value;
                }
            }
        }
    }

    fn read_slc(&mut self, offset: u64) -> u32 {
        self.update_status();
        match offset {
            0x08 => self.slc[0x04 / 4] & self.slc[0x0c / 4],
            0x18 => self.slc[0x14 / 4] & self.slc[0x1c / 4],
            0x24 | 0x30 => self.slc[(offset as usize) / 4],
            0x34 => self.peek_tx(0),
            0x38 => self.peek_tx(1),
            _ => self
                .slc
                .get((offset as usize) / 4)
                .copied()
                .unwrap_or_default(),
        }
    }

    fn write_slc(&mut self, offset: u64, value: u32) {
        match offset {
            0x00 => {
                self.slc[0] = value;
                if value & 1 != 0 || value & 2 != 0 {
                    self.fifo_reset(0);
                }
                if value & (1 << 16) != 0 || value & (1 << 17) != 0 {
                    self.fifo_reset(1);
                }
            }
            0x10 => self.slc[0x04 / 4] &= !value,
            0x20 => self.slc[0x14 / 4] &= !value,
            0x28 => {
                if value & (1 << 16) != 0 {
                    self.push_rx(0, value);
                }
            }
            0x2c => {
                if value & (1 << 16) != 0 {
                    self.push_rx(1, value);
                }
            }
            0x34 => {
                if value & (1 << 16) != 0 {
                    let _ = self.pop_tx(0);
                }
            }
            0x38 => {
                if value & (1 << 16) != 0 {
                    let _ = self.pop_tx(1);
                }
            }
            0x60 | 0x64 | 0x68 | 0x6c => self.write_token(offset, value),
            _ => {
                if let Some(register) = self.slc.get_mut((offset as usize) / 4) {
                    *register = value;
                }
            }
        }
        self.update_status();
    }

    fn write_token(&mut self, offset: u64, value: u32) {
        let index = (offset as usize) / 4;
        let current = self.slc[index] >> 16 & 0xfff;
        let data = value & 0xfff;
        let next = if value & (1 << 12) != 0 {
            data
        } else if value & (1 << 14) != 0 {
            current.wrapping_add(data) & 0xfff
        } else if value & (1 << 13) != 0 {
            current.wrapping_add(1) & 0xfff
        } else {
            current
        };
        self.slc[index] = next << 16;
    }
}

/// Host-facing SDIO slave FIFO access for deterministic tests.
#[derive(Clone)]
pub struct EspSdioSlaveHandle {
    state: Rc<RefCell<EspSdioState>>,
}

impl EspSdioSlaveHandle {
    /// Queues words received from an SDIO host on function zero or one.
    pub fn queue_rx(&self, function: usize, words: &[u32]) {
        if function > 1 {
            return;
        }
        let mut state = self.state.borrow_mut();
        for &word in words {
            state.push_rx(function, word);
        }
    }

    /// Queues a response word for the host to consume from the TX FIFO.
    pub fn queue_tx(&self, function: usize, words: &[u32]) {
        if function > 1 {
            return;
        }
        let mut state = self.state.borrow_mut();
        for &word in words.iter().take(FIFO_CAPACITY - state.tx[function].len()) {
            state.tx[function].push_back(word & 0x7ff);
        }
        state.update_status();
    }

    /// Drains words made available to the host on a TX FIFO.
    pub fn take_tx(&self, function: usize) -> Vec<u32> {
        if function > 1 {
            return Vec::new();
        }
        let mut state = self.state.borrow_mut();
        let words = state.tx[function].drain(..).collect();
        state.update_status();
        words
    }

    /// Returns whether an enabled SLC interrupt is pending for a function.
    pub fn interrupt_pending(&self, function: usize) -> bool {
        if function > 1 {
            return false;
        }
        let state = self.state.borrow();
        let base = 0x04 + function * 0x10;
        state.slc[base / 4] & state.slc[(base + 8) / 4] != 0
    }
}

/// ESP32-C6 SDIO host-interface register block.
pub struct EspSdioHinf {
    name: String,
    state: Rc<RefCell<EspSdioState>>,
}

/// ESP32-C6 SDIO link/FIFO controller register block.
pub struct EspSdioSlc {
    name: String,
    state: Rc<RefCell<EspSdioState>>,
}

/// Creates the ESP32-C6 SDIO HINF and SLC blocks with a shared host handle.
pub fn new_esp_sdio_slave(
    name: impl Into<String>,
) -> (EspSdioHinf, EspSdioSlc, EspSdioSlaveHandle) {
    let state = Rc::new(RefCell::new(EspSdioState::new()));
    let name = name.into();
    (
        EspSdioHinf {
            name: format!("{name}.hinf"),
            state: state.clone(),
        },
        EspSdioSlc {
            name: format!("{name}.slc"),
            state: state.clone(),
        },
        EspSdioSlaveHandle { state },
    )
}

impl Device for EspSdioHinf {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 || offset >= 0x100 {
            return Err(DeviceError::new(
                "ESP32-C6 SDIO HINF requires aligned word access",
            ));
        }
        Ok(u64::from(self.state.borrow_mut().read_hinf(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 || offset >= 0x100 {
            return Err(DeviceError::new(
                "ESP32-C6 SDIO HINF requires aligned word access",
            ));
        }
        self.state.borrow_mut().write_hinf(offset, value as u32);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}

impl Device for EspSdioSlc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 || offset >= 0x200 {
            return Err(DeviceError::new(
                "ESP32-C6 SDIO SLC requires aligned word access",
            ));
        }
        Ok(u64::from(self.state.borrow_mut().read_slc(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 || offset >= 0x200 {
            return Err(DeviceError::new(
                "ESP32-C6 SDIO SLC requires aligned word access",
            ));
        }
        self.state.borrow_mut().write_slc(offset, value as u32);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_matches_c6_identity_and_ready_defaults() {
        let (mut hinf, _, _) = new_esp_sdio_slave("sdio");
        assert_eq!(
            hinf.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x0092_6666
        );
        assert_eq!(
            hinf.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap() & 0x1f,
            0x11
        );
        assert_eq!(
            hinf.read(0x1c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 17
        );
    }

    #[test]
    fn host_queue_sets_interrupt_and_fifo_status() {
        let (mut hinf, mut slc, handle) = new_esp_sdio_slave("sdio");
        handle.queue_rx(0, &[0x123, 0x456]);
        slc.write(0x0c, AccessWidth::Word, 1 << 16, SimTime::ZERO)
            .unwrap();
        assert!(handle.interrupt_pending(0));
        assert_eq!(
            slc.read(0x24, AccessWidth::Word, SimTime::ZERO).unwrap() >> 2 & 0x3fff,
            2
        );
        hinf.write(0x1c, AccessWidth::Word, 1 << 16, SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending(0));
    }

    #[test]
    fn tx_fifo_is_host_observable_and_token_commands_are_deterministic() {
        let (_, mut slc, handle) = new_esp_sdio_slave("sdio");
        handle.queue_tx(1, &[0x55, 0xaa]);
        assert_eq!(
            slc.read(0x38, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x55
        );
        slc.write(0x38, AccessWidth::Word, 1 << 16, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.take_tx(1), vec![0xaa]);
        slc.write(0x60, AccessWidth::Word, 7 | (1 << 12), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            slc.read(0x60, AccessWidth::Word, SimTime::ZERO).unwrap() >> 16,
            7
        );
    }
}
