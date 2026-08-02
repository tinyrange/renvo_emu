//! Functional SAM D21 Event System register and software-event model.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

/// Number of event channels present on the SAM D21.
pub const SAMD21_EVSYS_CHANNEL_COUNT: usize = 12;
/// Number of user-multiplexer entries defined by the SAM D21.
pub const SAMD21_EVSYS_USER_COUNT: usize = 29;

const CHANNEL_MASK: u32 = 0x0f;
const USER_MASK: u32 = 0x1f;
const EVENT_CHANNEL_MASK: u32 = 0x1f;
const EVENT_GENERATOR_MASK: u32 = 0x7f;
// The interrupt vectors are split around the reserved bits in the SAM D21
// register layout: channels 0..7 occupy the low byte and channels 8..11 the
// upper nibble for both OVR and EVD.
const OVR_MASK: u32 = 0x000f_00ff;
const EVD_MASK: u32 = 0x0f00_ff00;
const CHSTATUS_RESET: u32 = 0x000f_00ff;

fn user_ready_bit(channel: usize) -> u32 {
    if channel < 8 {
        1 << channel
    } else {
        1 << (16 + channel - 8)
    }
}

fn evd_bit(channel: usize) -> u32 {
    if channel < 8 {
        1 << (8 + channel)
    } else {
        1 << (24 + channel - 8)
    }
}

/// Named SAM D21 EVSYS register offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Samd21EvsysRegister {
    /// Generic-clock request and software reset.
    Control = 0x00,
    /// Indexed channel configuration.
    Channel = 0x04,
    /// Indexed user multiplexer configuration.
    User = 0x08,
    /// Read-only channel busy and user-ready status.
    ChannelStatus = 0x0c,
    /// Write-one-to-clear interrupt enables.
    InterruptEnableClear = 0x10,
    /// Write-one-to-set interrupt enables.
    InterruptEnableSet = 0x14,
    /// Write-one-to-clear event and overrun flags.
    InterruptFlags = 0x18,
}

impl Samd21EvsysRegister {
    /// Converts a register-block byte offset to its named register.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        match offset & !3 {
            0x00 => Some(Self::Control),
            0x04 => Some(Self::Channel),
            0x08 => Some(Self::User),
            0x0c => Some(Self::ChannelStatus),
            0x10 => Some(Self::InterruptEnableClear),
            0x14 => Some(Self::InterruptEnableSet),
            0x18 => Some(Self::InterruptFlags),
            _ => None,
        }
    }

    /// Returns the native byte offset of this register.
    pub const fn offset(self) -> u64 {
        self as u64
    }
}

#[derive(Clone, Copy)]
struct ChannelConfig {
    event_generator: u8,
    path: u8,
    edge: u8,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            event_generator: 0,
            path: 0,
            edge: 0,
        }
    }
}

struct EvsysState {
    generic_clock_request: bool,
    channels: [ChannelConfig; SAMD21_EVSYS_CHANNEL_COUNT],
    channel_configured: [bool; SAMD21_EVSYS_CHANNEL_COUNT],
    selected_channel: u8,
    user_channels: [u8; SAMD21_EVSYS_USER_COUNT],
    selected_user: u8,
    interrupt_enable: u32,
    interrupt_flags: u32,
    events: [u64; SAMD21_EVSYS_CHANNEL_COUNT],
}

impl Default for EvsysState {
    fn default() -> Self {
        Self {
            generic_clock_request: false,
            channels: [ChannelConfig::default(); SAMD21_EVSYS_CHANNEL_COUNT],
            channel_configured: [false; SAMD21_EVSYS_CHANNEL_COUNT],
            selected_channel: 0,
            user_channels: [0; SAMD21_EVSYS_USER_COUNT],
            selected_user: 0,
            interrupt_enable: 0,
            interrupt_flags: 0,
            events: [0; SAMD21_EVSYS_CHANNEL_COUNT],
        }
    }
}

