use super::SignalHub;
use remu_bus::{Device, DeviceError, SharedMemory};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{SignalId, SignalValue};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const CR: u64 = 0x00;
const DCR: u64 = 0x04;
const SR: u64 = 0x08;
const FCR: u64 = 0x0c;
const DLR: u64 = 0x10;
const CCR: u64 = 0x14;
const AR: u64 = 0x18;
const ABR: u64 = 0x1c;
const DR: u64 = 0x20;
const PSMKR: u64 = 0x24;
const PSMAR: u64 = 0x28;
const PIR: u64 = 0x2c;
const LPTR: u64 = 0x30;

const CR_EN: u32 = 1 << 0;
const CR_ABORT: u32 = 1 << 1;
const CR_DMAEN: u32 = 1 << 2;
const CR_TCEN: u32 = 1 << 3;
const CR_SSHIFT: u32 = 1 << 4;
const CR_DFM: u32 = 1 << 6;
const CR_FSEL: u32 = 1 << 7;
const CR_FTHRES: u32 = 0xf << 8;
const CR_TEIE: u32 = 1 << 16;
const CR_TCIE: u32 = 1 << 17;
const CR_FTIE: u32 = 1 << 18;
const CR_SMIE: u32 = 1 << 19;
const CR_TOIE: u32 = 1 << 20;
const CR_APMS: u32 = 1 << 22;
const CR_PMM: u32 = 1 << 23;
const CR_PRESCALER: u32 = 0xff << 24;
const CR_PROGRAMMABLE_MASK: u32 = CR_EN
    | CR_DMAEN
    | CR_TCEN
    | CR_SSHIFT
    | CR_DFM
    | CR_FSEL
    | CR_FTHRES
    | CR_TEIE
    | CR_TCIE
    | CR_FTIE
    | CR_SMIE
    | CR_TOIE
    | CR_APMS
    | CR_PMM
    | CR_PRESCALER;

const SR_TEF: u32 = 1 << 0;
const SR_TCF: u32 = 1 << 1;
const SR_FTF: u32 = 1 << 2;
const SR_SMF: u32 = 1 << 3;
const SR_TOF: u32 = 1 << 4;
const SR_BUSY: u32 = 1 << 5;

const FCR_CTEF: u32 = 1 << 0;
const FCR_CTCF: u32 = 1 << 1;
const FCR_CSMF: u32 = 1 << 3;
const FCR_CTOF: u32 = 1 << 4;

const CCR_FMODE_MASK: u32 = 0x3 << 26;
const CCR_FMODE_INDIRECT_WRITE: u32 = 0;
const CCR_FMODE_INDIRECT_READ: u32 = 1 << 26;
const CCR_FMODE_AUTOMATIC_POLLING: u32 = 2 << 26;
const CCR_FMODE_MEMORY_MAPPED: u32 = 3 << 26;

const FLASH_WINDOW_SIZE: usize = 16 * 1024 * 1024;
const FIFO_SIZE: usize = 32;

/// Host-facing access to the STM32L432 external QUADSPI flash.
#[derive(Clone)]
pub struct Stm32QuadSpiHandle {
    state: Arc<Mutex<QuadSpiState>>,
}

impl Stm32QuadSpiHandle {
    /// Returns whether an enabled QUADSPI interrupt source is pending.
    pub fn interrupt_pending(&self) -> bool {
        self.state
            .lock()
            .expect("QUADSPI lock poisoned")
            .interrupt_pending()
    }

    /// Loads bytes into the external NOR flash backing store.
    pub fn load_flash(&self, offset: usize, bytes: &[u8]) -> bool {
        self.state
            .lock()
            .expect("QUADSPI lock poisoned")
            .flash
            .write_range(offset, bytes)
    }

    /// Returns a copy of the external NOR flash backing store.
    pub fn flash(&self) -> Vec<u8> {
        self.state
            .lock()
            .expect("QUADSPI lock poisoned")
            .flash
            .to_vec()
    }

    /// Returns whether firmware selected memory-mapped mode.
    pub fn memory_mapped(&self) -> bool {
        self.state
            .lock()
            .expect("QUADSPI lock poisoned")
            .memory_mapped
    }
}

struct QuadSpiState {
    registers: [u32; 13],
    flash: SharedMemory,
    fifo: VecDeque<u8>,
    cursor: usize,
    remaining: u64,
    busy: bool,
    memory_mapped: bool,
    raw_flags: u32,
    hub: SignalHub,
    irq_signal: SignalId,
    command_signal: SignalId,
    data_signal: SignalId,
    data_strobe_signal: SignalId,
    data_strobe: bool,
}

