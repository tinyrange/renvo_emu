use super::*;
use remu_core::{AccessKind, Bus};

const CHANNELS: usize = 7;
const CHANNEL_STRIDE: u64 = 0x14;
const CHANNEL_BASE: u64 = 0x08;
const CSELR_OFFSET: u64 = 0xa8;

const CCR_ENABLE: u32 = 1 << 0;
const CCR_TCIE: u32 = 1 << 1;
const CCR_HTIE: u32 = 1 << 2;
const CCR_TEIE: u32 = 1 << 3;
const CCR_DIR: u32 = 1 << 4;
const CCR_CIRC: u32 = 1 << 5;
const CCR_PINC: u32 = 1 << 6;
const CCR_MINC: u32 = 1 << 7;
const CCR_PSIZE: u32 = 0b11 << 8;
const CCR_MSIZE: u32 = 0b11 << 10;
const CCR_PINCOS: u32 = 1 << 15;
const CCR_MEM2MEM: u32 = 1 << 14;
const CCR_SUPPORTED: u32 = CCR_ENABLE
    | CCR_TCIE
    | CCR_HTIE
    | CCR_TEIE
    | CCR_DIR
    | CCR_CIRC
    | CCR_PINC
    | CCR_MINC
    | CCR_PSIZE
    | CCR_MSIZE
    | CCR_PINCOS
    | CCR_MEM2MEM;

const ISR_FLAGS_PER_CHANNEL: u32 = 4;
const ISR_MASK: u32 = (1 << (CHANNELS as u32 * ISR_FLAGS_PER_CHANNEL)) - 1;

/// Named register identifiers for the STM32L4 DMA block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stm32DmaRegister {
    /// Channel configuration register.
    Ccr,
    /// Channel transfer count register.
    Cndtr,
    /// Channel peripheral address register.
    Cpar,
    /// Channel memory address register.
    Cmar,
    /// Interrupt status register.
    Isr,
    /// Interrupt flag clear register.
    Ifcr,
    /// Channel request selection register.
    Cselr,
}

#[derive(Clone, Copy, Default)]
struct DmaChannel {
    ccr: u32,
    cndtr: u16,
    initial_cndtr: u16,
    cpar: u32,
    cmar: u32,
    initial_cpar: u32,
    initial_cmar: u32,
}

struct Stm32DmaState {
    channels: [DmaChannel; CHANNELS],
    isr: u32,
    cselr: u32,
}

/// Host-facing handle for deterministic STM32 DMA progress and interrupts.
#[derive(Clone)]
pub struct Stm32DmaHandle {
    state: Arc<Mutex<Stm32DmaState>>,
}

