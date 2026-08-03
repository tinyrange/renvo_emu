use super::*;
use std::collections::VecDeque;

// The RP2040 DW_apb_ssi integration has sixteen 32-bit entries in each FIFO.
const FIFO_DEPTH: usize = 16;
const CTRL0_MASK: u32 = 0x017f_ffff;
const CTRL1_MASK: u32 = 0x0000_ffff;
const SPI_CTRL0_MASK: u32 = 0xff07_fb3f;
const INTERRUPT_MASK: u32 = 0x3f;

/// Native RP2040 DW_apb_ssi register identifiers.
///
/// The RP2040 SPI blocks are not PrimeCell SSP peripherals. They use the Synopsys DW SSI
/// register contract, with up to 36 data-register windows starting at `0x60`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rp2040SpiRegister {
    /// Control register 0.
    Ctrlr0,
    /// Control register 1.
    Ctrlr1,
    /// SSI enable register.
    SsiEnr,
    /// Microwire control register.
    Mwcr,
    /// Slave-enable register.
    Ser,
    /// Baud-rate divisor.
    Baudr,
    /// Transmit FIFO threshold.
    Txftlr,
    /// Receive FIFO threshold.
    Rxftlr,
    /// Transmit FIFO level (read-only).
    Txflr,
    /// Receive FIFO level (read-only).
    Rxflr,
    /// Status register (read-only).
    Sr,
    /// Interrupt mask.
    Imr,
    /// Masked interrupt status (read-only).
    Isr,
    /// Raw interrupt status (read-only).
    Risr,
    /// Transmit FIFO overflow interrupt clear (read-only).
    Txoicr,
    /// Receive FIFO overflow interrupt clear (read-only).
    Rxoicr,
    /// Receive FIFO underflow interrupt clear (read-only).
    Rxuicr,
    /// Multi-master interrupt clear (read-only).
    Msticr,
    /// All-interrupt clear (read-only).
    Icr,
    /// DMA control.
    Dmacr,
    /// DMA transmit-data level.
    Dmatdlr,
    /// DMA receive-data level.
    Dmardlr,
    /// Peripheral identification register.
    Idr,
    /// SSI version identifier.
    SsiVersionId,
    /// Data register window, indexed from DR0 through DR35.
    Data(u8),
    /// Receive sample delay.
    RxSampleDly,
    /// SPI control register 0.
    SpiCtrlr0,
    /// Transmit drive edge.
    TxdDriveEdge,
}

impl Rp2040SpiRegister {
    /// Converts a controller-relative byte offset into a typed register identifier.
    pub fn from_offset(offset: u64) -> Option<Self> {
        if (0x60..=0xec).contains(&offset) && offset % 4 == 0 {
            return Some(Self::Data(u8::try_from((offset - 0x60) / 4).ok()?));
        }
        Some(match offset {
            0x00 => Self::Ctrlr0,
            0x04 => Self::Ctrlr1,
            0x08 => Self::SsiEnr,
            0x0c => Self::Mwcr,
            0x10 => Self::Ser,
            0x14 => Self::Baudr,
            0x18 => Self::Txftlr,
            0x1c => Self::Rxftlr,
            0x20 => Self::Txflr,
            0x24 => Self::Rxflr,
            0x28 => Self::Sr,
            0x2c => Self::Imr,
            0x30 => Self::Isr,
            0x34 => Self::Risr,
            0x38 => Self::Txoicr,
            0x3c => Self::Rxoicr,
            0x40 => Self::Rxuicr,
            0x44 => Self::Msticr,
            0x48 => Self::Icr,
            0x4c => Self::Dmacr,
            0x50 => Self::Dmatdlr,
            0x54 => Self::Dmardlr,
            0x58 => Self::Idr,
            0x5c => Self::SsiVersionId,
            0xf0 => Self::RxSampleDly,
            0xf4 => Self::SpiCtrlr0,
            0xf8 => Self::TxdDriveEdge,
            _ => return None,
        })
    }

