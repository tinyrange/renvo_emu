use super::*;
use remu_core::{AccessKind, Bus};

const CHANNELS: usize = 12;
const CHANNEL_STRIDE: u64 = 0x40;
const CTRL_EN: u32 = 1 << 0;
const CTRL_DATA_SIZE_MASK: u32 = 0x3 << 2;
const CTRL_INCR_READ: u32 = 1 << 4;
const CTRL_INCR_WRITE: u32 = 1 << 5;
const CTRL_IRQ_QUIET: u32 = 1 << 21;
const CTRL_BUSY: u32 = 1 << 24;
const CTRL_WRITE_ERROR: u32 = 1 << 29;

#[derive(Clone, Copy)]
struct DmaChannel {
    read_addr: u32,
    write_addr: u32,
    transfer_count: u32,
    control: u32,
    write_error: bool,
}

impl Default for DmaChannel {
    fn default() -> Self {
        Self {
            read_addr: 0,
            write_addr: 0,
            transfer_count: 0,
            control: 0,
            write_error: false,
        }
    }
}

struct RpDmaState {
    channels: [DmaChannel; CHANNELS],
    raw_interrupt: u32,
    interrupt_enable: u32,
    force_interrupt: u32,
}

/// Host-facing handle for deterministic RP DMA progress and interrupt state.
#[derive(Clone)]
pub struct RpDmaHandle {
    state: Arc<Mutex<RpDmaState>>,
}

impl RpDmaHandle {
    /// Copies one transfer unit for each enabled channel.
    pub fn service(&self, bus: &mut dyn Bus, at: SimTime) -> Result<usize, DeviceError> {
        let mut completed = 0;
        for index in 0..CHANNELS {
            let Some((read_addr, write_addr, width, increment_read, increment_write)) = ({
                let state = self.state.lock().expect("RP DMA lock poisoned");
                let channel = state.channels[index];
                if channel.control & CTRL_EN == 0 || channel.transfer_count == 0 {
                    None
                } else {
                    let width = match (channel.control & CTRL_DATA_SIZE_MASK) >> 2 {
                        0 => AccessWidth::Byte,
                        1 => AccessWidth::HalfWord,
                        _ => AccessWidth::Word,
                    };
                    Some((
                        u64::from(channel.read_addr),
                        u64::from(channel.write_addr),
                        width,
                        channel.control & CTRL_INCR_READ != 0,
                        channel.control & CTRL_INCR_WRITE != 0,
                    ))
                }
            }) else {
                continue;
            };
            let value = bus
                .read(read_addr, width, AccessKind::Read, at)
                .map_err(|error| {
                    DeviceError::new(format!("RP DMA channel {index} read: {error}"))
                })?;
            if let Err(error) = bus.write(write_addr, width, value, at) {
                let mut state = self.state.lock().expect("RP DMA lock poisoned");
                state.channels[index].write_error = true;
                state.channels[index].control &= !CTRL_EN;
                return Err(DeviceError::new(format!(
                    "RP DMA channel {index} write: {error}"
                )));
            }
            let step = width.bytes() as u32;
            let mut state = self.state.lock().expect("RP DMA lock poisoned");
            let channel = &mut state.channels[index];
            channel.transfer_count = channel.transfer_count.saturating_sub(1);
            if increment_read {
                channel.read_addr = channel.read_addr.wrapping_add(step);
            }
            if increment_write {
                channel.write_addr = channel.write_addr.wrapping_add(step);
            }
            if channel.transfer_count == 0 {
                channel.control &= !CTRL_EN;
                if channel.control & CTRL_IRQ_QUIET == 0 {
                    state.raw_interrupt |= 1 << index;
                }
            }
            completed += 1;
        }
        Ok(completed)
    }

    /// Returns the currently visible channel interrupt mask.
    pub fn pending(&self) -> u32 {
        let state = self.state.lock().expect("RP DMA lock poisoned");
        (state.raw_interrupt & state.interrupt_enable) | state.force_interrupt
    }
}

/// Functional RP2040/RP2350 DMA controller with one-unit-per-step transfers.
pub struct RpDma {
    name: String,
    state: Arc<Mutex<RpDmaState>>,
}