impl QuadSpiState {
    fn interrupt_pending(&self) -> bool {
        let enabled = self.registers[CR as usize / 4];
        (self.raw_flags & SR_TEF != 0 && enabled & CR_TEIE != 0)
            || (self.raw_flags & SR_TCF != 0 && enabled & CR_TCIE != 0)
            || (self.raw_flags & SR_FTF != 0 && enabled & CR_FTIE != 0)
            || (self.raw_flags & SR_SMF != 0 && enabled & CR_SMIE != 0)
            || (self.raw_flags & SR_TOF != 0 && enabled & CR_TOIE != 0)
    }

    fn set_signal(&self, signal: SignalId, value: u64, width: u16, at: SimTime) {
        self.hub
            .set(
                signal,
                SignalValue::from_u64(value, width).expect("fixed QUADSPI signal width"),
                at,
            )
            .expect("QUADSPI signal remains registered");
    }

    fn update_irq_signal(&self, at: SimTime) {
        self.set_signal(self.irq_signal, u64::from(self.interrupt_pending()), 1, at);
    }

    fn status(&self) -> u32 {
        let fifo_level = u32::try_from(self.fifo.len()).expect("QUADSPI FIFO fits u32");
        self.raw_flags | u32::from(self.busy) * SR_BUSY | (fifo_level << 8)
    }

    fn transfer_length(dlr: u32, flash_len: usize) -> u64 {
        // DLR is encoded as N-1. A max value is useful to firmware as an
        // open-ended read; bound it by the modeled flash rather than allowing
        // an unbounded host allocation.
        let requested = u64::from(dlr).saturating_add(1);
        requested.min(u64::try_from(flash_len).expect("flash size fits u64"))
    }

    fn begin_command(&mut self, at: SimTime) {
        self.fifo.clear();
        self.cursor = usize::try_from(self.registers[AR as usize / 4]).unwrap_or(usize::MAX);
        self.remaining = Self::transfer_length(self.registers[DLR as usize / 4], self.flash.len());
        self.busy = true;
        self.memory_mapped = false;
        self.raw_flags &= !(SR_TEF | SR_TCF | SR_FTF | SR_SMF | SR_TOF);
        self.set_signal(
            self.command_signal,
            u64::from(self.registers[CCR as usize / 4]),
            32,
            at,
        );
        match self.registers[CCR as usize / 4] & CCR_FMODE_MASK {
            CCR_FMODE_INDIRECT_READ => self.fill_fifo(),
            CCR_FMODE_AUTOMATIC_POLLING => {
                self.busy = false;
                self.raw_flags |= SR_SMF | SR_TCF;
            }
            CCR_FMODE_MEMORY_MAPPED => {
                self.busy = false;
                self.memory_mapped = true;
            }
            CCR_FMODE_INDIRECT_WRITE => {}
            _ => unreachable!("FMODE is two bits"),
        }
        self.update_irq_signal(at);
    }

    fn fill_fifo(&mut self) {
        if self.registers[CCR as usize / 4] & CCR_FMODE_MASK != CCR_FMODE_INDIRECT_READ {
            return;
        }
        while self.fifo.len() < FIFO_SIZE && self.remaining > 0 {
            let byte = self.flash.read_range(self.cursor, 1).map_or_else(
                || {
                    self.raw_flags |= SR_TEF;
                    0xff
                },
                |bytes| bytes[0],
            );
            self.fifo.push_back(byte);
            self.cursor = self.cursor.saturating_add(1);
            self.remaining = self.remaining.saturating_sub(1);
        }
        if !self.fifo.is_empty() {
            self.raw_flags |= SR_FTF;
        }
    }

    fn finish_if_done(&mut self) {
        if self.remaining == 0 && self.fifo.is_empty() {
            self.busy = false;
            self.raw_flags &= !SR_FTF;
            self.raw_flags |= SR_TCF;
        }
    }

    fn read_data(&mut self, width: AccessWidth, at: SimTime) -> u64 {
        self.fill_fifo();
        let mut value = 0_u64;
        for index in 0..usize::from(width.bytes()) {
            let byte = self.fifo.pop_front().unwrap_or(0xff);
            value |= u64::from(byte) << (index * 8);
        }
        self.set_signal(self.data_signal, value, 32, at);
        self.data_strobe = !self.data_strobe;
        self.set_signal(self.data_strobe_signal, u64::from(self.data_strobe), 1, at);
        self.raw_flags = if self.fifo.is_empty() {
            self.raw_flags & !SR_FTF
        } else {
            self.raw_flags | SR_FTF
        };
        self.finish_if_done();
        self.update_irq_signal(at);
        value
    }

