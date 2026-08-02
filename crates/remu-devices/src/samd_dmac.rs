use remu_bus::{Device, DeviceError};
use remu_core::{AccessKind, AccessWidth, Bus, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const CHANNEL_COUNT: usize = 12;
const CHANNEL_MASK: u32 = (1 << CHANNEL_COUNT) - 1;
const CHANNEL_STRIDE: u64 = 0x10;
const REGISTER_LIMIT: u64 = 0x50;

/// Named SAM D21 DMAC APB registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Samd21DmacRegister {
    /// DMA and CRC enable/priority control.
    Ctrl = 0x00,
    /// CRC input and polynomial selection.
    CrcCtrl = 0x02,
    /// CRC data input.
    CrcDataIn = 0x04,
    /// CRC checksum.
    CrcChecksum = 0x08,
    /// CRC status.
    CrcStatus = 0x0c,
    /// Debug-run control.
    DbgCtrl = 0x0d,
    /// Quality-of-service control.
    QosCtrl = 0x0e,
    /// Software trigger bitmap.
    SwTrigCtrl = 0x10,
    /// Priority arbitration control.
    PriCtrl0 = 0x14,
    /// Lowest pending channel and interrupt flags.
    IntPend = 0x20,
    /// Per-channel interrupt summary.
    IntStatus = 0x24,
    /// Busy channel bitmap.
    BusyCh = 0x28,
    /// Pending channel bitmap.
    PendCh = 0x2c,
    /// Active channel status.
    Active = 0x30,
    /// Descriptor memory base address.
    BaseAddr = 0x34,
    /// Write-back memory base address.
    WrbAddr = 0x38,
    /// Indirect channel selector.
    Chid = 0x3f,
    /// Selected channel enable/reset.
    ChCtrlA = 0x40,
    /// Selected channel trigger/event configuration.
    ChCtrlB = 0x44,
    /// Selected channel interrupt-enable clear.
    ChIntEnClr = 0x4c,
    /// Selected channel interrupt-enable set.
    ChIntEnSet = 0x4d,
    /// Selected channel interrupt flags and write-one-to-clear.
    ChIntFlag = 0x4e,
    /// Selected channel status.
    ChStatus = 0x4f,
}

impl Samd21DmacRegister {
    fn locate(offset: u64) -> Option<(Self, u64, u8)> {
        let registers = [
            (Self::Ctrl, 0x00, 2),
            (Self::CrcCtrl, 0x02, 2),
            (Self::CrcDataIn, 0x04, 4),
            (Self::CrcChecksum, 0x08, 4),
            (Self::CrcStatus, 0x0c, 1),
            (Self::DbgCtrl, 0x0d, 1),
            (Self::QosCtrl, 0x0e, 1),
            (Self::SwTrigCtrl, 0x10, 4),
            (Self::PriCtrl0, 0x14, 4),
            (Self::IntPend, 0x20, 2),
            (Self::IntStatus, 0x24, 4),
            (Self::BusyCh, 0x28, 4),
            (Self::PendCh, 0x2c, 4),
            (Self::Active, 0x30, 4),
            (Self::BaseAddr, 0x34, 4),
            (Self::WrbAddr, 0x38, 4),
            (Self::Chid, 0x3f, 1),
            (Self::ChCtrlA, 0x40, 1),
            (Self::ChCtrlB, 0x44, 4),
            (Self::ChIntEnClr, 0x4c, 1),
            (Self::ChIntEnSet, 0x4d, 1),
            (Self::ChIntFlag, 0x4e, 1),
            (Self::ChStatus, 0x4f, 1),
        ];
        registers
            .into_iter()
            .find(|(_, base, size)| offset >= *base && offset < *base + u64::from(*size))
    }
}