impl EvsysState {
    fn channel_value(&self) -> u32 {
        let channel = usize::from(self.selected_channel);
        let config = self.channels[channel];
        u32::from(self.selected_channel)
            | (u32::from(config.event_generator) << 16)
            | (u32::from(config.edge & 0x3) << 26)
            | (u32::from(config.path & 0x3) << 24)
    }

    fn user_value(&self) -> u16 {
        let user = usize::from(self.selected_user);
        let channel = self.user_channels[user];
        u16::from(self.selected_user) | (u16::from(channel) << 8)
    }

    fn status_value(&self) -> u32 {
        // On reset the device reports every channel user-ready.  Once a
        // channel is configured for the asynchronous path, the hardware
        // reports zero for that channel because asynchronous paths have no
        // channel-clock status or interrupt state.  The first eight channels
        // are the only channels that support synchronous/resynchronized paths
        // on SAM D21.  Events are delivered immediately in this functional
        // model, so CHBUSY is never asserted.
        let mut status = CHSTATUS_RESET;
        for channel in 0..SAMD21_EVSYS_CHANNEL_COUNT {
            if self.channel_configured[channel]
                && (channel >= 8 || self.channels[channel].path == 2)
            {
                status &= !user_ready_bit(channel);
            }
        }
        status
    }

    fn reset(&mut self) {
        *self = Self::default();
    }

    fn trigger(&mut self, channel: u8, software: bool) -> bool {
        let channel_index = usize::from(channel);
        if channel_index >= SAMD21_EVSYS_CHANNEL_COUNT {
            return false;
        }
        self.events[channel_index] = self.events[channel_index].saturating_add(1);
        let config = self.channels[channel_index];
        // The SAM D21 does not set EVD for an asynchronous channel.  A
        // synchronous/resynchronized channel with no edge selection also has
        // no event output.  This is the useful software-event contract while
        // peripheral generator wiring remains a later slice.
        let output_enabled = if software {
            // Microchip documents software events only for a clocked channel,
            // with a synchronous or resynchronized path and rising-edge
            // detection.  SAM D21 channels 8..11 cannot use those paths.
            channel_index < 8 && self.generic_clock_request && config.path <= 1 && config.edge == 1
        } else {
            match config.path {
                // Asynchronous events propagate directly to users but do not
                // generate EVD interrupts.
                2 => true,
                // Edge detection (and therefore EVD) is available only on the
                // first eight, clocked channels.
                0 | 1 => channel_index < 8 && config.edge != 0,
                _ => false,
            }
        };
        if output_enabled {
            if config.path != 2 {
                self.interrupt_flags |= evd_bit(channel_index);
            }
            true
        } else {
            false
        }
    }
}

/// Host-facing handle for injecting events and observing the functional slice.
#[derive(Clone)]
pub struct Samd21EvsysHandle(Arc<Mutex<EvsysState>>);

impl Samd21EvsysHandle {
    /// Injects one event on a channel, as a peripheral generator would.
    pub fn trigger(&self, channel: u8) -> bool {
        self.0
            .lock()
            .expect("EVSYS lock poisoned")
            .trigger(channel, false)
    }

    /// Number of events observed on a channel since reset.
    pub fn event_count(&self, channel: u8) -> u64 {
        self.0
            .lock()
            .expect("EVSYS lock poisoned")
            .events
            .get(usize::from(channel))
            .copied()
            .unwrap_or(0)
    }

    /// Current event/overrun interrupt flags.
    pub fn flags(&self) -> u32 {
        self.0.lock().expect("EVSYS lock poisoned").interrupt_flags
    }

    /// Whether an enabled event-system interrupt is currently requested.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.0.lock().expect("EVSYS lock poisoned");
        state.interrupt_flags & state.interrupt_enable != 0
    }

    /// Channel connected to a user, or `None` when the user mux is disabled.
    pub fn user_channel(&self, user: u8) -> Option<u8> {
        self.0
            .lock()
            .expect("EVSYS lock poisoned")
            .user_channels
            .get(usize::from(user))
            .copied()
            .and_then(|channel| channel.checked_sub(1))
            .filter(|channel| usize::from(*channel) < SAMD21_EVSYS_CHANNEL_COUNT)
    }
}