    fn write_data(&mut self, width: AccessWidth, value: u64, at: SimTime) {
        for index in 0..usize::from(width.bytes()) {
            if self.remaining == 0 {
                break;
            }
            let byte = (value >> (index * 8)) as u8;
            if let Some(current) = self
                .flash
                .read_range(self.cursor, 1)
                .and_then(|bytes| bytes.first().copied())
            {
                // NOR programming may only change erased one bits to zero.
                let _ = self.flash.write_range(self.cursor, &[current & byte]);
            } else {
                self.raw_flags |= SR_TEF;
            }
            self.cursor = self.cursor.saturating_add(1);
            self.remaining = self.remaining.saturating_sub(1);
        }
        self.set_signal(self.data_signal, value, 32, at);
        self.data_strobe = !self.data_strobe;
        self.set_signal(self.data_strobe_signal, u64::from(self.data_strobe), 1, at);
        self.finish_if_done();
        self.update_irq_signal(at);
    }

    fn reset(&mut self, at: SimTime) {
        self.registers = [0; 13];
        self.fifo.clear();
        self.cursor = 0;
        self.remaining = 0;
        self.busy = false;
        self.memory_mapped = false;
        self.raw_flags = 0;
        self.data_strobe = false;
        self.set_signal(self.irq_signal, 0, 1, at);
        self.set_signal(self.command_signal, 0, 32, at);
        self.set_signal(self.data_signal, 0, 32, at);
        self.set_signal(self.data_strobe_signal, 0, 1, at);
    }
}

/// Functional STM32L432 QUADSPI controller and external NOR flash window.
pub struct Stm32QuadSpi {
    name: String,
    state: Arc<Mutex<QuadSpiState>>,
}

impl Stm32QuadSpi {
    /// Creates a controller with a deterministic erased external flash.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Stm32QuadSpiHandle, SharedMemory), remu_signals::SignalError> {
        let flash = SharedMemory::from_bytes(vec![0xff; FLASH_WINDOW_SIZE]);
        let irq_signal = hub.declare(
            "board.stm32l432kc.quadspi.irq",
            SignalValue::from_u64(0, 1)?,
            Some("enabled QUADSPI interrupt request".to_owned()),
        )?;
        let command_signal = hub.declare(
            "board.stm32l432kc.quadspi.command",
            SignalValue::from_u64(0, 32)?,
            Some("last QUADSPI communication configuration".to_owned()),
        )?;
        let data_signal = hub.declare(
            "board.stm32l432kc.quadspi.data",
            SignalValue::from_u64(0, 32)?,
            Some("last QUADSPI data word".to_owned()),
        )?;
        let data_strobe_signal = hub.declare(
            "board.stm32l432kc.quadspi.data_strobe",
            SignalValue::from_u64(0, 1)?,
            Some("toggles for QUADSPI data transfers".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(QuadSpiState {
            registers: [0; 13],
            flash: flash.clone(),
            fifo: VecDeque::new(),
            cursor: 0,
            remaining: 0,
            busy: false,
            memory_mapped: false,
            raw_flags: 0,
            hub,
            irq_signal,
            command_signal,
            data_signal,
            data_strobe_signal,
            data_strobe: false,
        }));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32QuadSpiHandle { state },
            flash,
        ))
    }
}