/// Named SAM D21 SRAM transfer-descriptor fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Samd21DmacDescriptorRegister {
    /// Block transfer configuration.
    Btctrl = 0x00,
    /// Number of beats in the block.
    Btcnt = 0x02,
    /// Source address (one-past-end when source increment is enabled).
    SrcAddr = 0x04,
    /// Destination address.
    DstAddr = 0x08,
    /// Next linked descriptor address.
    DescAddr = 0x0c,
}

#[derive(Clone, Copy, Default)]
struct ChannelState {
    ctrl_a: u8,
    ctrl_b: u32,
    interrupt_enable: u8,
    interrupt_flags: u8,
    fetch_error: bool,
    busy: bool,
    pending: bool,
}

#[derive(Clone, Copy)]
struct DmacState {
    ctrl: u16,
    crc_ctrl: u16,
    crc_data_in: u32,
    crc_checksum: u32,
    crc_status: u8,
    dbg_ctrl: u8,
    qos_ctrl: u8,
    sw_trig_ctrl: u16,
    prictl0: u32,
    channel_id: u8,
    base_addr: u32,
    wrb_addr: u32,
    active: Option<(u8, u16)>,
    channels: [ChannelState; CHANNEL_COUNT],
}

impl Default for DmacState {
    fn default() -> Self {
        Self {
            ctrl: 0,
            crc_ctrl: 0,
            crc_data_in: 0,
            crc_checksum: 0,
            crc_status: 0,
            dbg_ctrl: 0,
            qos_ctrl: 0x2a,
            sw_trig_ctrl: 0,
            prictl0: 0,
            channel_id: 0,
            base_addr: 0,
            wrb_addr: 0,
            active: None,
            channels: [ChannelState::default(); CHANNEL_COUNT],
        }
    }
}

impl DmacState {
    fn dma_enabled(self) -> bool {
        self.ctrl & (1 << 1) != 0
    }

    fn channel(self, channel: u8) -> ChannelState {
        self.channels[usize::from(channel)]
    }

    fn channel_mut(&mut self, channel: u8) -> &mut ChannelState {
        &mut self.channels[usize::from(channel)]
    }

    fn reset_channel(&mut self, channel: u8) {
        self.channels[usize::from(channel)] = ChannelState::default();
    }

    fn clear_activity(&mut self) {
        for channel in &mut self.channels {
            channel.busy = false;
            channel.pending = false;
        }
        self.sw_trig_ctrl = 0;
        self.active = None;
    }

    fn pending_channels(self) -> u32 {
        self.channels
            .iter()
            .enumerate()
            .fold(0_u32, |mask, (channel, state)| {
                mask | (u32::from(state.pending) << channel)
            })
    }

    fn busy_channels(self) -> u32 {
        self.channels
            .iter()
            .enumerate()
            .fold(0_u32, |mask, (channel, state)| {
                mask | (u32::from(state.busy) << channel)
            })
    }

    fn interrupt_status(self) -> u32 {
        self.channels
            .iter()
            .enumerate()
            .fold(0_u32, |mask, (channel, state)| {
                mask | (u32::from(state.interrupt_flags & state.interrupt_enable != 0) << channel)
            })
    }

    fn lowest_interrupt_channel(self) -> Option<u8> {
        self.channels
            .iter()
            .enumerate()
            .find_map(|(channel, state)| {
                (state.interrupt_flags & state.interrupt_enable != 0)
                    .then(|| u8::try_from(channel).expect("DMAC channel index fits u8"))
            })
    }
}

/// Host-facing handle for deterministic SAM D21 DMAC triggers and status.
#[derive(Clone)]
pub struct Samd21DmacHandle(Arc<Mutex<DmacState>>);