impl RpDma {
    /// Creates a reset DMA controller and a service handle.
    pub fn new(name: impl Into<String>) -> (Self, RpDmaHandle) {
        let state = Arc::new(Mutex::new(RpDmaState {
            channels: [DmaChannel::default(); CHANNELS],
            raw_interrupt: 0,
            interrupt_enable: 0,
            force_interrupt: 0,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RpDmaHandle { state },
        )
    }

    fn channel_index(offset: u64) -> Option<(usize, u64)> {
        if offset < CHANNEL_STRIDE * CHANNELS as u64 {
            Some((
                usize::try_from(offset / CHANNEL_STRIDE).ok()?,
                offset % CHANNEL_STRIDE,
            ))
        } else {
            None
        }
    }

    fn channel_control(channel: &DmaChannel) -> u32 {
        let mut control = channel.control;
        if channel.transfer_count != 0 && control & CTRL_EN != 0 {
            control |= CTRL_BUSY;
        } else {
            control &= !CTRL_BUSY;
        }
        if channel.write_error {
            control |= CTRL_WRITE_ERROR;
        }
        control
    }
}

impl Device for RpDma {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RP DMA requires aligned word access"));
        }
        let state = self.state.lock().expect("RP DMA lock poisoned");
        if let Some((index, register)) = Self::channel_index(offset) {
            let channel = &state.channels[index];
            let value = match register {
                0x00 => channel.read_addr,
                0x04 => channel.write_addr,
                0x08 => channel.transfer_count,
                0x0c => Self::channel_control(channel),
                _ => 0,
            };
            return Ok(u64::from(value));
        }
        let value = match offset {
            0x400 => state.raw_interrupt,
            0x404 => state.interrupt_enable,
            0x414 => state.force_interrupt,
            0x424 => (state.raw_interrupt & state.interrupt_enable) | state.force_interrupt,
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RP DMA requires aligned word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits");
        let mut state = self.state.lock().expect("RP DMA lock poisoned");
        if let Some((index, register)) = Self::channel_index(offset) {
            let channel = &mut state.channels[index];
            match register {
                0x00 => channel.read_addr = value,
                0x04 => channel.write_addr = value,
                0x08 => channel.transfer_count = value,
                0x0c => {
                    channel.control = value & !CTRL_BUSY;
                    channel.write_error = false;
                }
                _ => {}
            }
            return Ok(());
        }
        match offset {
            0x400 => state.raw_interrupt &= !value,
            0x404 => state.interrupt_enable = value,
            0x414 => state.force_interrupt = value,
            0x424 => return Err(DeviceError::new("RP DMA interrupt status is read-only")),
            0x430 => {
                for index in 0..CHANNELS {
                    if value & (1 << index) != 0 {
                        state.channels[index].control |= CTRL_EN;
                    }
                }
            }
            0x444 => {
                for index in 0..CHANNELS {
                    if value & (1 << index) != 0 {
                        state.channels[index].control &= !CTRL_EN;
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
        let mut state = self.state.lock().expect("RP DMA lock poisoned");
        state.channels = [DmaChannel::default(); CHANNELS];
        state.raw_interrupt = 0;
        state.interrupt_enable = 0;
        state.force_interrupt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::{AddressSpace, Endianness};
    use remu_core::SimTime;

    #[test]
    fn word_channel_copies_memory_and_raises_interrupt() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2000_0000, 0x100, true).unwrap();
        let (dma, handle) = RpDma::new("dma");
        bus.map_device("dma", 0x5000_0000, 0x1000, Box::new(dma))
            .unwrap();
        bus.write(0x2000_0000, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        bus.write(0x5000_0000, AccessWidth::Word, 0x2000_0000, SimTime::ZERO)
            .unwrap();
        bus.write(0x5000_0004, AccessWidth::Word, 0x2000_0004, SimTime::ZERO)
            .unwrap();
        bus.write(0x5000_0008, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        bus.write(
            0x5000_000c,
            AccessWidth::Word,
            u64::from(CTRL_EN | (2 << 2)),
            SimTime::ZERO,
        )
        .unwrap();
        bus.write(0x5000_0404, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.service(&mut bus, SimTime::ZERO).unwrap(), 1);
        assert_eq!(
            bus.read(
                0x2000_0004,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
            0x1234_5678
        );
        assert_eq!(handle.pending(), 1);
    }
}
