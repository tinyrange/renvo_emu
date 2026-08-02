use super::*;

const FIFO_DEPTH: usize = 8;
const CR1_LBM: u32 = 1 << 0;
const CR1_SSE: u32 = 1 << 1;
const CR1_MS: u32 = 1 << 2;
const CR0_DSS_MASK: u32 = 0x0f;
const IMSC_MASK: u32 = 0x0f;

/// PrimeCell SSP registers used by the RP2350 SPI controllers.
#[derive(Clone, Copy)]
enum Rp2350SpiRegister {
    Cr0 = 0x000,
    Cr1 = 0x004,
    Dr = 0x008,
    Sr = 0x00c,
    Cpsr = 0x010,
    Imsc = 0x014,
    Ris = 0x018,
    Mis = 0x01c,
    Ic = 0x020,
    Dmacr = 0x024,
    PeriphId0 = 0xfe0,
    PeriphId1 = 0xfe4,
    PeriphId2 = 0xfe8,
    PeriphId3 = 0xfec,
    CellId0 = 0xff0,
    CellId1 = 0xff4,
    CellId2 = 0xff8,
    CellId3 = 0xffc,
}

impl Rp2350SpiRegister {
    fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x000 => Self::Cr0,
            0x004 => Self::Cr1,
            0x008 => Self::Dr,
            0x00c => Self::Sr,
            0x010 => Self::Cpsr,
            0x014 => Self::Imsc,
            0x018 => Self::Ris,
            0x01c => Self::Mis,
            0x020 => Self::Ic,
            0x024 => Self::Dmacr,
            0xfe0 => Self::PeriphId0,
            0xfe4 => Self::PeriphId1,
            0xfe8 => Self::PeriphId2,
            0xfec => Self::PeriphId3,
            0xff0 => Self::CellId0,
            0xff4 => Self::CellId1,
            0xff8 => Self::CellId2,
            0xffc => Self::CellId3,
            _ => return None,
        })
    }

    const fn offset(self) -> u64 {
        self as u64
    }
}

struct Rp2350SpiState {
    registers: [u32; 0x1000 / 4],
    tx_fifo: VecDeque<u16>,
    rx_fifo: VecDeque<u16>,
    queued_input: VecDeque<u16>,
    output: Vec<u16>,
    receive_overrun: bool,
    receive_timeout: bool,
}

impl Rp2350SpiState {
    fn reset() -> Self {
        let mut registers = [0; 0x1000 / 4];
        registers[Rp2350SpiRegister::PeriphId0.offset() as usize / 4] = 0x22;
        registers[Rp2350SpiRegister::PeriphId1.offset() as usize / 4] = 0x10;
        registers[Rp2350SpiRegister::PeriphId2.offset() as usize / 4] = 0x34;
        registers[Rp2350SpiRegister::PeriphId3.offset() as usize / 4] = 0;
        registers[Rp2350SpiRegister::CellId0.offset() as usize / 4] = 0x0d;
        registers[Rp2350SpiRegister::CellId1.offset() as usize / 4] = 0xf0;
        registers[Rp2350SpiRegister::CellId2.offset() as usize / 4] = 0x05;
        registers[Rp2350SpiRegister::CellId3.offset() as usize / 4] = 0xb1;
        Self {
            registers,
            tx_fifo: VecDeque::new(),
            rx_fifo: VecDeque::new(),
            queued_input: VecDeque::new(),
            output: Vec::new(),
            receive_overrun: false,
            receive_timeout: false,
        }
    }

    fn data_mask(&self) -> u16 {
        let dss = self.registers[Rp2350SpiRegister::Cr0.offset() as usize / 4] & CR0_DSS_MASK;
        let bits = dss.saturating_add(1).clamp(4, 16);
        u16::MAX >> (16 - bits)
    }

    fn raw_interrupts(&self) -> u32 {
        let mut raw = 0;
        if self.tx_fifo.len() <= FIFO_DEPTH / 2 {
            raw |= 1 << 3;
        }
        if self.rx_fifo.len() >= FIFO_DEPTH / 2 {
            raw |= 1 << 2;
        }
        if self.receive_timeout && !self.rx_fifo.is_empty() {
            raw |= 1 << 1;
        }
        if self.receive_overrun {
            raw |= 1;
        }
        raw
    }

    fn status(&self) -> u32 {
        let mut status = 0;
        if self.tx_fifo.len() > 0 {
            status |= 1 << 4;
        }
        if self.rx_fifo.len() >= FIFO_DEPTH {
            status |= 1 << 3;
        }
        if !self.rx_fifo.is_empty() {
            status |= 1 << 2;
        }
        if self.tx_fifo.len() < FIFO_DEPTH {
            status |= 1 << 1;
        }
        if self.tx_fifo.is_empty() {
            status |= 1;
        }
        status
    }

    fn process_tx(&mut self) {
        let cr1 = self.registers[Rp2350SpiRegister::Cr1.offset() as usize / 4];
        if cr1 & CR1_SSE == 0 {
            return;
        }
        let mask = self.data_mask();
        while let Some(value) = self.tx_fifo.pop_front() {
            let value = value & mask;
            self.output.push(value);
            let response = if cr1 & CR1_LBM != 0 {
                value
            } else {
                self.queued_input.pop_front().unwrap_or(0)
            };
            if self.rx_fifo.len() < FIFO_DEPTH {
                self.rx_fifo.push_back(response & mask);
            } else {
                self.receive_overrun = true;
            }
        }
    }
}