impl Samd21DmacHandle {
    /// Returns whether any enabled channel interrupt is pending.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.0.lock().expect("DMAC lock poisoned");
        state.interrupt_status() != 0
    }

    /// Returns the pending-channel bitmap used by host assertions.
    pub fn pending_channels(&self) -> u16 {
        let state = self.0.lock().expect("DMAC lock poisoned");
        u16::try_from(state.pending_channels()).expect("12 channels fit u16")
    }

    /// Injects a software trigger without requiring an MMIO write.
    pub fn software_trigger(&self, channel: u8) -> bool {
        if usize::from(channel) >= CHANNEL_COUNT {
            return false;
        }
        let mut state = self.0.lock().expect("DMAC lock poisoned");
        if !state.dma_enabled() || state.channel(channel).ctrl_a & 0x02 == 0 {
            return false;
        }
        let selected = state.channel_mut(channel);
        if selected.pending {
            selected.pending = true;
            state.sw_trig_ctrl |= 1 << channel;
        } else {
            selected.pending = true;
        }
        true
    }

    /// Services one pending channel using the machine's shared bus.
    ///
    /// A single valid descriptor is transferred atomically at the current
    /// abstract simulation time. This gives firmware tests useful memory-DMA
    /// behavior without claiming cycle-level arbitration or linked-descriptor
    /// fidelity.
    pub fn service<B: Bus + ?Sized>(&self, bus: &mut B, at: SimTime) -> bool {
        let Some((channel, base, write_back)) = self.begin_transfer() else {
            return false;
        };
        let descriptor = match read_descriptor(bus, base, channel, at) {
            Ok(descriptor) => descriptor,
            Err(_) => {
                self.finish_error(channel, true);
                return true;
            }
        };
        if descriptor.btctrl & 1 == 0 || descriptor.btcnt == 0 {
            self.finish_error(channel, true);
            return true;
        }
        let Some(beat) = beat_width(descriptor.btctrl) else {
            self.finish_error(channel, true);
            return true;
        };
        let beat_bytes = u32::from(beat.bytes());
        let source_increment = descriptor.btctrl & (1 << 10) != 0;
        let destination_increment = descriptor.btctrl & (1 << 11) != 0;
        let step_selection = descriptor.btctrl & (1 << 12) != 0;
        let step = 1_u32 << ((descriptor.btctrl >> 13) & 0x7);
        let source_step = if source_increment {
            beat_bytes.saturating_mul(if step_selection { step } else { 1 })
        } else {
            0
        };
        let destination_step = if destination_increment {
            beat_bytes.saturating_mul(if step_selection { 1 } else { step })
        } else {
            0
        };
        let mut source = descriptor.src_addr;
        let mut destination = descriptor.dst_addr;
        let mut success = true;
        for _ in 0..descriptor.btcnt {
            if source_increment {
                let Some(next) = source.checked_sub(source_step) else {
                    success = false;
                    break;
                };
                source = next;
            }
            let value = match bus.read(u64::from(source), beat, AccessKind::Read, at) {
                Ok(value) => value,
                Err(_) => {
                    success = false;
                    break;
                }
            };
            if bus.write(u64::from(destination), beat, value, at).is_err() {
                success = false;
                break;
            }
            if destination_increment {
                let Some(next) = destination.checked_add(destination_step) else {
                    success = false;
                    break;
                };
                destination = next;
            }
        }
        if !success {
            self.finish_error(channel, false);
            return true;
        }
        if write_back != 0 {
            let write_back_descriptor = DmacDescriptor {
                btctrl: descriptor.btctrl & !1,
                btcnt: 0,
                src_addr: source,
                dst_addr: destination,
                desc_addr: descriptor.desc_addr,
            };
            if write_descriptor(bus, write_back, channel, &write_back_descriptor, at).is_err() {
                self.finish_error(channel, false);
                return true;
            }
        }
        let block_action = (descriptor.btctrl >> 3) & 0x3;
        self.finish_success(
            channel,
            block_action == 2 || block_action == 3,
            block_action == 1 || block_action == 3,
        );
        true
    }

    fn begin_transfer(&self) -> Option<(u8, u32, u32)> {
        let mut state = self.0.lock().expect("DMAC lock poisoned");
        if !state.dma_enabled() {
            return None;
        }
        let channel = state
            .channels
            .iter()
            .enumerate()
            .find_map(|(channel, value)| {
                (value.pending && value.ctrl_a & 0x02 != 0)
                    .then(|| u8::try_from(channel).expect("DMAC channel index fits u8"))
            })?;
        let base = state.base_addr;
        let write_back = state.wrb_addr;
        {
            let selected = state.channel_mut(channel);
            selected.pending = false;
            selected.busy = true;
        }
        state.sw_trig_ctrl &= !(1 << channel);
        state.active = Some((channel, 0));
        Some((channel, base, write_back))
    }

    fn finish_error(&self, channel: u8, fetch_error: bool) {
        let mut state = self.0.lock().expect("DMAC lock poisoned");
        let selected = state.channel_mut(channel);
        selected.busy = false;
        selected.pending = false;
        selected.fetch_error |= fetch_error;
        if fetch_error {
            selected.interrupt_flags |= 1 << 2;
        } else {
            selected.ctrl_a &= !0x02;
            selected.interrupt_flags |= 1;
        }
        state.active = None;
    }

    fn finish_success(&self, channel: u8, suspend: bool, complete_interrupt: bool) {
        let mut state = self.0.lock().expect("DMAC lock poisoned");
        let selected = state.channel_mut(channel);
        selected.busy = false;
        selected.pending = false;
        if suspend {
            selected.interrupt_flags |= 1 << 2;
        } else {
            selected.ctrl_a &= !0x02;
        }
        if complete_interrupt {
            selected.interrupt_flags |= 1 << 1;
        }
        state.active = None;
    }
}