    /// Returns the controller-relative byte offset for this register identifier.
    pub const fn offset(self) -> u64 {
        match self {
            Self::Ctrlr0 => 0x00,
            Self::Ctrlr1 => 0x04,
            Self::SsiEnr => 0x08,
            Self::Mwcr => 0x0c,
            Self::Ser => 0x10,
            Self::Baudr => 0x14,
            Self::Txftlr => 0x18,
            Self::Rxftlr => 0x1c,
            Self::Txflr => 0x20,
            Self::Rxflr => 0x24,
            Self::Sr => 0x28,
            Self::Imr => 0x2c,
            Self::Isr => 0x30,
            Self::Risr => 0x34,
            Self::Txoicr => 0x38,
            Self::Rxoicr => 0x3c,
            Self::Rxuicr => 0x40,
            Self::Msticr => 0x44,
            Self::Icr => 0x48,
            Self::Dmacr => 0x4c,
            Self::Dmatdlr => 0x50,
            Self::Dmardlr => 0x54,
            Self::Idr => 0x58,
            Self::SsiVersionId => 0x5c,
            Self::Data(index) => 0x60 + (index as u64) * 4,
            Self::RxSampleDly => 0xf0,
            Self::SpiCtrlr0 => 0xf4,
            Self::TxdDriveEdge => 0xf8,
        }
    }
}

#[derive(Clone)]
struct Rp2040SpiState {
    registers: [u32; 0x100 / 4],
    tx_fifo: VecDeque<u32>,
    rx_fifo: VecDeque<u32>,
    queued_input: VecDeque<u32>,
    transmitted: Vec<u8>,
    receive_overrun: bool,
    receive_underflow: bool,
    transmit_overflow: bool,
}

impl Default for Rp2040SpiState {
    fn default() -> Self {
        Self::reset()
    }
}

impl Rp2040SpiState {
    fn reset() -> Self {
        let mut registers = [0; 0x100 / 4];
        registers[Rp2040SpiRegister::Idr.offset() as usize / 4] = 0x5153_5049;
        registers[Rp2040SpiRegister::SsiVersionId.offset() as usize / 4] = 0x3430_312a;
        registers[Rp2040SpiRegister::SpiCtrlr0.offset() as usize / 4] = 0x0300_0000;
        Self {
            registers,
            tx_fifo: VecDeque::new(),
            rx_fifo: VecDeque::new(),
            queued_input: VecDeque::new(),
            transmitted: Vec::new(),
            receive_overrun: false,
            receive_underflow: false,
            transmit_overflow: false,
        }
    }

    fn data_mask(&self) -> u32 {
        let raw = self.registers[Rp2040SpiRegister::Ctrlr0.offset() as usize / 4] & 0x0f;
        // DW SSI encodes a 4..16-bit frame as value-minus-one. Treat the reset value as the
        // useful eight-bit default used by the Pico SDK's byte-oriented helpers.
        let bits = if raw < 3 {
            8
        } else {
            raw.saturating_add(1).min(16)
        };
        u32::MAX >> (32 - bits)
    }

    fn enabled(&self) -> bool {
        self.registers[Rp2040SpiRegister::SsiEnr.offset() as usize / 4] & 1 != 0
            && self.registers[Rp2040SpiRegister::Ser.offset() as usize / 4] & 1 != 0
    }

    fn status(&self) -> u32 {
        let mut status = 0;
        if self.tx_fifo.len() >= FIFO_DEPTH {
            status |= 1 << 5;
        }
        if self.rx_fifo.len() >= FIFO_DEPTH {
            status |= 1 << 4;
        }
        if !self.rx_fifo.is_empty() {
            status |= 1 << 3;
        }
        if self.tx_fifo.is_empty() {
            status |= 1 << 2;
        }
        if self.tx_fifo.len() < FIFO_DEPTH {
            status |= 1 << 1;
        }
        // Transfers complete in one functional step, so BUSY remains low between accesses.
        status
    }

    fn raw_interrupts(&self) -> u32 {
        let tx_threshold =
            usize::try_from(self.registers[Rp2040SpiRegister::Txftlr.offset() as usize / 4] & 0xff)
                .expect("threshold fits usize");
        let rx_threshold =
            usize::try_from(self.registers[Rp2040SpiRegister::Rxftlr.offset() as usize / 4] & 0xff)
                .expect("threshold fits usize");
        let mut raw = 0;
        if self.tx_fifo.len() <= tx_threshold {
            raw |= 1;
        }
        if self.receive_overrun {
            raw |= 1 << 3;
        }
        if self.receive_underflow {
            raw |= 1 << 2;
        }
        if self.transmit_overflow {
            raw |= 1 << 1;
        }
        if self.rx_fifo.len() > rx_threshold {
            raw |= 1 << 4;
        }
        raw
    }