/// Functional RP2350 SPI0/SPI1 PrimeCell SSP controller.
///
/// The model preserves the documented eight-word FIFOs, control/status registers and interrupt
/// conditions. Transfers complete immediately in abstract time: writes are captured by the host
/// handle and each frame receives either queued host data, loopback data, or zero when no external
/// response is supplied. Bit-level clock waveforms and DMA handshakes are intentionally outside
/// this functional slice.
pub struct Rp2350Spi {
    name: String,
    state: Arc<Mutex<Rp2350SpiState>>,
}

/// Host-side control surface for an RP2350 SPI controller.
#[derive(Clone)]
pub struct Rp2350SpiHandle {
    state: Arc<Mutex<Rp2350SpiState>>,
}

impl Rp2350Spi {
    /// Creates a reset-state SPI controller and its host handle.
    pub fn new(name: impl Into<String>) -> (Self, Rp2350SpiHandle) {
        let state = Arc::new(Mutex::new(Rp2350SpiState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Rp2350SpiHandle { state },
        )
    }
}

impl Rp2350SpiHandle {
    /// Queues words returned by the external SPI slave on subsequent frames.
    pub fn queue_input(&self, words: &[u16]) {
        self.state
            .lock()
            .expect("RP2350 SPI lock poisoned")
            .queued_input
            .extend(words.iter().copied());
    }

    /// Drains words transmitted by firmware since the previous call.
    pub fn take_output(&self) -> Vec<u16> {
        let mut state = self.state.lock().expect("RP2350 SPI lock poisoned");
        std::mem::take(&mut state.output)
    }

    /// Returns the raw PrimeCell interrupt condition bits.
    pub fn raw_interrupts(&self) -> u32 {
        self.state
            .lock()
            .expect("RP2350 SPI lock poisoned")
            .raw_interrupts()
    }

    /// Returns the masked interrupt output for the corresponding RP2350 IRQ line.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("RP2350 SPI lock poisoned");
        state.raw_interrupts()
            & state.registers[Rp2350SpiRegister::Imsc.offset() as usize / 4]
            & IMSC_MASK
            != 0
    }
}

impl Device for Rp2350Spi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new("RP2350 SPI requires aligned word access"));
        }
        let register = Rp2350SpiRegister::from_offset(offset & 0x0fff)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?;
        let mut state = self.state.lock().expect("RP2350 SPI lock poisoned");
        let value = match register {
            Rp2350SpiRegister::Dr => u32::from(state.rx_fifo.pop_front().unwrap_or(0)),
            Rp2350SpiRegister::Sr => state.status(),
            Rp2350SpiRegister::Cpsr => state.registers[register.offset() as usize / 4] & 0xfe,
            Rp2350SpiRegister::Ris => state.raw_interrupts(),
            Rp2350SpiRegister::Mis => {
                state.raw_interrupts()
                    & state.registers[Rp2350SpiRegister::Imsc.offset() as usize / 4]
                    & IMSC_MASK
            }
            Rp2350SpiRegister::Ic => 0,
            _ => state.registers[register.offset() as usize / 4],
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
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new("RP2350 SPI requires aligned word access"));
        }
        let register = Rp2350SpiRegister::from_offset(offset & 0x0fff)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        let value =
            u32::try_from(value).map_err(|_| DeviceError::new("RP2350 SPI value overflow"))?;
        let mut state = self.state.lock().expect("RP2350 SPI lock poisoned");
        match register {
            Rp2350SpiRegister::Dr => {
                if state.tx_fifo.len() >= FIFO_DEPTH {
                    return Err(DeviceError::new("RP2350 SPI transmit FIFO is full"));
                }
                let mask = state.data_mask();
                state.tx_fifo.push_back((value as u16) & mask);
                state.process_tx();
            }
            Rp2350SpiRegister::Cr0 => {
                state.registers[register.offset() as usize / 4] = value & 0x0000_ffff;
            }
            Rp2350SpiRegister::Cr1 => {
                let old = state.registers[register.offset() as usize / 4];
                let mut next = value & 0x0f;
                if old & CR1_SSE != 0 {
                    next = (next & !CR1_MS) | (old & CR1_MS);
                }
                state.registers[register.offset() as usize / 4] = next;
                state.process_tx();
            }
            Rp2350SpiRegister::Cpsr => {
                state.registers[register.offset() as usize / 4] = value & 0xfe;
            }
            Rp2350SpiRegister::Imsc => {
                state.registers[register.offset() as usize / 4] = value & IMSC_MASK;
            }
            Rp2350SpiRegister::Ic => {
                if value & (1 << 1) != 0 {
                    state.receive_timeout = false;
                }
                if value & 1 != 0 {
                    state.receive_overrun = false;
                }
            }
            Rp2350SpiRegister::Dmacr => {
                state.registers[register.offset() as usize / 4] = value & 3;
            }
            Rp2350SpiRegister::Sr
            | Rp2350SpiRegister::Ris
            | Rp2350SpiRegister::Mis
            | Rp2350SpiRegister::PeriphId0
            | Rp2350SpiRegister::PeriphId1
            | Rp2350SpiRegister::PeriphId2
            | Rp2350SpiRegister::PeriphId3
            | Rp2350SpiRegister::CellId0
            | Rp2350SpiRegister::CellId1
            | Rp2350SpiRegister::CellId2
            | Rp2350SpiRegister::CellId3 => {
                return Err(DeviceError::new("RP2350 SPI register is read-only"));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RP2350 SPI lock poisoned") = Rp2350SpiState::reset();
    }
}