#[derive(Clone, Copy)]
struct DmacDescriptor {
    btctrl: u16,
    btcnt: u16,
    src_addr: u32,
    dst_addr: u32,
    desc_addr: u32,
}

fn beat_width(btctrl: u16) -> Option<AccessWidth> {
    match (btctrl >> 8) & 0x3 {
        0 => Some(AccessWidth::Byte),
        1 => Some(AccessWidth::HalfWord),
        2 => Some(AccessWidth::Word),
        _ => None,
    }
}

fn descriptor_base(base: u32, channel: u8) -> Option<u64> {
    u64::from(base).checked_add(u64::from(channel) * CHANNEL_STRIDE)
}

fn read_descriptor<B: Bus + ?Sized>(
    bus: &mut B,
    base: u32,
    channel: u8,
    at: SimTime,
) -> Result<DmacDescriptor, ()> {
    let base = descriptor_base(base, channel).ok_or(())?;
    let mut read = |offset: Samd21DmacDescriptorRegister, width: AccessWidth| {
        bus.read(base + offset as u64, width, AccessKind::Read, at)
            .map_err(|_| ())
    };
    Ok(DmacDescriptor {
        btctrl: u16::try_from(read(
            Samd21DmacDescriptorRegister::Btctrl,
            AccessWidth::HalfWord,
        )?)
        .map_err(|_| ())?,
        btcnt: u16::try_from(read(
            Samd21DmacDescriptorRegister::Btcnt,
            AccessWidth::HalfWord,
        )?)
        .map_err(|_| ())?,
        src_addr: u32::try_from(read(
            Samd21DmacDescriptorRegister::SrcAddr,
            AccessWidth::Word,
        )?)
        .map_err(|_| ())?,
        dst_addr: u32::try_from(read(
            Samd21DmacDescriptorRegister::DstAddr,
            AccessWidth::Word,
        )?)
        .map_err(|_| ())?,
        desc_addr: u32::try_from(read(
            Samd21DmacDescriptorRegister::DescAddr,
            AccessWidth::Word,
        )?)
        .map_err(|_| ())?,
    })
}