impl Stm32DmaHandle {
    /// Copies one transfer unit for every enabled channel through the guest bus.
    ///
    /// A functional run services at most one unit per channel per call. This
    /// gives firmware tests deterministic progress without pretending to model
    /// the DMA bus arbitration or peripheral request timing.
    pub fn service(&self, bus: &mut dyn Bus, at: SimTime) -> Result<usize, DeviceError> {
        let mut completed = 0;
        for index in 0..CHANNELS {
            let Some((paddr, maddr, pwidth, mwidth, direction, pinc, minc)) = ({
                let state = self.state.lock().expect("STM32 DMA lock poisoned");
                let channel = state.channels[index];
                if channel.ccr & CCR_ENABLE == 0 || channel.cndtr == 0 {
                    None
                } else {
                    Some((
                        u64::from(channel.cpar),
                        u64::from(channel.cmar),
                        Self::width(channel.ccr, CCR_PSIZE),
                        Self::width(channel.ccr, CCR_MSIZE),
                        channel.ccr & CCR_DIR != 0,
                        channel.ccr & CCR_PINC != 0,
                        channel.ccr & CCR_MINC != 0,
                    ))
                }
            }) else {
                continue;
            };

            let (source, source_width, destination, destination_width) = if direction {
                (maddr, mwidth, paddr, pwidth)
            } else {
                (paddr, pwidth, maddr, mwidth)
            };
            if source_width != destination_width {
                self.mark_error(index);
                continue;
            }

            let value = bus
                .read(source, source_width, AccessKind::Read, at)
                .map_err(|error| {
                    self.mark_error(index);
                    DeviceError::new(format!("STM32 DMA channel {index} read: {error}"))
                })?;
            if let Err(error) = bus.write(destination, destination_width, value, at) {
                self.mark_error(index);
                return Err(DeviceError::new(format!(
                    "STM32 DMA channel {index} write: {error}"
                )));
            }

            let step = u32::from(source_width.bytes());
            let mut state = self.state.lock().expect("STM32 DMA lock poisoned");
            let (remaining, halfway, control) = {
                let channel = &mut state.channels[index];
                channel.cndtr = channel.cndtr.saturating_sub(1);
                if direction {
                    if minc {
                        channel.cmar = channel.cmar.wrapping_add(step);
                    }
                    if pinc {
                        channel.cpar = channel.cpar.wrapping_add(step);
                    }
                } else {
                    if pinc {
                        channel.cpar = channel.cpar.wrapping_add(step);
                    }
                    if minc {
                        channel.cmar = channel.cmar.wrapping_add(step);
                    }
                }
                (channel.cndtr, channel.initial_cndtr / 2, channel.ccr)
            };
            let base = Self::flag_base(index);
            if remaining == halfway && halfway != 0 && control & CCR_HTIE != 0 {
                state.isr |= 1 << (base + 2);
            }
            if remaining == 0 {
                state.isr |= 1 << base;
                state.isr |= 1 << (base + 1);
                if control & CCR_CIRC != 0 {
                    let channel = &mut state.channels[index];
                    channel.cndtr = channel.initial_cndtr;
                    channel.cpar = channel.initial_cpar;
                    channel.cmar = channel.initial_cmar;
                } else {
                    state.channels[index].ccr &= !CCR_ENABLE;
                }
            }
            completed += 1;
        }
        Ok(completed)
    }

    /// Returns true when an enabled channel has a pending interrupt flag.
    pub fn channel_pending(&self, index: usize) -> bool {
        let state = self.state.lock().expect("STM32 DMA lock poisoned");
        let Some(channel) = state.channels.get(index) else {
            return false;
        };
        let flags = (state.isr >> Self::flag_base(index)) & 0xf;
        let enabled = ((channel.ccr & CCR_TCIE != 0) as u32) << 1
            | ((channel.ccr & CCR_HTIE != 0) as u32) << 2
            | ((channel.ccr & CCR_TEIE != 0) as u32) << 3;
        flags & enabled != 0
    }

    /// Returns the selected request number from the STM32L4 CSELR request mux.
    pub fn request_selection(&self, index: usize) -> Option<u8> {
        if index >= CHANNELS {
            return None;
        }
        let state = self.state.lock().expect("STM32 DMA lock poisoned");
        Some(((state.cselr >> (index * 4)) & 0xf) as u8)
    }

    /// Returns whether any channel currently has the enable bit set.
    pub fn active(&self) -> bool {
        let state = self.state.lock().expect("STM32 DMA lock poisoned");
        state
            .channels
            .iter()
            .any(|channel| channel.ccr & CCR_ENABLE != 0)
    }

    fn flag_base(index: usize) -> u32 {
        u32::try_from(index).expect("DMA channel index fits u32") * ISR_FLAGS_PER_CHANNEL
    }

    fn width(control: u32, mask: u32) -> AccessWidth {
        match (control & mask) >> mask.trailing_zeros() {
            0 => AccessWidth::Byte,
            1 => AccessWidth::HalfWord,
            _ => AccessWidth::Word,
        }
    }

