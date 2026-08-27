use super::*;
use remu_core::{AccessKind, Bus};

const CHANNEL_STRIDE: u64 = 0x40;
const CTRL_EN: u32 = 1 << 0;
const CTRL_DATA_SIZE_MASK: u32 = 0x3 << 2;
const CTRL_INCR_READ: u32 = 1 << 4;
const CTRL_INCR_WRITE: u32 = 1 << 5;
const CTRL_RING_SIZE_MASK: u32 = 0xf << 6;
const CTRL_RING_SEL: u32 = 1 << 10;
const CTRL_CHAIN_TO_MASK: u32 = 0xf << 11;
const CTRL_TREQ_SEL_MASK: u32 = 0x3f << 15;
const CTRL_IRQ_QUIET: u32 = 1 << 21;
const CTRL_BSWAP: u32 = 1 << 22;
const CTRL_SNIFF_EN: u32 = 1 << 23;
const CTRL_BUSY: u32 = 1 << 24;
const CTRL_WRITE_ERROR: u32 = 1 << 29;
const CTRL_READ_ERROR: u32 = 1 << 30;

#[derive(Clone, Copy)]
struct DmaChannel {
    read_addr: u32,
    write_addr: u32,
    transfer_count: u32,
    control: u32,
    write_error: bool,
    read_error: bool,
    read_ring_base: u32,
    write_ring_base: u32,
}

impl Default for DmaChannel {
    fn default() -> Self {
        Self {
            read_addr: 0,
            write_addr: 0,
            transfer_count: 0,
            control: 0,
            write_error: false,
            read_error: false,
            read_ring_base: 0,
            write_ring_base: 0,
        }
    }
}

struct RpDmaState {
    channels: Vec<DmaChannel>,
    raw_interrupt: u32,
    interrupt_enable: [u32; 4],
    force_interrupt: [u32; 4],
    variant: RpDmaVariant,
    timers: [u32; 4],
    timer_accumulators: [u32; 4],
    dreq: [Option<bool>; 64],
    sniff_control: u32,
    sniff_data: u32,
    security_channels: [u8; 16],
    security_interrupts: [u8; 4],
    security_misc: u16,
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

    const fn timer_base(self) -> u64 {
        match self {
            Self::Rp2040 => 0x420,
            Self::Rp2350 => 0x440,
        }
    }

    const fn sniff_control(self) -> u64 {
        match self {
            Self::Rp2040 => 0x434,
            Self::Rp2350 => 0x454,
        }
    }

    const fn sniff_data(self) -> u64 {
        self.sniff_control() + 4
    }

    const fn fifo_levels(self) -> u64 {
        match self {
            Self::Rp2040 => 0x440,
            Self::Rp2350 => 0x460,
        }
    }

    const fn channel_count_register(self) -> u64 {
        self.channel_abort() + 4
    }
}

/// Host-facing handle for deterministic RP DMA progress and interrupt state.
#[derive(Clone)]
pub struct RpDmaHandle {
    state: Arc<Mutex<RpDmaState>>,
}

impl RpDmaHandle {
    /// Sets one external DREQ input. Unconnected request lines remain
    /// permissive for compatibility; connected lines are strictly paced.
    pub fn set_dreq(&self, request: u8, asserted: bool) {
        if let Some(line) = self
            .state
            .lock()
            .expect("RP DMA lock poisoned")
            .dreq
            .get_mut(usize::from(request))
        {
            *line = Some(asserted);
        }
    }

    /// Copies at most one paced transfer unit for each enabled channel.
    pub fn service(&self, bus: &mut dyn Bus, at: SimTime) -> Result<usize, DeviceError> {
        self.service_with_context(bus, at, |_, _, _| {})
    }