fn write_descriptor<B: Bus + ?Sized>(
    bus: &mut B,
    base: u32,
    channel: u8,
    descriptor: &DmacDescriptor,
    at: SimTime,
) -> Result<(), ()> {
    let base = descriptor_base(base, channel).ok_or(())?;
    let writes = [
        (
            Samd21DmacDescriptorRegister::Btctrl,
            AccessWidth::HalfWord,
            u64::from(descriptor.btctrl),
        ),
        (
            Samd21DmacDescriptorRegister::Btcnt,
            AccessWidth::HalfWord,
            u64::from(descriptor.btcnt),
        ),
        (
            Samd21DmacDescriptorRegister::SrcAddr,
            AccessWidth::Word,
            u64::from(descriptor.src_addr),
        ),
        (
            Samd21DmacDescriptorRegister::DstAddr,
            AccessWidth::Word,
            u64::from(descriptor.dst_addr),
        ),
        (
            Samd21DmacDescriptorRegister::DescAddr,
            AccessWidth::Word,
            u64::from(descriptor.desc_addr),
        ),
    ];
    for (offset, width, value) in writes {
        bus.write(base + offset as u64, width, value, at)
            .map_err(|_| ())?;
    }
    Ok(())
}

/// Functional SAM D21 twelve-channel DMAC register and descriptor slice.
pub struct Samd21Dmac {
    name: String,
    state: Arc<Mutex<DmacState>>,
}

impl Samd21Dmac {
    /// Constructs a reset DMAC and its machine-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21DmacHandle) {
        let state = Arc::new(Mutex::new(DmacState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Samd21DmacHandle(state),
        )
    }

    fn selected_channel(state: &DmacState) -> u8 {
        state
            .channel_id
            .min(u8::try_from(CHANNEL_COUNT - 1).expect("DMAC count fits u8"))
    }

    fn read_register(state: &DmacState, register: Samd21DmacRegister) -> u64 {
        match register {
            Samd21DmacRegister::Ctrl => u64::from(state.ctrl),
            Samd21DmacRegister::CrcCtrl => u64::from(state.crc_ctrl),
            Samd21DmacRegister::CrcDataIn => u64::from(state.crc_data_in),
            Samd21DmacRegister::CrcChecksum => u64::from(state.crc_checksum),
            Samd21DmacRegister::CrcStatus => u64::from(state.crc_status),
            Samd21DmacRegister::DbgCtrl => u64::from(state.dbg_ctrl),
            Samd21DmacRegister::QosCtrl => u64::from(state.qos_ctrl),
            Samd21DmacRegister::SwTrigCtrl => u64::from(state.sw_trig_ctrl),
            Samd21DmacRegister::PriCtrl0 => u64::from(state.prictl0),
            Samd21DmacRegister::IntPend => state.lowest_interrupt_channel().map_or(0, |channel| {
                let selected = state.channel(channel);
                u64::from(channel)
                    | (u64::from(selected.pending) << 15)
                    | (u64::from(selected.busy) << 14)
                    | (u64::from(selected.fetch_error) << 13)
                    | (u64::from(selected.interrupt_flags & 0x04 != 0) << 10)
                    | (u64::from(selected.interrupt_flags & 0x02 != 0) << 9)
                    | (u64::from(selected.interrupt_flags & 0x01 != 0) << 8)
            }),
            Samd21DmacRegister::IntStatus => u64::from(state.interrupt_status()),
            Samd21DmacRegister::BusyCh => u64::from(state.busy_channels()),
            Samd21DmacRegister::PendCh => u64::from(state.pending_channels()),
            Samd21DmacRegister::Active => state.active.map_or(0, |(channel, count)| {
                (u64::from(count) << 16) | (1 << 15) | (u64::from(channel) << 8)
            }),
            Samd21DmacRegister::BaseAddr => u64::from(state.base_addr),
            Samd21DmacRegister::WrbAddr => u64::from(state.wrb_addr),
            Samd21DmacRegister::Chid => u64::from(state.channel_id),
            Samd21DmacRegister::ChCtrlA => {
                u64::from(state.channel(Self::selected_channel(state)).ctrl_a)
            }
            Samd21DmacRegister::ChCtrlB => {
                u64::from(state.channel(Self::selected_channel(state)).ctrl_b)
            }
            Samd21DmacRegister::ChIntEnClr | Samd21DmacRegister::ChIntEnSet => u64::from(
                state
                    .channel(Self::selected_channel(state))
                    .interrupt_enable,
            ),
            Samd21DmacRegister::ChIntFlag => {
                u64::from(state.channel(Self::selected_channel(state)).interrupt_flags)
            }
            Samd21DmacRegister::ChStatus => {
                let channel = state.channel(Self::selected_channel(state));
                u64::from(channel.pending)
                    | (u64::from(channel.busy) << 1)
                    | (u64::from(channel.fetch_error) << 2)
            }
        }
    }