    fn mark_error(&self, index: usize) {
        let mut state = self.state.lock().expect("STM32 DMA lock poisoned");
        let base = Self::flag_base(index);
        state.isr |= 1 << base;
        state.isr |= 1 << (base + 3);
        state.channels[index].ccr &= !CCR_ENABLE;
    }
}

/// Functional STM32L4 DMA1/DMA2 controller with CSELR request selection.
pub struct Stm32Dma {
    name: String,
    state: Arc<Mutex<Stm32DmaState>>,
}

impl Stm32Dma {
    /// Creates a reset seven-channel controller and service handle.
    pub fn new(name: impl Into<String>) -> (Self, Stm32DmaHandle) {
        let state = Arc::new(Mutex::new(Stm32DmaState {
            channels: [DmaChannel::default(); CHANNELS],
            isr: 0,
            cselr: 0,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32DmaHandle { state },
        )
    }

    fn decode(offset: u64) -> Option<(Stm32DmaRegister, Option<usize>)> {
        if offset == 0 {
            return Some((Stm32DmaRegister::Isr, None));
        }
        if offset == 4 {
            return Some((Stm32DmaRegister::Ifcr, None));
        }
        if offset == CSELR_OFFSET {
            return Some((Stm32DmaRegister::Cselr, None));
        }
        let relative = offset.checked_sub(CHANNEL_BASE)?;
        if relative >= CHANNEL_STRIDE * CHANNELS as u64 {
            return None;
        }
        let index = usize::try_from(relative / CHANNEL_STRIDE).ok()?;
        let register = match relative % CHANNEL_STRIDE {
            0x00 => Stm32DmaRegister::Ccr,
            0x04 => Stm32DmaRegister::Cndtr,
            0x08 => Stm32DmaRegister::Cpar,
            0x0c => Stm32DmaRegister::Cmar,
            _ => return None,
        };
        Some((register, Some(index)))
    }

    fn require_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("STM32 DMA requires aligned word accesses"));
        }
        Ok(())
    }
}