    fn process_tx(&mut self) {
        if !self.enabled() {
            return;
        }
        let mask = self.data_mask();
        while let Some(value) = self.tx_fifo.pop_front() {
            let value = value & mask;
            self.transmitted.push(value as u8);
            let response = self.queued_input.pop_front().unwrap_or(value) & mask;
            if self.rx_fifo.len() < FIFO_DEPTH {
                self.rx_fifo.push_back(response);
            } else {
                self.receive_overrun = true;
            }
        }
    }
}

/// Host-facing state for a functional RP2040 SPI controller.
#[derive(Clone)]
pub struct SpiHandle {
    state: Arc<Mutex<Rp2040SpiState>>,
}

impl Default for SpiHandle {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(Rp2040SpiState::reset())),
        }
    }
}

impl SpiHandle {
    /// Returns bytes written to the controller's data-register windows.
    pub fn transmitted(&self) -> Vec<u8> {
        self.state
            .lock()
            .expect("SPI lock poisoned")
            .transmitted
            .clone()
    }

    /// Queues bytes to be returned by subsequent data-register reads.
    pub fn queue_received(&self, bytes: &[u8]) {
        self.state
            .lock()
            .expect("SPI lock poisoned")
            .queued_input
            .extend(bytes.iter().map(|byte| u32::from(*byte)));
    }

    /// Returns the raw DW SSI interrupt conditions.
    pub fn raw_interrupts(&self) -> u32 {
        self.state
            .lock()
            .expect("SPI lock poisoned")
            .raw_interrupts()
    }

    /// Returns whether the masked interrupt output is asserted.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("SPI lock poisoned");
        state.raw_interrupts()
            & state.registers[Rp2040SpiRegister::Imr.offset() as usize / 4]
            & INTERRUPT_MASK
            != 0
    }

    /// Clears captured traffic and pending receive/input bytes.
    pub fn clear(&self) {
        let mut state = self.state.lock().expect("SPI lock poisoned");
        state.transmitted.clear();
        state.tx_fifo.clear();
        state.rx_fifo.clear();
        state.queued_input.clear();
        state.receive_overrun = false;
        state.receive_underflow = false;
        state.transmit_overflow = false;
    }
}

/// Deterministic functional model of the RP2040's DW_apb_ssi SPI0/SPI1 blocks.
///
/// Transfers complete immediately in abstract time. A host response is consumed first; if no
/// response is queued, the transmitted frame is looped back. The native sixteen-entry FIFO
/// depth and clear-on-read overflow/underflow status are preserved; serial clock waveforms, DMA
/// handshakes and pin muxing remain outside this functional slice.
///
/// Register offsets and reset values follow Raspberry Pi's generated [`ssi.h`](https://raw.githubusercontent.com/raspberrypi/pico-sdk/master/src/rp2040/hardware_regs/include/hardware/regs/ssi.h)
/// and the [RP2040 datasheet](https://datasheets.raspberrypi.com/rp2040/rp2040-datasheet.pdf).
pub struct FunctionalSpi {
    name: String,
    state: Arc<Mutex<Rp2040SpiState>>,
}