    fn reset_all(state: &mut DmacState) {
        let debug = state.dbg_ctrl;
        *state = DmacState::default();
        state.dbg_ctrl = debug;
    }

    fn write_register(state: &mut DmacState, register: Samd21DmacRegister, value: u64) {
        let value32 = value as u32;
        match register {
            Samd21DmacRegister::Ctrl => {
                let requested = u16::try_from(value & 0x0f03).expect("CTRL mask fits u16");
                if requested & 1 != 0 && requested & 0x06 == 0 {
                    Self::reset_all(state);
                } else {
                    state.ctrl = requested & !1;
                    if state.ctrl & 0x02 == 0 {
                        state.clear_activity();
                    }
                }
            }
            Samd21DmacRegister::CrcCtrl => {
                state.crc_ctrl = u16::try_from(value & 0x3f0f).expect("CRCCTRL mask fits u16")
            }
            Samd21DmacRegister::CrcDataIn => state.crc_data_in = value32,
            Samd21DmacRegister::CrcChecksum => {
                if state.ctrl & 0x04 == 0 {
                    state.crc_checksum = value32;
                }
            }
            Samd21DmacRegister::CrcStatus => {
                if value & 1 != 0 {
                    state.crc_status &= !1;
                }
            }
            Samd21DmacRegister::DbgCtrl => state.dbg_ctrl = value as u8 & 1,
            Samd21DmacRegister::QosCtrl => state.qos_ctrl = value as u8 & 0x3f,
            Samd21DmacRegister::SwTrigCtrl => {
                let requested = value32 & CHANNEL_MASK;
                let dma_enabled = state.dma_enabled();
                for channel in 0..CHANNEL_COUNT {
                    let bit = 1_u32 << channel;
                    if requested & bit == 0 {
                        continue;
                    }
                    let channel = u8::try_from(channel).expect("DMAC channel index fits u8");
                    let selected = state.channel_mut(channel);
                    if selected.pending {
                        state.sw_trig_ctrl |= u16::try_from(bit).expect("DMAC bit fits u16");
                    } else if dma_enabled && selected.ctrl_a & 0x02 != 0 {
                        selected.pending = true;
                    }
                }
            }
            Samd21DmacRegister::PriCtrl0 => state.prictl0 = value32,
            Samd21DmacRegister::IntPend => {
                state.channel_id = (value as u8).min(11);
                let selected = state.channel_mut(state.channel_id);
                if value & (1 << 10) != 0 {
                    selected.interrupt_flags &= !(1 << 2);
                }
                if value & (1 << 9) != 0 {
                    selected.interrupt_flags &= !(1 << 1);
                }
                if value & (1 << 8) != 0 {
                    selected.interrupt_flags &= !1;
                }
            }
            Samd21DmacRegister::IntStatus
            | Samd21DmacRegister::BusyCh
            | Samd21DmacRegister::PendCh
            | Samd21DmacRegister::Active
            | Samd21DmacRegister::ChStatus => {}
            Samd21DmacRegister::BaseAddr => {
                if !state.dma_enabled() {
                    state.base_addr = value32 & !0x3f;
                }
            }
            Samd21DmacRegister::WrbAddr => {
                if !state.dma_enabled() {
                    state.wrb_addr = value32 & !0x3f;
                }
            }
            Samd21DmacRegister::Chid => state.channel_id = (value as u8).min(11),
            Samd21DmacRegister::ChCtrlA => {
                let channel = Self::selected_channel(state);
                if value & 1 != 0 && state.channel(channel).ctrl_a & 0x02 == 0 {
                    state.reset_channel(channel);
                } else {
                    let selected = state.channel_mut(channel);
                    selected.ctrl_a = value as u8 & 0x02;
                    if selected.ctrl_a == 0 {
                        selected.pending = false;
                        selected.busy = false;
                    }
                }
            }
            Samd21DmacRegister::ChCtrlB => {
                let channel = Self::selected_channel(state);
                let command = (value32 >> 24) & 0x3;
                let selected = state.channel_mut(channel);
                selected.ctrl_b = value32 & 0x00c0_3f7f;
                match command {
                    1 => {
                        selected.pending = false;
                        selected.busy = false;
                        selected.interrupt_flags |= 1 << 2;
                    }
                    2 => {
                        selected.fetch_error = false;
                        selected.interrupt_flags &= !(1 << 2);
                    }
                    _ => {}
                }
            }
            Samd21DmacRegister::ChIntEnClr => {
                state
                    .channel_mut(Self::selected_channel(state))
                    .interrupt_enable &= !(value as u8 & 0x07)
            }
            Samd21DmacRegister::ChIntEnSet => {
                state
                    .channel_mut(Self::selected_channel(state))
                    .interrupt_enable |= value as u8 & 0x07
            }
            Samd21DmacRegister::ChIntFlag => {
                state
                    .channel_mut(Self::selected_channel(state))
                    .interrupt_flags &= !(value as u8 & 0x07)
            }
        }
    }
}

