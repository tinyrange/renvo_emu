use super::*;
use remu_core::{AccessKind, Bus};

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
    channels: Vec<DmaChannel>,
    raw_interrupt: u32,
    interrupt_enable: [u32; 4],
    force_interrupt: [u32; 4],
    variant: RpDmaVariant,
}

/// Target-specific RP DMA layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpDmaVariant {
    /// RP2040: twelve channels and two interrupt outputs.
    Rp2040,
    /// RP2350: sixteen channels and four interrupt outputs.
    Rp2350,
}

impl RpDmaVariant {
    const fn channel_count(self) -> usize {
        match self {
            Self::Rp2040 => 12,
            Self::Rp2350 => 16,
        }
    }

    const fn interrupt_count(self) -> usize {
        match self {
            Self::Rp2040 => 2,
            Self::Rp2350 => 4,
        }
    }

    const fn multi_channel_trigger(self) -> u64 {
        match self {
            Self::Rp2040 => 0x430,
            Self::Rp2350 => 0x450,
        }
    }

    const fn channel_abort(self) -> u64 {
        match self {
            Self::Rp2040 => 0x444,
            Self::Rp2350 => 0x464,
        }
    }
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
        let channel_count = self
            .state
            .lock()
            .expect("RP DMA lock poisoned")
            .channels
            .len();
        for index in 0..channel_count {
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
        self.pending_mask(0)
    }

    /// Returns the channel mask visible on one target interrupt output.
    pub fn pending_mask(&self, interrupt: usize) -> u32 {
        let state = self.state.lock().expect("RP DMA lock poisoned");
        if interrupt >= state.variant.interrupt_count() {
            return 0;
        }
        (state.raw_interrupt & state.interrupt_enable[interrupt]) | state.force_interrupt[interrupt]
    }

    /// Returns whether one target interrupt output is asserted.
    pub fn interrupt_pending(&self, interrupt: usize) -> bool {
        self.pending_mask(interrupt) != 0
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
        Self::new_for_variant(name, RpDmaVariant::Rp2040)
    }