    /// Services DMA while selecting each RP2350 channel's security context.
    pub fn service_with_context(
        &self,
        bus: &mut dyn Bus,
        at: SimTime,
        mut select_context: impl FnMut(usize, bool, bool),
    ) -> Result<usize, DeviceError> {
        let mut completed = 0;
        let (channel_count, timer_ready) = {
            let mut state = self.state.lock().expect("RP DMA lock poisoned");
            let mut ready = [false; 4];
            for timer in 0..4 {
                let numerator = state.timers[timer] >> 16;
                let denominator = state.timers[timer] & 0xffff;
                if numerator != 0 && denominator != 0 {
                    let accumulator = state.timer_accumulators[timer].saturating_add(numerator);
                    if accumulator >= denominator {
                        ready[timer] = true;
                        state.timer_accumulators[timer] = accumulator - denominator;
                    } else {
                        state.timer_accumulators[timer] = accumulator;
                    }
                }
            }
            (state.channels.len(), ready)
        };
        for index in 0..channel_count {
            let Some((
                read_addr,
                write_addr,
                width,
                increment_read,
                increment_write,
                byte_swap,
                secure,
                privileged,
            )) = ({
                let state = self.state.lock().expect("RP DMA lock poisoned");
                let channel = state.channels[index];
                if channel.control & CTRL_EN == 0 || channel.transfer_count == 0 {
                    None
                } else {
                    let request = ((channel.control & CTRL_TREQ_SEL_MASK) >> 15) as usize;
                    let paced = match request {
                        0x3f => true,
                        0x3b..=0x3e => timer_ready[request - 0x3b],
                        _ => state.dreq[request].unwrap_or(true),
                    };
                    if !paced {
                        continue;
                    }
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
                        channel.control & CTRL_BSWAP != 0,
                        state.variant != RpDmaVariant::Rp2350
                            || state.security_channels[index] & 2 != 0,
                        state.variant != RpDmaVariant::Rp2350
                            || state.security_channels[index] & 1 != 0,
                    ))
                }
            })
            else {
                continue;
            };
            select_context(index, secure, privileged);
            let value = match bus.read(read_addr, width, AccessKind::Read, at) {
                Ok(value) => value,
                Err(error) => {
                    let mut state = self.state.lock().expect("RP DMA lock poisoned");
                    state.channels[index].read_error = true;
                    state.channels[index].control &= !CTRL_EN;
                    return Err(DeviceError::new(format!(
                        "RP DMA channel {index} read: {error}"
                    )));
                }
            };
            let value = if byte_swap {
                match width {
                    AccessWidth::Byte => value,
                    AccessWidth::HalfWord => u64::from((value as u16).swap_bytes()),
                    AccessWidth::Word => u64::from((value as u32).swap_bytes()),
                    AccessWidth::DoubleWord => value.swap_bytes(),
                }
            } else {
                value
            };
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
            if state.sniff_control & 1 != 0
                && ((state.sniff_control >> 1) & 0xf) as usize == index
                && state.channels[index].control & CTRL_SNIFF_EN != 0
            {
                update_sniffer(&mut state, value as u32, width);
            }
            let (finished, quiet, chain) = {
                let channel = &mut state.channels[index];
                channel.transfer_count -= 1;
                if increment_read {
                    channel.read_addr = ring_increment(
                        channel.read_addr,
                        channel.read_ring_base,
                        step,
                        channel.control,
                        false,
                    );
                }
                if increment_write {
                    channel.write_addr = ring_increment(
                        channel.write_addr,
                        channel.write_ring_base,
                        step,
                        channel.control,
                        true,
                    );
                }
                let finished = channel.transfer_count == 0;
                if finished {
                    channel.control &= !CTRL_EN;
                }
                (
                    finished,
                    channel.control & CTRL_IRQ_QUIET != 0,
                    ((channel.control & CTRL_CHAIN_TO_MASK) >> 11) as usize,
                )
            };
            if finished {
                if !quiet {
                    state.raw_interrupt |= 1 << index;
                }
                if chain != index && chain < state.channels.len() {
                    state.channels[chain].control |= CTRL_EN;
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

fn ring_increment(address: u32, base: u32, step: u32, control: u32, write: bool) -> u32 {
    let ring_size = (control & CTRL_RING_SIZE_MASK) >> 6;
    let selected = control & CTRL_RING_SEL != 0;
    if ring_size == 0 || selected != write {
        return address.wrapping_add(step);
    }
    let mask = (1_u32 << ring_size) - 1;
    (base & !mask) | address.wrapping_add(step) & mask
}

fn update_sniffer(state: &mut RpDmaState, value: u32, width: AccessWidth) {
    let bytes = value.to_le_bytes();
    let bytes = &bytes[..usize::from(width.bytes().min(4))];
    let calculation = (state.sniff_control >> 5) & 0xf;
    match calculation {
        0 | 1 => {
            let reflected = calculation == 1;
            for byte in bytes {
                let mut input = if reflected {
                    byte.reverse_bits()
                } else {
                    *byte
                };
                for _ in 0..8 {
                    let carry = (state.sniff_data >> 31) as u8 ^ (input >> 7);
                    state.sniff_data <<= 1;
                    if carry != 0 {
                        state.sniff_data ^= 0x04c1_1db7;
                    }
                    input <<= 1;
                }
            }
        }
        2 | 3 => {
            let reflected = calculation == 3;
            let mut crc = state.sniff_data as u16;
            for byte in bytes {
                let mut input = if reflected {
                    byte.reverse_bits()
                } else {
                    *byte
                };
                for _ in 0..8 {
                    let carry = (crc >> 15) as u8 ^ (input >> 7);
                    crc <<= 1;
                    if carry != 0 {
                        crc ^= 0x1021;
                    }
                    input <<= 1;
                }
            }
            state.sniff_data = u32::from(crc);
        }
        0xe => state.sniff_data ^= value.count_ones() & 1,
        0xf => state.sniff_data = state.sniff_data.wrapping_add(value),
        _ => {}
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
            timers: [0; 4],
            timer_accumulators: [0; 4],
            dreq: [None; 64],
            sniff_control: 0,
            sniff_data: 0,
            security_channels: [3; 16],
            security_interrupts: [3; 4],
            security_misc: 0x03ff,
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
        if channel.read_error {
            control |= CTRL_READ_ERROR;
        }
        control
    }

    fn channel_register(channel: &DmaChannel, register: u64) -> u32 {
        match register {
            0x00 | 0x14 | 0x28 | 0x3c => channel.read_addr,
            0x04 | 0x18 | 0x2c | 0x34 => channel.write_addr,
            0x08 | 0x1c | 0x24 | 0x38 => channel.transfer_count,
            0x0c | 0x10 | 0x20 | 0x30 => Self::channel_control(channel),
            _ => 0,
        }
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
            let value = Self::channel_register(channel, register);
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
        } else if (state.variant.timer_base()..state.variant.timer_base() + 0x10).contains(&offset)
        {
            state.timers[((offset - state.variant.timer_base()) / 4) as usize]
        } else if offset == state.variant.sniff_control() {
            state.sniff_control
        } else if offset == state.variant.sniff_data() {
            let mut value = state.sniff_data;
            if state.sniff_control & (1 << 9) != 0 {
                value = value.swap_bytes();
            }
            if state.sniff_control & (1 << 10) != 0 {
                value = value.reverse_bits();
            }
            if state.sniff_control & (1 << 11) != 0 {
                value = !value;
            }
            value
        } else if offset == state.variant.fifo_levels()
            || offset == state.variant.multi_channel_trigger()
            || offset == state.variant.channel_abort()
        {
            0
        } else if offset == state.variant.channel_count_register() {
            state.channels.len() as u32
        } else if state.variant == RpDmaVariant::Rp2350 && (0x480..0x4c0).contains(&offset) {
            u32::from(state.security_channels[((offset - 0x480) / 4) as usize])
        } else if state.variant == RpDmaVariant::Rp2350 && (0x4c0..0x4d0).contains(&offset) {
            u32::from(state.security_interrupts[((offset - 0x4c0) / 4) as usize])
        } else if state.variant == RpDmaVariant::Rp2350 && offset == 0x4d0 {
            u32::from(state.security_misc)
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
            let trigger_value = {
                let channel = &mut state.channels[index];
                match register {
                    0x00 | 0x14 | 0x28 | 0x3c => {
                        Rp2040Resets::update(&mut channel.read_addr, alias, value)?;
                        channel.read_ring_base = channel.read_addr;
                    }
                    0x04 | 0x18 | 0x2c | 0x34 => {
                        Rp2040Resets::update(&mut channel.write_addr, alias, value)?;
                        channel.write_ring_base = channel.write_addr;
                    }
                    0x08 | 0x1c | 0x24 | 0x38 => {
                        Rp2040Resets::update(&mut channel.transfer_count, alias, value)?;
                    }
                    0x0c | 0x10 | 0x20 | 0x30 => {
                        Rp2040Resets::update(&mut channel.control, alias, value & !CTRL_BUSY)?;
                        channel.write_error = false;
                        channel.read_error = false;
                    }
                    _ => {
                        return Err(DeviceError::new(format!(
                            "{} write outside modeled channel registers at offset {offset:#x}",
                            self.name
                        )));
                    }
                }
                match register {
                    0x0c => channel.control & CTRL_EN,
                    0x1c => channel.transfer_count,
                    0x2c => channel.write_addr,
                    0x3c => channel.read_addr,
                    _ => 0,
                }
            };
            if state.variant == RpDmaVariant::Rp2350 {
                state.security_channels[index] |= 1 << 2;
            }
            let trigger = matches!(register, 0x0c | 0x1c | 0x2c | 0x3c);
            if trigger {
                if trigger_value != 0 {
                    state.channels[index].control |= CTRL_EN;
                } else if state.channels[index].control & CTRL_IRQ_QUIET != 0 {
                    state.raw_interrupt |= 1 << index;
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
        } else if (state.variant.timer_base()..state.variant.timer_base() + 0x10).contains(&offset)
        {
            let timer = ((offset - state.variant.timer_base()) / 4) as usize;
            Rp2040Resets::update(&mut state.timers[timer], alias, value)?;
            state.timer_accumulators[timer] = 0;
        } else if offset == state.variant.sniff_control() {
            Rp2040Resets::update(&mut state.sniff_control, alias, value & 0x0fff)?;
        } else if offset == state.variant.sniff_data() {
            Rp2040Resets::update(&mut state.sniff_data, alias, value)?;
        } else if state.variant == RpDmaVariant::Rp2350 && (0x480..0x4c0).contains(&offset) {
            let channel = ((offset - 0x480) / 4) as usize;
            if state.security_channels[channel] & (1 << 2) == 0 {
                state.security_channels[channel] = (value & 7) as u8;
            }
        } else if state.variant == RpDmaVariant::Rp2350 && (0x4c0..0x4d0).contains(&offset) {
            let interrupt = ((offset - 0x4c0) / 4) as usize;
            state.security_interrupts[interrupt] = (value & 3) as u8;
        } else if state.variant == RpDmaVariant::Rp2350 && offset == 0x4d0 {
            state.security_misc = (value & 0x03ff) as u16;
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
        state.timers = [0; 4];
        state.timer_accumulators = [0; 4];
        state.dreq = [None; 64];
        state.sniff_control = 0;
        state.sniff_data = 0;
        state.security_channels = [3; 16];
        state.security_interrupts = [3; 4];
        state.security_misc = 0x03ff;
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
            dma.read(0x4c0, AccessWidth::Word, SimTime::ZERO)
                .unwrap_err()
                .to_string()
                .contains("outside modeled registers")
        );
    }

    #[test]
    fn pacing_ring_byte_swap_and_chaining_are_functional() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2000_0000, 0x100, true).unwrap();
        let (dma, handle) = RpDma::new("dma");
        bus.map_device("dma", 0x5000_0000, 0x1000, Box::new(dma))
            .unwrap();
        for (offset, value) in [(0, 0x1122_u64), (2, 0x3344), (4, 0x5566), (6, 0x7788)] {
            bus.write(
                0x2000_0000 + offset,
                AccessWidth::HalfWord,
                value,
                SimTime::ZERO,
            )
            .unwrap();
        }
        // Channel 1 is armed by channel 0's CHAIN_TO on completion.
        for (offset, value) in [
            (0x40, 0x2000_0000),
            (0x44, 0x2000_0020),
            (0x48, 1),
            (0x50, u64::from((2_u32 << 2) | (0x3f << 15))),
        ] {
            bus.write(
                0x5000_0000 + offset,
                AccessWidth::Word,
                value,
                SimTime::ZERO,
            )
            .unwrap();
        }
        for (offset, value) in [
            (0x00, 0x2000_0000),
            (0x04, 0x2000_0010),
            (0x08, 4),
            (
                0x0c,
                u64::from(
                    CTRL_EN
                        | (1 << 2)
                        | CTRL_INCR_READ
                        | CTRL_INCR_WRITE
                        | (2 << 6)
                        | CTRL_RING_SEL
                        | (1 << 11)
                        | (0x3f << 15)
                        | CTRL_BSWAP,
                ),
            ),
        ] {
            bus.write(
                0x5000_0000 + offset,
                AccessWidth::Word,
                value,
                SimTime::ZERO,
            )
            .unwrap();
        }
        for tick in 0..4 {
            handle.service(&mut bus, SimTime::from_ticks(tick)).unwrap();
        }
        assert_eq!(
            bus.read(
                0x2000_0010,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
            0x8877_6655
        );
        assert_eq!(
            bus.read(
                0x2000_0020,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
            0x3344_1122
        );
    }

    #[test]
    fn timer_dreq_and_sum_sniffer_pace_and_observe_data() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2000_0000, 0x100, true).unwrap();
        let (dma, handle) = RpDma::new("dma");
        bus.map_device("dma", 0x5000_0000, 0x1000, Box::new(dma))
            .unwrap();
        bus.write(0x2000_0000, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        bus.write(0x5000_0420, AccessWidth::Word, (1 << 16) | 2, SimTime::ZERO)
            .unwrap();
        bus.write(
            0x5000_0434,
            AccessWidth::Word,
            1 | (0xf << 5),
            SimTime::ZERO,
        )
        .unwrap();
        for (offset, value) in [
            (0x00, 0x2000_0000),
            (0x04, 0x2000_0004),
            (0x08, 1),
            (
                0x0c,
                u64::from(CTRL_EN | (2 << 2) | (0x3b << 15) | CTRL_SNIFF_EN),
            ),
        ] {
            bus.write(
                0x5000_0000 + offset,
                AccessWidth::Word,
                value,
                SimTime::ZERO,
            )
            .unwrap();
        }
        assert_eq!(handle.service(&mut bus, SimTime::from_ticks(1)).unwrap(), 0);
        assert_eq!(handle.service(&mut bus, SimTime::from_ticks(2)).unwrap(), 1);
        assert_eq!(
            bus.read(
                0x5000_0438,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO
            )
            .unwrap(),
            0x1234_5678
        );
    }

    #[test]
    fn quiet_terminator_and_rp2350_security_lock_are_visible() {
        let mut dma = RpDma::new_for_variant("dma", RpDmaVariant::Rp2350).0;
        dma.write(0x480, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        dma.write(
            0x10,
            AccessWidth::Word,
            u64::from(CTRL_IRQ_QUIET),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            dma.read(0x480, AccessWidth::Word, SimTime::ZERO).unwrap(),
            4
        );
        dma.write(0x480, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            dma.read(0x480, AccessWidth::Word, SimTime::ZERO).unwrap(),
            4
        );
        dma.write(0x1c, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            dma.read(0x400, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1
        );
        assert_eq!(
            dma.read(0x468, AccessWidth::Word, SimTime::ZERO).unwrap(),
            16
        );
    }

    #[test]
    fn rp2350_channel_security_selects_each_dma_bus_context() {
        let mut bus = AddressSpace::new(Endianness::Little);
        bus.map_ram("ram", 0x2000_0000, 0x100, true).unwrap();
        bus.write(0x2000_0000, AccessWidth::Word, 0xfeed_beef, SimTime::ZERO)
            .unwrap();
        let (mut dma, handle) = RpDma::new_for_variant("dma", RpDmaVariant::Rp2350);
        dma.write(0x480, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        for (offset, value) in [
            (0x00, 0x2000_0000),
            (0x04, 0x2000_0004),
            (0x08, 1),
            (0x0c, u64::from(CTRL_EN | (2 << 2) | (0x3f << 15))),
        ] {
            dma.write(offset, AccessWidth::Word, value, SimTime::ZERO)
                .unwrap();
        }
        let mut selected = Vec::new();
        assert_eq!(
            handle
                .service_with_context(&mut bus, SimTime::ZERO, |channel, secure, privileged| {
                    selected.push((channel, secure, privileged));
                })
                .unwrap(),
            1
        );
        assert_eq!(selected, [(0, false, false)]);
    }
}