impl Device for Samd21Dmac {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let Some((register, base, size)) = Samd21DmacRegister::locate(offset) else {
            return if offset < REGISTER_LIMIT {
                Ok(0)
            } else {
                Err(DeviceError::new(format!(
                    "unmodeled DMAC read at {offset:#x}"
                )))
            };
        };
        let end = offset
            .checked_add(u64::from(width.bytes()))
            .ok_or_else(|| DeviceError::new("DMAC read offset overflow"))?;
        if end > base + u64::from(size) {
            return Err(DeviceError::new(format!("DMAC read crosses {register:?}")));
        }
        let state = self.state.lock().expect("DMAC lock poisoned");
        Ok(
            (Samd21Dmac::read_register(&state, register) >> ((offset - base) * 8))
                & width.value_mask(),
        )
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let Some((register, base, size)) = Samd21DmacRegister::locate(offset) else {
            if offset < REGISTER_LIMIT {
                return Ok(());
            }
            return Err(DeviceError::new(format!(
                "unmodeled DMAC write at {offset:#x}"
            )));
        };
        let end = offset
            .checked_add(u64::from(width.bytes()))
            .ok_or_else(|| DeviceError::new("DMAC write offset overflow"))?;
        if end > base + u64::from(size) {
            return Err(DeviceError::new(format!("DMAC write crosses {register:?}")));
        }
        let mut state = self.state.lock().expect("DMAC lock poisoned");
        let old = Samd21Dmac::read_register(&state, register);
        let shift = (offset - base) * 8;
        let mask = width.value_mask() << shift;
        let merged = (old & !mask) | ((value & width.value_mask()) << shift);
        Samd21Dmac::write_register(&mut state, register, merged);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("DMAC lock poisoned") = DmacState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::{AddressSpace, Endianness};