    /// Creates the DMA layout implemented by the selected RP target.
    pub fn new_for_variant(name: impl Into<String>, variant: RpDmaVariant) -> (Self, RpDmaHandle) {
        let state = Arc::new(Mutex::new(RpDmaState {
            channels: vec![DmaChannel::default(); variant.channel_count()],
            raw_interrupt: 0,
            interrupt_enable: [0; 4],
            force_interrupt: [0; 4],
            variant,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RpDmaHandle { state },
        )
    }

    fn channel_index(offset: u64, channel_count: usize) -> Option<(usize, u64)> {
        if offset < CHANNEL_STRIDE * channel_count as u64 {
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

    fn interrupt_register(variant: RpDmaVariant, offset: u64) -> Option<(usize, u64)> {
        let relative = offset.checked_sub(0x404)?;
        let interrupt = usize::try_from(relative / 0x10).ok()?;
        let register = relative % 0x10;
        (interrupt < variant.interrupt_count() && register <= 8).then_some((interrupt, register))
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
        let offset = offset & 0x0fff;
        let state = self.state.lock().expect("RP DMA lock poisoned");
        if let Some((index, register)) = Self::channel_index(offset, state.channels.len()) {
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
        let value = if offset == 0x400 {
            state.raw_interrupt
        } else if let Some((interrupt, register)) = Self::interrupt_register(state.variant, offset)
        {
            match register {
                0 => state.interrupt_enable[interrupt],
                4 => state.force_interrupt[interrupt],
                8 => {
                    (state.raw_interrupt & state.interrupt_enable[interrupt])
                        | state.force_interrupt[interrupt]
                }
                _ => unreachable!("validated RP DMA interrupt register"),
            }
        } else if offset == state.variant.multi_channel_trigger()
            || offset == state.variant.channel_abort()
        {
            0
        } else {
            return Err(DeviceError::new(format!(
                "{} read outside modeled registers at offset {offset:#x}",
                self.name
            )));
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
        let alias = (offset >> 12) & 3;
        let offset = offset & 0x0fff;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits");
        let mut state = self.state.lock().expect("RP DMA lock poisoned");
        let channel_mask = (1_u32 << state.channels.len()) - 1;
        if let Some((index, register)) = Self::channel_index(offset, state.channels.len()) {
            let channel = &mut state.channels[index];
            match register {
                0x00 => Rp2040Resets::update(&mut channel.read_addr, alias, value)?,
                0x04 => Rp2040Resets::update(&mut channel.write_addr, alias, value)?,
                0x08 => Rp2040Resets::update(&mut channel.transfer_count, alias, value)?,
                0x0c => {
                    Rp2040Resets::update(&mut channel.control, alias, value & !CTRL_BUSY)?;
                    channel.write_error = false;
                }
                _ => {
                    return Err(DeviceError::new(format!(
                        "{} write outside modeled channel registers at offset {offset:#x}",
                        self.name
                    )));
                }
            }
            return Ok(());
        }
        if offset == 0x400 {
            state.raw_interrupt &= !(value & channel_mask);
        } else if let Some((interrupt, register)) = Self::interrupt_register(state.variant, offset)
        {
            match register {
                0 => Rp2040Resets::update(
                    &mut state.interrupt_enable[interrupt],
                    alias,
                    value & channel_mask,
                )?,
                4 => Rp2040Resets::update(
                    &mut state.force_interrupt[interrupt],
                    alias,
                    value & channel_mask,
                )?,
                8 => state.raw_interrupt &= !(value & channel_mask),
                _ => unreachable!("validated RP DMA interrupt register"),
            }
        } else if offset == state.variant.multi_channel_trigger() {
            for (index, channel) in state.channels.iter_mut().enumerate() {
                if value & (1 << index) != 0 {
                    channel.control |= CTRL_EN;
                }
            }
        } else if offset == state.variant.channel_abort() {
            for (index, channel) in state.channels.iter_mut().enumerate() {
                if value & (1 << index) != 0 {
                    channel.control &= !CTRL_EN;
                }
            }
        } else {
            return Err(DeviceError::new(format!(
                "{} write outside modeled registers at offset {offset:#x}",
                self.name
            )));
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("RP DMA lock poisoned");
        state.channels.fill(DmaChannel::default());
        state.raw_interrupt = 0;
        state.interrupt_enable = [0; 4];
        state.force_interrupt = [0; 4];
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

    #[test]
    fn rp2350_exposes_sixteen_channels_and_four_native_irq_banks() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2000_0000, 0x100, true).unwrap();
        let (dma, handle) = RpDma::new_for_variant("dma", RpDmaVariant::Rp2350);
        bus.map_device("dma", 0x5000_0000, 0x4000, Box::new(dma))
            .unwrap();
        let channel15 = 0x5000_0000 + 15 * CHANNEL_STRIDE;
        bus.write(0x2000_0000, AccessWidth::Word, 0x89ab_cdef, SimTime::ZERO)
            .unwrap();
        for (offset, value) in [
            (0x00, 0x2000_0000),
            (0x04, 0x2000_0004),
            (0x08, 1),
            (0x0c, u64::from(CTRL_EN | (2 << 2))),
        ] {
            bus.write(channel15 + offset, AccessWidth::Word, value, SimTime::ZERO)
                .unwrap();
        }
        bus.write(0x5000_0424, AccessWidth::Word, 1 << 15, SimTime::ZERO)
            .unwrap();

        assert_eq!(handle.service(&mut bus, SimTime::ZERO).unwrap(), 1);
        assert_eq!(handle.pending_mask(2), 1 << 15);
        assert_eq!(handle.pending_mask(3), 0);
        assert_eq!(
            bus.read(
                0x5000_042c,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            1 << 15
        );
        bus.write(0x5000_042c, AccessWidth::Word, 1 << 15, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.pending_mask(2), 0);
        bus.write(0x5000_0438, AccessWidth::Word, 1 << 15, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.pending_mask(3), 1 << 15);
        bus.write(0x5000_0438, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.pending_mask(3), 0);
    }

    #[test]
    fn rp2040_rejects_rp2350_only_irq_banks() {
        let mut dma = RpDma::new("dma").0;
        assert!(
            dma.read(0x424, AccessWidth::Word, SimTime::ZERO)
                .unwrap_err()
                .to_string()
                .contains("outside modeled registers")
        );
    }
}