impl Device for Stm32Dma {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_access(offset, width)?;
        let (register, channel) = Self::decode(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} read outside registers at {offset:#x}",
                self.name
            ))
        })?;
        let state = self.state.lock().expect("STM32 DMA lock poisoned");
        let value = match (register, channel) {
            (Stm32DmaRegister::Isr, None) => state.isr,
            (Stm32DmaRegister::Ifcr, None) => 0,
            (Stm32DmaRegister::Cselr, None) => state.cselr,
            (Stm32DmaRegister::Ccr, Some(index)) => state.channels[index].ccr,
            (Stm32DmaRegister::Cndtr, Some(index)) => u32::from(state.channels[index].cndtr),
            (Stm32DmaRegister::Cpar, Some(index)) => state.channels[index].cpar,
            (Stm32DmaRegister::Cmar, Some(index)) => state.channels[index].cmar,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} invalid register decode at {offset:#x}",
                    self.name
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
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        Self::require_access(offset, width)?;
        let value = u32::try_from(value).expect("word access value fits u32");
        let (register, channel) = Self::decode(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write outside registers at {offset:#x}",
                self.name
            ))
        })?;
        let mut state = self.state.lock().expect("STM32 DMA lock poisoned");
        match (register, channel) {
            (Stm32DmaRegister::Isr, None) => {
                return Err(DeviceError::new(format!(
                    "{} ISR is read-only; use IFCR to clear flags",
                    self.name
                )));
            }
            (Stm32DmaRegister::Ifcr, None) => state.isr &= !(value & ISR_MASK),
            (Stm32DmaRegister::Cselr, None) => state.cselr = value & 0x0fff_ffff,
            (Stm32DmaRegister::Ccr, Some(index)) => {
                let channel = &mut state.channels[index];
                let was_enabled = channel.ccr & CCR_ENABLE != 0;
                channel.ccr = value & CCR_SUPPORTED;
                if !was_enabled && channel.ccr & CCR_ENABLE != 0 {
                    channel.initial_cndtr = channel.cndtr;
                    channel.initial_cpar = channel.cpar;
                    channel.initial_cmar = channel.cmar;
                }
            }
            (Stm32DmaRegister::Cndtr, Some(index)) => {
                let channel = &mut state.channels[index];
                channel.cndtr = u16::try_from(value & u32::from(u16::MAX))
                    .expect("STM32 DMA transfer count fits u16");
                if channel.ccr & CCR_ENABLE == 0 {
                    channel.initial_cndtr = channel.cndtr;
                }
            }
            (Stm32DmaRegister::Cpar, Some(index)) => {
                let channel = &mut state.channels[index];
                channel.cpar = value;
                if channel.ccr & CCR_ENABLE == 0 {
                    channel.initial_cpar = value;
                }
            }
            (Stm32DmaRegister::Cmar, Some(index)) => {
                let channel = &mut state.channels[index];
                channel.cmar = value;
                if channel.ccr & CCR_ENABLE == 0 {
                    channel.initial_cmar = value;
                }
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "{} invalid register decode at {offset:#x}",
                    self.name
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("STM32 DMA lock poisoned");
        state.channels = [DmaChannel::default(); CHANNELS];
        state.isr = 0;
        state.cselr = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::{AddressSpace, Endianness};

    #[test]
    fn memory_to_memory_transfer_sets_half_and_complete_flags() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2000_0000, 0x100, true).unwrap();
        let (dma, handle) = Stm32Dma::new("dma1");
        bus.map_device("dma", 0x4002_0000, 0x100, Box::new(dma))
            .unwrap();
        bus.write(0x2000_0000, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        bus.write(0x2000_0004, AccessWidth::Word, 0x9abc_def0, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_0010, AccessWidth::Word, 0x2000_0000, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_0014, AccessWidth::Word, 0x2000_0008, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_000c, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        bus.write(
            0x4002_0008,
            AccessWidth::Word,
            u64::from(
                CCR_ENABLE
                    | CCR_TCIE
                    | CCR_HTIE
                    | CCR_PINC
                    | CCR_MINC
                    | CCR_MEM2MEM
                    | (2 << 8)
                    | (2 << 10),
            ),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.service(&mut bus, SimTime::ZERO).unwrap(), 1);
        assert_eq!(handle.service(&mut bus, SimTime::ZERO).unwrap(), 1);
        assert_eq!(
            bus.read(
                0x2000_0008,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            0x1234_5678
        );
        assert_eq!(
            bus.read(
                0x2000_000c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            0x9abc_def0
        );
        let flags = bus
            .read(
                0x4002_0000,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(flags & 0x7, 0x7);
        assert!(handle.channel_pending(0));
        bus.write(0x4002_0004, AccessWidth::Word, 0x7, SimTime::ZERO)
            .unwrap();
        assert!(!handle.channel_pending(0));
    }

    #[test]
    fn cselr_selects_requests_and_width_mismatch_sets_error() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2000_0000, 0x100, true).unwrap();
        let (dma, handle) = Stm32Dma::new("dma2");
        bus.map_device("dma", 0x4002_0400, 0x100, Box::new(dma))
            .unwrap();
        bus.write(0x4002_04a8, AccessWidth::Word, 0x0a5, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.request_selection(0), Some(5));
        assert_eq!(handle.request_selection(1), Some(10));
        bus.write(0x4002_0410, AccessWidth::Word, 0x2000_0000, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_0414, AccessWidth::Word, 0x2000_0004, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_040c, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        bus.write(
            0x4002_0408,
            AccessWidth::Word,
            u64::from(CCR_ENABLE | CCR_TEIE | (1 << 8)),
            SimTime::ZERO,
        )
        .unwrap();
        handle.service(&mut bus, SimTime::ZERO).unwrap();
        assert!(handle.channel_pending(0));
        let flags = bus
            .read(
                0x4002_0400,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(flags & 0xf, 0x9);
    }
}
