use super::*;
use remu_core::{AccessKind, Bus};

const CHANNELS: usize = 7;
const CHANNEL_STRIDE: u64 = 0x14;
const DMA_ENABLE: u32 = 1 << 0;
const DMA_TCIE: u32 = 1 << 1;
const DMA_HTIE: u32 = 1 << 2;
const DMA_TEIE: u32 = 1 << 3;
const DMA_DIR: u32 = 1 << 4;
const DMA_CIRC: u32 = 1 << 5;
const DMA_PINC: u32 = 1 << 6;
const DMA_MINC: u32 = 1 << 7;
const DMA_PSIZE: u32 = 0b11 << 8;
const DMA_MSIZE: u32 = 0b11 << 10;
const DMA_PL: u32 = 0b11 << 12;
const DMA_MEM2MEM: u32 = 1 << 14;
const DMA_SUPPORTED: u32 = DMA_ENABLE
    | DMA_TCIE
    | DMA_HTIE
    | DMA_TEIE
    | DMA_DIR
    | DMA_CIRC
    | DMA_PINC
    | DMA_MINC
    | DMA_PSIZE
    | DMA_MSIZE
    | DMA_PL
    | DMA_MEM2MEM;

#[derive(Clone, Copy, Default)]
struct DmaChannel {
    cfgr: u32,
    cntr: u16,
    initial_cntr: u16,
    paddr: u32,
    maddr: u32,
    initial_paddr: u32,
    initial_maddr: u32,
}

struct WchDmaState {
    channels: [DmaChannel; CHANNELS],
    intfr: u32,
}

/// Host-facing handle for deterministic WCH DMA progress and interrupts.
#[derive(Clone)]
pub struct WchDmaHandle {
    state: Arc<Mutex<WchDmaState>>,
}