impl FunctionalSpi {
    /// Creates a reset RP2040 SSI controller and host handle.
    pub fn new(name: impl Into<String>) -> (Self, SpiHandle) {
        let state = Arc::new(Mutex::new(Rp2040SpiState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            SpiHandle { state },
        )
    }

    fn replicated_write(value: u64, width: AccessWidth) -> Result<u32, DeviceError> {
        match width {
            AccessWidth::Byte => {
                let value = u32::try_from(value & 0xff).expect("SPI byte value fits");
                Ok(value | (value << 8) | (value << 16) | (value << 24))
            }
            AccessWidth::HalfWord => {
                let value = u32::try_from(value & 0xffff).expect("SPI halfword value fits");
                Ok(value | (value << 16))
            }
            AccessWidth::Word => u32::try_from(value & u64::from(u32::MAX))
                .map_err(|_| DeviceError::new("RP2040 SPI value overflow")),
            AccessWidth::DoubleWord => Err(DeviceError::new(
                "RP2040 SPI does not support 64-bit access",
            )),
        }
    }

    fn read_lane(value: u32, offset: u64, width: AccessWidth) -> Result<u64, DeviceError> {
        match width {
            AccessWidth::Byte => Ok(u64::from((value >> ((offset & 3) * 8)) & 0xff)),
            AccessWidth::HalfWord => Ok(u64::from((value >> ((offset & 2) * 8)) & 0xffff)),
            AccessWidth::Word => Ok(u64::from(value)),
            AccessWidth::DoubleWord => Err(DeviceError::new(
                "RP2040 SPI does not support 64-bit access",
            )),
        }
    }

    fn atomic_update(current: u32, alias: u64, value: u32) -> u32 {
        match alias {
            0 => value,
            1 => current ^ value,
            2 => current | value,
            3 => current & !value,
            _ => unreachable!("RP2040 SPI atomic alias is two bits"),
        }
    }
}

impl Device for FunctionalSpi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width == AccessWidth::DoubleWord {
            return Err(DeviceError::new(
                "RP2040 SPI does not support 64-bit access",
            ));
        }
        let register_offset = (offset & 0x0fff) & !3;
        let register = Rp2040SpiRegister::from_offset(register_offset)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?;
        let mut state = self.state.lock().expect("SPI lock poisoned");
        let value = match register {
            Rp2040SpiRegister::Data(_) => match state.rx_fifo.pop_front() {
                Some(value) => value,
                None => {
                    // DW_apb_ssi latches RXUIR when firmware reads an empty RX FIFO.
                    state.receive_underflow = true;
                    0
                }
            },
            Rp2040SpiRegister::Txflr => state.tx_fifo.len() as u32,
            Rp2040SpiRegister::Rxflr => state.rx_fifo.len() as u32,
            Rp2040SpiRegister::Sr => state.status(),
            Rp2040SpiRegister::Isr => {
                state.raw_interrupts()
                    & state.registers[Rp2040SpiRegister::Imr.offset() as usize / 4]
                    & INTERRUPT_MASK
            }
            Rp2040SpiRegister::Risr => state.raw_interrupts(),
            Rp2040SpiRegister::Rxoicr => {
                let value = u32::from(state.receive_overrun);
                state.receive_overrun = false;
                value
            }
            Rp2040SpiRegister::Rxuicr => {
                let value = u32::from(state.receive_underflow);
                state.receive_underflow = false;
                value
            }
            Rp2040SpiRegister::Txoicr => {
                let value = u32::from(state.transmit_overflow);
                state.transmit_overflow = false;
                value
            }
            Rp2040SpiRegister::Msticr => 0,
            Rp2040SpiRegister::Icr => {
                let value = state.raw_interrupts();
                state.receive_overrun = false;
                state.receive_underflow = false;
                state.transmit_overflow = false;
                value
            }
            Rp2040SpiRegister::Ctrlr0
            | Rp2040SpiRegister::Ctrlr1
            | Rp2040SpiRegister::SsiEnr
            | Rp2040SpiRegister::Mwcr
            | Rp2040SpiRegister::Ser
            | Rp2040SpiRegister::Baudr
            | Rp2040SpiRegister::Txftlr
            | Rp2040SpiRegister::Rxftlr
            | Rp2040SpiRegister::Imr
            | Rp2040SpiRegister::Dmacr
            | Rp2040SpiRegister::Dmatdlr
            | Rp2040SpiRegister::Dmardlr
            | Rp2040SpiRegister::Idr
            | Rp2040SpiRegister::SsiVersionId
            | Rp2040SpiRegister::RxSampleDly
            | Rp2040SpiRegister::SpiCtrlr0
            | Rp2040SpiRegister::TxdDriveEdge => state.registers[register_offset as usize / 4],
        };
        Self::read_lane(value, offset, width)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let register_offset = (offset & 0x0fff) & !3;
        let alias = (offset >> 12) & 3;
        let register = Rp2040SpiRegister::from_offset(register_offset)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        let value = Self::replicated_write(value, width)?;
        let mut state = self.state.lock().expect("SPI lock poisoned");
        match register {
            Rp2040SpiRegister::Data(_) => {
                if alias != 0 {
                    return Err(DeviceError::new(
                        "RP2040 SPI atomic aliases are not valid for data registers",
                    ));
                }
                if state.tx_fifo.len() >= FIFO_DEPTH {
                    // A full DW_apb_ssi TX FIFO drops the APB write and latches TXOIR;
                    // it does not turn a peripheral status condition into a bus fault.
                    state.transmit_overflow = true;
                    return Ok(());
                }
                let mask = state.data_mask();
                state.tx_fifo.push_back(value & mask);
                state.process_tx();
            }
            Rp2040SpiRegister::Ctrlr0 => {
                let current = state.registers[register_offset as usize / 4];
                state.registers[register_offset as usize / 4] =
                    Self::atomic_update(current, alias, value) & CTRL0_MASK;
            }
            Rp2040SpiRegister::Ctrlr1 => {
                let current = state.registers[register_offset as usize / 4];
                state.registers[register_offset as usize / 4] =
                    Self::atomic_update(current, alias, value) & CTRL1_MASK;
            }
            Rp2040SpiRegister::SsiEnr => {
                let current = state.registers[register_offset as usize / 4];
                let next = Self::atomic_update(current, alias, value) & 1;
                state.registers[register_offset as usize / 4] = next;
                if next == 0 {
                    // The hardware clears both FIFOs whenever SSI_EN is deasserted.
                    state.tx_fifo.clear();
                    state.rx_fifo.clear();
                    state.receive_overrun = false;
                    state.receive_underflow = false;
                    state.transmit_overflow = false;
                }
                state.process_tx();
            }
            Rp2040SpiRegister::Mwcr => {
                let current = state.registers[register_offset as usize / 4];
                state.registers[register_offset as usize / 4] =
                    Self::atomic_update(current, alias, value) & 7;
            }
            Rp2040SpiRegister::Ser => {
                let current = state.registers[register_offset as usize / 4];
                state.registers[register_offset as usize / 4] =
                    Self::atomic_update(current, alias, value) & 1;
                state.process_tx();
            }
            Rp2040SpiRegister::Baudr => {
                let current = state.registers[register_offset as usize / 4];
                state.registers[register_offset as usize / 4] =
                    Self::atomic_update(current, alias, value) & 0xffff;
            }
            Rp2040SpiRegister::Txftlr
            | Rp2040SpiRegister::Rxftlr
            | Rp2040SpiRegister::Dmatdlr
            | Rp2040SpiRegister::Dmardlr
            | Rp2040SpiRegister::RxSampleDly => {
                let current = state.registers[register_offset as usize / 4];
                state.registers[register_offset as usize / 4] =
                    Self::atomic_update(current, alias, value) & 0xff;
            }
            Rp2040SpiRegister::Imr => {
                let current = state.registers[register_offset as usize / 4];
                state.registers[register_offset as usize / 4] =
                    Self::atomic_update(current, alias, value) & INTERRUPT_MASK;
            }
            Rp2040SpiRegister::Dmacr => {
                let current = state.registers[register_offset as usize / 4];
                state.registers[register_offset as usize / 4] =
                    Self::atomic_update(current, alias, value) & 3;
            }
            Rp2040SpiRegister::SpiCtrlr0 => {
                let current = state.registers[register_offset as usize / 4];
                state.registers[register_offset as usize / 4] =
                    Self::atomic_update(current, alias, value) & SPI_CTRL0_MASK;
            }
            Rp2040SpiRegister::TxdDriveEdge => {
                let current = state.registers[register_offset as usize / 4];
                state.registers[register_offset as usize / 4] =
                    Self::atomic_update(current, alias, value) & 0xff;
            }
            Rp2040SpiRegister::Txflr
            | Rp2040SpiRegister::Rxflr
            | Rp2040SpiRegister::Sr
            | Rp2040SpiRegister::Isr
            | Rp2040SpiRegister::Risr
            | Rp2040SpiRegister::Txoicr
            | Rp2040SpiRegister::Rxoicr
            | Rp2040SpiRegister::Rxuicr
            | Rp2040SpiRegister::Msticr
            | Rp2040SpiRegister::Icr
            | Rp2040SpiRegister::Idr
            | Rp2040SpiRegister::SsiVersionId => {
                return Err(DeviceError::new("RP2040 SPI register is read-only"));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("SPI lock poisoned") = Rp2040SpiState::reset();
    }
}