/// Functional SAM D21 12-channel Event System register block.
pub struct Samd21Evsys {
    name: String,
    state: Arc<Mutex<EvsysState>>,
}

impl Samd21Evsys {
    /// Creates EVSYS and its scheduler/board-facing event handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21EvsysHandle) {
        let state = Arc::new(Mutex::new(EvsysState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Samd21EvsysHandle(state),
        )
    }

    fn raw_read(state: &EvsysState, register: Samd21EvsysRegister) -> u32 {
        match register {
            Samd21EvsysRegister::Control => u32::from(state.generic_clock_request) << 4,
            Samd21EvsysRegister::Channel => state.channel_value(),
            Samd21EvsysRegister::User => u32::from(state.user_value()),
            Samd21EvsysRegister::ChannelStatus => state.status_value(),
            Samd21EvsysRegister::InterruptEnableClear | Samd21EvsysRegister::InterruptEnableSet => {
                state.interrupt_enable
            }
            Samd21EvsysRegister::InterruptFlags => state.interrupt_flags,
        }
    }

    fn register_value(
        state: &EvsysState,
        offset: u64,
        width: AccessWidth,
    ) -> Result<(Samd21EvsysRegister, u32), DeviceError> {
        let register = Samd21EvsysRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!("unmodeled SAM D21 EVSYS access at {offset:#x}"))
        })?;
        let base = register.offset();
        let shift = offset
            .checked_sub(base)
            .and_then(|value| value.checked_mul(8))
            .ok_or_else(|| DeviceError::new("SAM D21 EVSYS access offset overflow"))?;
        let bits = u64::from(width.bytes()) * 8;
        if shift + bits > 32 {
            return Err(DeviceError::new(format!(
                "SAM D21 EVSYS access crosses register boundary at {offset:#x}"
            )));
        }
        Ok((
            register,
            (Self::raw_read(state, register) >> shift) & width.value_mask() as u32,
        ))
    }

    fn merged_write(
        state: &EvsysState,
        offset: u64,
        width: AccessWidth,
        value: u64,
    ) -> Result<(Samd21EvsysRegister, u32), DeviceError> {
        let register = Samd21EvsysRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!("unmodeled SAM D21 EVSYS access at {offset:#x}"))
        })?;
        let base = register.offset();
        let shift = offset
            .checked_sub(base)
            .and_then(|value| value.checked_mul(8))
            .ok_or_else(|| DeviceError::new("SAM D21 EVSYS access offset overflow"))?;
        let bits = u64::from(width.bytes()) * 8;
        if shift + bits > 32 {
            return Err(DeviceError::new(format!(
                "SAM D21 EVSYS access crosses register boundary at {offset:#x}"
            )));
        }
        let mask = (width.value_mask() as u32) << shift;
        let old = Self::raw_read(state, register);
        let merged = (old & !mask) | (((value & width.value_mask()) as u32) << shift);
        Ok((register, merged))
    }

    fn payload(
        offset: u64,
        width: AccessWidth,
        value: u64,
    ) -> Result<(Samd21EvsysRegister, u32), DeviceError> {
        let register = Samd21EvsysRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!("unmodeled SAM D21 EVSYS access at {offset:#x}"))
        })?;
        let base = register.offset();
        let shift = offset
            .checked_sub(base)
            .and_then(|value| value.checked_mul(8))
            .ok_or_else(|| DeviceError::new("SAM D21 EVSYS access offset overflow"))?;
        let bits = u64::from(width.bytes()) * 8;
        if shift + bits > 32 {
            return Err(DeviceError::new(format!(
                "SAM D21 EVSYS access crosses register boundary at {offset:#x}"
            )));
        }
        Ok((register, ((value & width.value_mask()) as u32) << shift))
    }
}