impl WchDmaHandle {
    /// Copies one transfer unit for each enabled channel through the guest bus.
    pub fn service(&self, bus: &mut dyn Bus, at: SimTime) -> Result<usize, DeviceError> {
        let mut completed = 0;
        for index in 0..CHANNELS {
            let Some((paddr, maddr, pwidth, mwidth, direction, pinc, minc, mem2mem, circular)) = ({
                let state = self.state.lock().expect("WCH DMA lock poisoned");
                let channel = state.channels[index];
                if channel.cfgr & DMA_ENABLE == 0 || channel.cntr == 0 {
                    None
                } else {
                    Some((
                        u64::from(channel.paddr),
                        u64::from(channel.maddr),
                        Self::width(channel.cfgr, DMA_PSIZE),
                        Self::width(channel.cfgr, DMA_MSIZE),
                        channel.cfgr & DMA_DIR != 0,
                        channel.cfgr & DMA_PINC != 0,
                        channel.cfgr & DMA_MINC != 0,
                        channel.cfgr & DMA_MEM2MEM != 0,
                        channel.cfgr & DMA_CIRC != 0,
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
            let value = bus
                .read(source, source_width, AccessKind::Read, at)
                .map_err(|error| {
                    self.mark_error(index);
                    DeviceError::new(format!("WCH DMA channel {index} read: {error}"))
                })?;
            let value = match destination_width {
                AccessWidth::Byte => value & u64::from(u8::MAX),
                AccessWidth::HalfWord => value & u64::from(u16::MAX),
                AccessWidth::Word => value & u64::from(u32::MAX),
                AccessWidth::DoubleWord => value,
            };
            if let Err(error) = bus.write(destination, destination_width, value, at) {
                self.mark_error(index);
                return Err(DeviceError::new(format!(
                    "WCH DMA channel {index} write: {error}"
                )));
            }
            let source_step = u32::from(source_width.bytes());
            let destination_step = u32::from(destination_width.bytes());
            let mut state = self.state.lock().expect("WCH DMA lock poisoned");
            let (remaining, halfway, initial_count) = {
                let channel = &mut state.channels[index];
                channel.cntr = channel.cntr.saturating_sub(1);
                if pinc {
                    channel.paddr = channel.paddr.wrapping_add(if direction {
                        destination_step
                    } else {
                        source_step
                    });
                }
                if minc {
                    channel.maddr = channel.maddr.wrapping_add(if direction {
                        source_step
                    } else {
                        destination_step
                    });
                }
                (channel.cntr, channel.initial_cntr / 2, channel.initial_cntr)
            };
            let flag_base = index * 4;
            if remaining == halfway && initial_count > 1 {
                state.intfr |= 1 << flag_base;
                state.intfr |= 1 << (flag_base + 2);
            }
            if remaining == 0 {
                state.intfr |= 1 << flag_base;
                state.intfr |= 1 << (flag_base + 1);
                if circular && !mem2mem {
                    let channel = &mut state.channels[index];
                    channel.cntr = channel.initial_cntr;
                    channel.paddr = channel.initial_paddr;
                    channel.maddr = channel.initial_maddr;
                } else {
                    let channel = &mut state.channels[index];
                    channel.cfgr &= !DMA_ENABLE;
                }
            }
            completed += 1;
        }
        Ok(completed)
    }

    /// Returns true when a channel has an enabled pending interrupt flag.
    pub fn channel_pending(&self, index: usize) -> bool {
        let state = self.state.lock().expect("WCH DMA lock poisoned");
        let Some(channel) = state.channels.get(index) else {
            return false;
        };
        let flags = (state.intfr >> (index * 4)) & 0xf;
        flags & ((channel.cfgr >> 1) & 0x7) != 0
    }

    fn width(control: u32, mask: u32) -> AccessWidth {
        match (control & mask) >> mask.trailing_zeros() {
            0 => AccessWidth::Byte,
            1 => AccessWidth::HalfWord,
            _ => AccessWidth::Word,
        }
    }

    fn mark_error(&self, index: usize) {
        let mut state = self.state.lock().expect("WCH DMA lock poisoned");
        let flag_base = index * 4;
        state.intfr |= (1 << flag_base) | (1 << (flag_base + 3));
        state.channels[index].cfgr &= !DMA_ENABLE;
    }
}

/// Functional seven-channel WCH DMA controller.
pub struct WchDma {
    name: String,
    state: Arc<Mutex<WchDmaState>>,
}

impl WchDma {
    /// Creates a reset controller and service handle.
    pub fn new(name: impl Into<String>) -> (Self, WchDmaHandle) {
        let state = Arc::new(Mutex::new(WchDmaState {
            channels: [DmaChannel::default(); CHANNELS],
            intfr: 0,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            WchDmaHandle { state },
        )
    }

    fn channel_index(offset: u64) -> Option<(usize, u64)> {
        let relative = offset.checked_sub(0x08)?;
        if relative < CHANNEL_STRIDE * CHANNELS as u64 {
            Some((
                usize::try_from(relative / CHANNEL_STRIDE).ok()?,
                relative % CHANNEL_STRIDE,
            ))
        } else {
            None
        }
    }

    fn require_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("WCH DMA requires aligned word accesses"));
        }
        Ok(())
    }
}

impl Device for WchDma {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_access(offset, width)?;
        let state = self.state.lock().expect("WCH DMA lock poisoned");
        if let Some((index, register)) = Self::channel_index(offset) {
            let channel = &state.channels[index];
            let value = match register {
                0x00 => channel.cfgr,
                0x04 => u32::from(channel.cntr),
                0x08 => channel.paddr,
                0x0c => channel.maddr,
                _ => 0,
            };
            return Ok(u64::from(value));
        }
        let value = match offset {
            0x00 => state.intfr,
            0x04 => 0,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} read outside modeled registers at offset {offset:#x}",
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
        let mut state = self.state.lock().expect("WCH DMA lock poisoned");
        if let Some((index, register)) = Self::channel_index(offset) {
            let channel = &mut state.channels[index];
            match register {
                0x00 => {
                    let was_enabled = channel.cfgr & DMA_ENABLE != 0;
                    channel.cfgr = value & DMA_SUPPORTED;
                    if !was_enabled && channel.cfgr & DMA_ENABLE != 0 {
                        channel.initial_cntr = channel.cntr;
                        channel.initial_paddr = channel.paddr;
                        channel.initial_maddr = channel.maddr;
                    }
                }
                0x04 => {
                    if channel.cfgr & DMA_ENABLE == 0 {
                        channel.cntr = u16::try_from(value & u32::from(u16::MAX))
                            .expect("masked WCH DMA count fits u16");
                        channel.initial_cntr = channel.cntr;
                    }
                }
                0x08 => {
                    if channel.cfgr & DMA_ENABLE == 0 {
                        channel.paddr = value;
                        channel.initial_paddr = value;
                    }
                }
                0x0c => {
                    if channel.cfgr & DMA_ENABLE == 0 {
                        channel.maddr = value;
                        channel.initial_maddr = value;
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        match offset {
            0x00 => {}
            0x04 => {
                for index in 0..CHANNELS {
                    let base = index * 4;
                    let mask = (value >> base) & 0xf;
                    if mask & 1 != 0 {
                        state.intfr &= !(0xf << base);
                    } else {
                        state.intfr &= !(mask << base);
                    }
                }
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "{} write outside modeled registers at offset {offset:#x}",
                    self.name
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("WCH DMA lock poisoned");
        state.channels = [DmaChannel::default(); CHANNELS];
        state.intfr = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::{AddressSpace, Endianness};

    #[test]
    fn channel_copies_a_word_and_reports_completion() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2000_0000, 0x100, true).unwrap();
        let (dma, handle) = WchDma::new("dma");
        bus.map_device("dma", 0x4002_0000, 0x100, Box::new(dma))
            .unwrap();
        bus.write(0x2000_0000, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_0010, AccessWidth::Word, 0x2000_0000, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_0014, AccessWidth::Word, 0x2000_0004, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_000c, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        bus.write(
            0x4002_0008,
            AccessWidth::Word,
            u64::from(DMA_ENABLE | DMA_TCIE | DMA_MINC | DMA_PSIZE | DMA_MSIZE),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.service(&mut bus, SimTime::ZERO).unwrap(), 1);
        assert_eq!(
            bus.read(
                0x2000_0004,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            0x1234_5678
        );
        assert!(handle.channel_pending(0));
    }

    #[test]
    fn transfer_flags_are_raised_without_interrupt_enables_and_clear_as_documented() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2000_0000, 0x100, true).unwrap();
        let (dma, handle) = WchDma::new("dma");
        bus.map_device("dma", 0x4002_0000, 0x100, Box::new(dma))
            .unwrap();
        bus.write(0x2000_0000, AccessWidth::Word, 0x0000_2211, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_0010, AccessWidth::Word, 0x2000_0000, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_0014, AccessWidth::Word, 0x2000_0004, SimTime::ZERO)
            .unwrap();
        bus.write(0x4002_000c, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        bus.write(
            0x4002_0008,
            AccessWidth::Word,
            u64::from(DMA_ENABLE | DMA_PINC | DMA_MINC),
            SimTime::ZERO,
        )
        .unwrap();
        handle.service(&mut bus, SimTime::ZERO).unwrap();
        handle.service(&mut bus, SimTime::ZERO).unwrap();
        assert_eq!(
            bus.read(
                0x4002_0000,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap()
                & 0x7,
            0x7
        );
        bus.write(0x4002_0004, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            bus.read(
                0x4002_0000,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            0
        );
    }

    #[test]
    fn different_transfer_widths_zero_extend_and_use_independent_steps() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2000_0000, 0x100, true).unwrap();
        let (dma, handle) = WchDma::new("dma");
        bus.map_device("dma", 0x4002_0000, 0x100, Box::new(dma))
            .unwrap();
        bus.write(0x2000_0000, AccessWidth::Word, 0x0000_2211, SimTime::ZERO)
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
            u64::from(DMA_ENABLE | DMA_PINC | DMA_MINC | (2 << 10)),
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
            0x11
        );
        assert_eq!(
            bus.read(
                0x2000_000c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            0x22
        );
    }
}