impl Device for Stm32QuadSpi {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let mut state = self.state.lock().expect("QUADSPI lock poisoned");
        if offset == DR {
            if !matches!(
                width,
                AccessWidth::Byte | AccessWidth::HalfWord | AccessWidth::Word
            ) {
                return Err(DeviceError::new(
                    "QUADSPI DR supports byte, half-word, or word reads",
                ));
            }
            return Ok(state.read_data(width, at));
        }
        if width != AccessWidth::Word {
            return Err(DeviceError::new(
                "STM32 QUADSPI registers require word accesses",
            ));
        }
        let index = usize::try_from(offset / 4).unwrap_or(usize::MAX);
        let value = match offset {
            CR | DCR | DLR | CCR | AR | ABR | PSMKR | PSMAR | PIR | LPTR => {
                state.registers.get(index).copied().unwrap_or_default()
            }
            SR => state.status(),
            FCR => 0,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled QUADSPI read at {offset:#x}"
                )));
            }
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("QUADSPI lock poisoned");
        if offset == DR {
            if !matches!(
                width,
                AccessWidth::Byte | AccessWidth::HalfWord | AccessWidth::Word
            ) {
                return Err(DeviceError::new(
                    "QUADSPI DR supports byte, half-word, or word writes",
                ));
            }
            if state.registers[CCR as usize / 4] & CCR_FMODE_MASK == CCR_FMODE_INDIRECT_WRITE {
                state.write_data(width, value, at);
            }
            return Ok(());
        }
        if width != AccessWidth::Word {
            return Err(DeviceError::new(
                "STM32 QUADSPI registers require word accesses",
            ));
        }
        let value = value as u32;
        match offset {
            CR => {
                let old = state.registers[CR as usize / 4];
                state.registers[CR as usize / 4] = value & CR_PROGRAMMABLE_MASK;
                if value & CR_ABORT != 0 {
                    state.busy = false;
                    state.fifo.clear();
                    state.remaining = 0;
                    state.raw_flags |= SR_TCF;
                }
                if old & CR_EN == 0 && value & CR_EN != 0 {
                    state.raw_flags &= !(SR_TEF | SR_TCF | SR_FTF | SR_SMF | SR_TOF);
                }
                state.update_irq_signal(at);
            }
            DCR | DLR | AR | ABR | PSMKR | PSMAR | PIR | LPTR => {
                state.registers[offset as usize / 4] = value;
            }
            CCR => {
                state.registers[CCR as usize / 4] = value;
                if state.registers[CR as usize / 4] & CR_EN != 0 {
                    state.begin_command(at);
                }
            }
            FCR => {
                let clear = (if value & FCR_CTEF != 0 { SR_TEF } else { 0 })
                    | (if value & FCR_CTCF != 0 { SR_TCF } else { 0 })
                    | (if value & FCR_CSMF != 0 { SR_SMF } else { 0 })
                    | (if value & FCR_CTOF != 0 { SR_TOF } else { 0 });
                state.raw_flags &= !clear;
                state.update_irq_signal(at);
            }
            SR => {
                return Err(DeviceError::new(
                    "QUADSPI SR is read-only; use FCR to clear flags",
                ));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled QUADSPI write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("QUADSPI lock poisoned")
            .reset(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::Device;

    fn controller() -> (Stm32QuadSpi, Stm32QuadSpiHandle, SharedMemory, SignalHub) {
        let hub = SignalHub::new();
        let (device, handle, flash) = Stm32QuadSpi::new("quadspi", hub.clone()).unwrap();
        (device, handle, flash, hub)
    }

    #[test]
    fn indirect_read_streams_nor_flash_and_completes() {
        let (mut qspi, handle, flash, _) = controller();
        assert!(flash.write_range(0x20, &[0x12, 0x34, 0x56, 0x78]));
        qspi.write(
            CR,
            AccessWidth::Word,
            u64::from(CR_EN | CR_TCIE),
            SimTime::ZERO,
        )
        .unwrap();
        qspi.write(DLR, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        qspi.write(AR, AccessWidth::Word, 0x20, SimTime::ZERO)
            .unwrap();
        qspi.write(
            CCR,
            AccessWidth::Word,
            u64::from(
                0x0b | (1 << 8) | (1 << 10) | (2 << 12) | (1 << 24) | CCR_FMODE_INDIRECT_READ,
            ),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            qspi.read(DR, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x7856_3412
        );
        assert_ne!(
            qspi.read(SR, AccessWidth::Word, SimTime::ZERO).unwrap() & u64::from(SR_TCF),
            0
        );
        assert!(handle.interrupt_pending());
        qspi.write(FCR, AccessWidth::Word, u64::from(FCR_CTCF), SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn indirect_write_obeys_nor_one_to_zero_programming() {
        let (mut qspi, _, flash, _) = controller();
        assert!(flash.write_range(0x40, &[0xf0, 0x0f]));
        qspi.write(CR, AccessWidth::Word, u64::from(CR_EN), SimTime::ZERO)
            .unwrap();
        qspi.write(DLR, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        qspi.write(AR, AccessWidth::Word, 0x40, SimTime::ZERO)
            .unwrap();
        qspi.write(
            CCR,
            AccessWidth::Word,
            u64::from(0x02 | CCR_FMODE_INDIRECT_WRITE),
            SimTime::ZERO,
        )
        .unwrap();
        qspi.write(DR, AccessWidth::HalfWord, 0x33cc, SimTime::ZERO)
            .unwrap();
        assert_eq!(flash.read_range(0x40, 2).unwrap(), &[0xc0, 0x03]);
    }

    #[test]
    fn memory_mapped_mode_selects_shared_flash_window() {
        let (mut qspi, handle, flash, _) = controller();
        assert!(flash.write_range(0x100, &[0xaa, 0xbb, 0xcc, 0xdd]));
        qspi.write(CR, AccessWidth::Word, u64::from(CR_EN), SimTime::ZERO)
            .unwrap();
        qspi.write(AR, AccessWidth::Word, 0x100, SimTime::ZERO)
            .unwrap();
        qspi.write(
            CCR,
            AccessWidth::Word,
            u64::from(
                0x0b | (1 << 8) | (1 << 10) | (2 << 12) | (1 << 24) | CCR_FMODE_MEMORY_MAPPED,
            ),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.memory_mapped());
        assert_eq!(
            flash.read_range(0x100, 4).unwrap(),
            &[0xaa, 0xbb, 0xcc, 0xdd]
        );
    }
}