    #[test]
    fn named_registers_expose_channel_and_control_state() {
        let (mut dmac, _handle) = Samd21Dmac::new("dmac");
        dmac.write(0x0e, AccessWidth::Byte, 0x15, SimTime::ZERO)
            .unwrap();
        dmac.write(0x3f, AccessWidth::Byte, 3, SimTime::ZERO)
            .unwrap();
        dmac.write(0x40, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        dmac.write(0x44, AccessWidth::Word, 0x00c0_3f7f, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            dmac.read(0x0e, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            0x15
        );
        assert_eq!(
            dmac.read(0x3f, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            3
        );
        assert_eq!(
            dmac.read(0x40, AccessWidth::Byte, SimTime::ZERO).unwrap(),
            2
        );
        assert_eq!(
            dmac.read(0x44, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x00c0_3f7f
        );
    }

    #[test]
    fn software_trigger_copies_a_valid_descriptor_and_writes_back() {
        let mut bus = AddressSpace::new(Endianness::Little);
        let ram = bus.map_ram("sram", 0x2000_0000, 0x1000, true).unwrap();
        let (dmac, handle) = Samd21Dmac::new("dmac");
        bus.map_device("dmac", 0x4100_4800, 0x100, Box::new(dmac))
            .unwrap();
        ram.write_range(0x100, &[0xa1, 0xb2, 0xc3, 0xd4]);
        ram.write_u32(0x000, (1 << 10) | (1 << 11) | (1 << 3) | 1);
        ram.write_range(0x002, &4_u16.to_le_bytes());
        ram.write_u32(0x004, 0x2000_0104);
        ram.write_u32(0x008, 0x2000_0200);
        ram.write_u32(0x00c, 0);
        bus.write(0x4100_4834, AccessWidth::Word, 0x2000_0000, SimTime::ZERO)
            .unwrap();
        bus.write(0x4100_4838, AccessWidth::Word, 0x2000_0040, SimTime::ZERO)
            .unwrap();
        bus.write(0x4100_483f, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        bus.write(0x4100_484d, AccessWidth::Byte, 0x02, SimTime::ZERO)
            .unwrap();
        bus.write(0x4100_4840, AccessWidth::Byte, 0x02, SimTime::ZERO)
            .unwrap();
        bus.write(0x4100_4800, AccessWidth::HalfWord, 0x0202, SimTime::ZERO)
            .unwrap();
        bus.write(0x4100_4810, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.pending_channels(), 1);
        assert!(handle.service(&mut bus, SimTime::from_ticks(1)));
        assert_eq!(ram.read_range(0x200, 4).unwrap(), [0xd4, 0xc3, 0xb2, 0xa1]);
        assert_eq!(ram.read_range(0x040 + 2, 2), Some(vec![0, 0]));
        let flags = bus
            .read(
                0x4100_484e,
                AccessWidth::Byte,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(flags, 0x02);
        assert!(handle.interrupt_pending());
        bus.write(0x4100_484e, AccessWidth::Byte, 0x02, SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn invalid_descriptor_latches_fetch_error_and_suspend_flag() {
        let mut bus = AddressSpace::new(Endianness::Little);
        let ram = bus.map_ram("sram", 0x2000_0000, 0x200, true).unwrap();
        let (dmac, handle) = Samd21Dmac::new("dmac");
        bus.map_device("dmac", 0x4100_4800, 0x100, Box::new(dmac))
            .unwrap();
        ram.write_u32(0x000, 0);
        bus.write(0x4100_4834, AccessWidth::Word, 0x2000_0000, SimTime::ZERO)
            .unwrap();
        bus.write(0x4100_483f, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        bus.write(0x4100_4840, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        bus.write(0x4100_4800, AccessWidth::HalfWord, 2, SimTime::ZERO)
            .unwrap();
        bus.write(0x4100_4810, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(handle.service(&mut bus, SimTime::ZERO));
        let status = bus
            .read(
                0x4100_484f,
                AccessWidth::Byte,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap();
        let flags = bus
            .read(
                0x4100_484e,
                AccessWidth::Byte,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(status, 0x04);
        assert_eq!(flags, 0x04);
    }
}