impl Device for Samd21Evsys {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("EVSYS lock poisoned");
        let (_, value) = Self::register_value(&state, offset, width)?;
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("EVSYS lock poisoned");
        let (register, merged) = Self::merged_write(&state, offset, width, value)?;
        let (_, payload) = Self::payload(offset, width, value)?;
        match register {
            Samd21EvsysRegister::Control => {
                if payload & 1 != 0 {
                    state.reset();
                } else {
                    state.generic_clock_request = payload & (1 << 4) != 0;
                }
            }
            Samd21EvsysRegister::Channel => {
                let channel = merged & CHANNEL_MASK;
                if channel >= u32::try_from(SAMD21_EVSYS_CHANNEL_COUNT).expect("channel count") {
                    return Err(DeviceError::new(format!(
                        "SAM D21 EVSYS channel index {channel} is out of range"
                    )));
                }
                state.selected_channel = channel as u8;
                let index = usize::from(state.selected_channel);
                state.channel_configured[index] = true;
                state.channels[index] = ChannelConfig {
                    event_generator: ((merged >> 16) & EVENT_GENERATOR_MASK) as u8,
                    path: ((merged >> 24) & 0x3) as u8,
                    edge: ((merged >> 26) & 0x3) as u8,
                };
                if payload & (1 << 8) != 0 {
                    let selected_channel = state.selected_channel;
                    state.trigger(selected_channel, true);
                }
            }
            Samd21EvsysRegister::User => {
                let user = merged & USER_MASK;
                let channel = (merged >> 8) & EVENT_CHANNEL_MASK;
                if user >= u32::try_from(SAMD21_EVSYS_USER_COUNT).expect("user count") {
                    return Err(DeviceError::new(format!(
                        "SAM D21 EVSYS user index {user} is outside the modeled range"
                    )));
                }
                if channel > u32::try_from(SAMD21_EVSYS_CHANNEL_COUNT).expect("channel count") {
                    return Err(DeviceError::new(format!(
                        "SAM D21 EVSYS user channel {channel} is outside the modeled range"
                    )));
                }
                state.selected_user = user as u8;
                state.user_channels[user as usize] = channel as u8;
            }
            Samd21EvsysRegister::ChannelStatus => {
                // Read-only on silicon.
            }
            Samd21EvsysRegister::InterruptEnableClear => {
                state.interrupt_enable &= !(payload & (EVD_MASK | OVR_MASK));
            }
            Samd21EvsysRegister::InterruptEnableSet => {
                state.interrupt_enable |= payload & (EVD_MASK | OVR_MASK);
            }
            Samd21EvsysRegister::InterruptFlags => {
                state.interrupt_flags &= !(payload & (EVD_MASK | OVR_MASK));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.lock().expect("EVSYS lock poisoned").reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn software_event_configures_channel_user_and_event_flag() {
        let (mut evsys, handle) = Samd21Evsys::new("evsys");
        evsys
            .write(0x00, AccessWidth::Byte, 1 << 4, SimTime::ZERO)
            .unwrap();
        evsys
            .write(
                0x04,
                AccessWidth::Word,
                2 | (0x36 << 16) | (1 << 26),
                SimTime::ZERO,
            )
            .unwrap();
        evsys
            .write(0x08, AccessWidth::HalfWord, 0x13 | (3 << 8), SimTime::ZERO)
            .unwrap();
        evsys
            .write(
                0x14,
                AccessWidth::Word,
                u64::from(evd_bit(2)),
                SimTime::ZERO,
            )
            .unwrap();
        evsys
            .write(0x04, AccessWidth::HalfWord, 2 | (1 << 8), SimTime::ZERO)
            .unwrap();

        assert_eq!(handle.event_count(2), 1);
        assert_eq!(handle.user_channel(0x13), Some(2));
        assert_eq!(handle.flags(), evd_bit(2));
        assert!(handle.interrupt_pending());
        assert_eq!(
            evsys.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
            2 | (0x36 << 16) | (1 << 26)
        );
        assert_eq!(
            evsys.read(0x0c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(CHSTATUS_RESET)
        );

        evsys
            .write(
                0x18,
                AccessWidth::Word,
                u64::from(evd_bit(2)),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(handle.flags(), 0);
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn named_register_ids_round_trip_native_offsets() {
        let registers = [
            Samd21EvsysRegister::Control,
            Samd21EvsysRegister::Channel,
            Samd21EvsysRegister::User,
            Samd21EvsysRegister::ChannelStatus,
            Samd21EvsysRegister::InterruptEnableClear,
            Samd21EvsysRegister::InterruptEnableSet,
            Samd21EvsysRegister::InterruptFlags,
        ];
        for register in registers {
            assert_eq!(
                Samd21EvsysRegister::from_offset(register.offset()),
                Some(register)
            );
        }
    }

    #[test]
    fn interrupt_masks_match_split_sam_d21_layout_and_w1_semantics() {
        let (mut evsys, handle) = Samd21Evsys::new("evsys");
        evsys
            .write(0x00, AccessWidth::Byte, 1 << 4, SimTime::ZERO)
            .unwrap();
        evsys
            .write(0x04, AccessWidth::Word, 2 | (1 << 26), SimTime::ZERO)
            .unwrap();
        evsys
            .write(
                0x14,
                AccessWidth::Word,
                u64::from(evd_bit(2) | (1 << 16)),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(handle.interrupt_pending() == false);
        evsys
            .write(0x04, AccessWidth::HalfWord, 2 | (1 << 8), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.flags(), evd_bit(2));
        assert!(handle.interrupt_pending());

        // A zero write must not clear a W1C register's existing state.
        evsys
            .write(0x18, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.flags(), evd_bit(2));
        evsys
            .write(
                0x18,
                AccessWidth::Word,
                u64::from(evd_bit(2)),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(handle.flags(), 0);

        // INTENCLR also consumes only the written one bits.
        evsys
            .write(0x10, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        assert!(handle.interrupt_pending() == false);
        evsys
            .write(
                0x14,
                AccessWidth::Word,
                u64::from(evd_bit(2)),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.interrupt_pending());
        evsys
            .write(
                0x10,
                AccessWidth::Word,
                u64::from(evd_bit(2)),
                SimTime::ZERO,
            )
            .unwrap();
    }

    #[test]
    fn asynchronous_channels_have_no_channel_status_or_event_interrupt() {
        let (mut evsys, handle) = Samd21Evsys::new("evsys");
        evsys
            .write(0x04, AccessWidth::Word, 2 | (2 << 24), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            evsys.read(0x0c, AccessWidth::Word, SimTime::ZERO).unwrap() & user_ready_bit(2) as u64,
            0
        );
        assert!(handle.trigger(2));
        assert_eq!(handle.flags(), 0);
    }

    #[test]
    fn user_mux_rejects_reserved_user_and_channel_values() {
        let (mut evsys, _) = Samd21Evsys::new("evsys");
        assert!(
            evsys
                .write(0x08, AccessWidth::HalfWord, 29, SimTime::ZERO)
                .is_err()
        );
        assert!(
            evsys
                .write(0x08, AccessWidth::HalfWord, 1 | (13 << 8), SimTime::ZERO)
                .is_err()
        );
    }

    #[test]
    fn asynchronous_software_event_counts_without_event_detected_flag() {
        let (mut evsys, handle) = Samd21Evsys::new("evsys");
        evsys
            .write(
                0x04,
                AccessWidth::Word,
                1 | (2 << 24) | (1 << 26),
                SimTime::ZERO,
            )
            .unwrap();
        evsys
            .write(0x04, AccessWidth::HalfWord, 1 | (1 << 8), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.event_count(1), 1);
        assert_eq!(handle.flags(), 0);
    }

    #[test]
    fn software_reset_returns_all_indexed_state_to_reset() {
        let (mut evsys, handle) = Samd21Evsys::new("evsys");
        evsys
            .write(0x08, AccessWidth::HalfWord, 0x13 | (3 << 8), SimTime::ZERO)
            .unwrap();
        evsys
            .write(0x00, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.user_channel(0x13), None);
        assert_eq!(
            evsys
                .read(0x08, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0
        );
    }
}
