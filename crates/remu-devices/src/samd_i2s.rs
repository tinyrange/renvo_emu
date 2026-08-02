use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const REGISTER_LIMIT: u64 = 0x38;
const CTRLA_MASK: u8 = 0x3f;
const CLKCTRL_MASK: u32 = 0xfffd_19ff;
const INTERRUPT_MASK: u16 = 0x3333;
const SYNCBUSY_MASK: u16 = 0x033f;
const SERCTRL_MASK: u32 = 0x07ff_f7bf;
const TX_READY_BITS: [u16; 2] = [1 << 8, 1 << 9];
const RX_READY_BITS: [u16; 2] = [1, 1 << 1];
const RX_OVERRUN_BITS: [u16; 2] = [1 << 4, 1 << 5];

/// Named SAM D21 I2S register identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Samd21I2sRegister {
    /// Module, clock-unit, and serializer enables.
    Ctrla = 0x00,
    /// Clock unit zero format and divider configuration.
    Clkctrl0 = 0x04,
    /// Clock unit one format and divider configuration.
    Clkctrl1 = 0x08,
    /// Interrupt-enable clear bitmap.
    Intenclr = 0x0c,
    /// Interrupt-enable set bitmap.
    Intenset = 0x10,
    /// Interrupt flags and write-one-to-clear bitmap.
    Intflag = 0x14,
    /// Synchronization status.
    Syncbusy = 0x18,
    /// Serializer zero formatting and mode configuration.
    Serctrl0 = 0x20,
    /// Serializer one formatting and mode configuration.
    Serctrl1 = 0x24,
    /// Serializer zero data holding register.
    Data0 = 0x30,
    /// Serializer one data holding register.
    Data1 = 0x34,
}

impl Samd21I2sRegister {
    /// Converts a documented I2S register offset to its named ID.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x00 => Some(Self::Ctrla),
            0x04 => Some(Self::Clkctrl0),
            0x08 => Some(Self::Clkctrl1),
            0x0c => Some(Self::Intenclr),
            0x10 => Some(Self::Intenset),
            0x14 => Some(Self::Intflag),
            0x18 => Some(Self::Syncbusy),
            0x20 => Some(Self::Serctrl0),
            0x24 => Some(Self::Serctrl1),
            0x30 => Some(Self::Data0),
            0x34 => Some(Self::Data1),
            _ => None,
        }
    }

    /// Returns the documented byte offset for this register.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    fn width(self) -> u8 {
        match self {
            Self::Ctrla => 1,
            Self::Clkctrl0 | Self::Clkctrl1 => 4,
            Self::Intenclr | Self::Intenset | Self::Intflag | Self::Syncbusy => 2,
            Self::Serctrl0 | Self::Serctrl1 | Self::Data0 | Self::Data1 => 4,
        }
    }

    fn locate(offset: u64) -> Option<(Self, u64, u8)> {
        let register = if offset == 0 {
            Self::Ctrla
        } else {
            Self::from_offset(offset & !0x3)?
        };
        let base = register.offset();
        let size = register.width();
        if offset >= base && offset < base + u64::from(size) {
            Some((register, base, size))
        } else {
            None
        }
    }

    fn is_write_one_register(self) -> bool {
        matches!(self, Self::Intenclr | Self::Intenset | Self::Intflag)
    }

    fn serializer(self) -> Option<usize> {
        match self {
            Self::Serctrl0 | Self::Data0 => Some(0),
            Self::Serctrl1 | Self::Data1 => Some(1),
            _ => None,
        }
    }
}

#[derive(Clone, Default)]
struct I2sState {
    ctrla: u8,
    clock: [u32; 2],
    interrupt_enable: u16,
    interrupt_flags: u16,
    syncbusy: u16,
    serializer: [u32; 2],
    data: [u32; 2],
    transmitted: Vec<(u8, u32)>,
}

impl I2sState {
    fn module_enabled(&self) -> bool {
        self.ctrla & (1 << 1) != 0
    }

    fn serializer_enabled(&self, serializer: usize) -> bool {
        self.ctrla & (1 << (4 + serializer)) != 0
    }

    fn clock_enabled(&self, clock: usize) -> bool {
        self.ctrla & (1 << (2 + clock)) != 0
    }

    fn serializer_active(&self, serializer: usize) -> bool {
        self.module_enabled()
            && self.clock_enabled(self.clock_for_serializer(serializer))
            && self.serializer_enabled(serializer)
    }

    fn transmit_mode(&self, serializer: usize) -> bool {
        self.serializer[serializer] & 0x3 == 1
    }

    fn receive_mode(&self, serializer: usize) -> bool {
        self.serializer[serializer] & 0x3 == 0
    }

    fn clock_for_serializer(&self, serializer: usize) -> usize {
        usize::try_from((self.serializer[serializer] >> 5) & 1).expect("clock selector fits usize")
    }

    fn refresh_ready_flags(&mut self) {
        for serializer in 0..2 {
            if self.serializer_active(serializer) && self.transmit_mode(serializer) {
                self.interrupt_flags |= TX_READY_BITS[serializer];
            } else {
                self.interrupt_flags &= !TX_READY_BITS[serializer];
            }
        }
    }

    fn write_data(&mut self, serializer: usize, value: u32) {
        self.data[serializer] = value;
        if self.serializer_active(serializer) && self.transmit_mode(serializer) {
            self.interrupt_flags &= !TX_READY_BITS[serializer];
            self.transmitted.push((
                u8::try_from(serializer).expect("serializer index fits u8"),
                value,
            ));
            // The functional model completes one abstract frame at the same
            // simulation instant, so a polling transmitter can continue.
            self.interrupt_flags |= TX_READY_BITS[serializer];
        }
    }

    fn read_data(&mut self, serializer: usize) -> u32 {
        let value = self.data[serializer];
        if self.receive_mode(serializer) {
            self.interrupt_flags &= !RX_READY_BITS[serializer];
        }
        value
    }
}

/// Host-facing SAM D21 I2S sample and interrupt handle.
#[derive(Clone)]
pub struct Samd21I2sHandle(Arc<Mutex<I2sState>>);

impl Samd21I2sHandle {
    /// Returns whether an enabled I2S interrupt is pending.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.0.lock().expect("I2S lock poisoned");
        state.interrupt_flags & state.interrupt_enable != 0
    }

    /// Returns captured transmitted samples as `(serializer, word)` pairs.
    pub fn transmitted(&self) -> Vec<(u8, u32)> {
        self.0
            .lock()
            .expect("I2S lock poisoned")
            .transmitted
            .clone()
    }

    /// Injects one received sample into a serializer's holding register.
    ///
    /// A second sample arriving before firmware reads the first one latches
    /// the corresponding receive-overrun flag and retains the original word.
    pub fn inject_rx(&self, serializer: u8, sample: u32) -> bool {
        let serializer = usize::from(serializer);
        if serializer >= 2 {
            return false;
        }
        let mut state = self.0.lock().expect("I2S lock poisoned");
        if !state.receive_mode(serializer) || !state.serializer_active(serializer) {
            return false;
        }
        if state.interrupt_flags & RX_READY_BITS[serializer] != 0 {
            state.interrupt_flags |= RX_OVERRUN_BITS[serializer];
        } else {
            state.data[serializer] = sample;
            state.interrupt_flags |= RX_READY_BITS[serializer];
        }
        true
    }

    /// Clears captured transmitted words after a host assertion consumes them.
    pub fn clear_transmitted(&self) {
        self.0
            .lock()
            .expect("I2S lock poisoned")
            .transmitted
            .clear();
    }
}

/// Functional SAM D21 two-clock, two-serializer I2S register slice.
pub struct Samd21I2s {
    name: String,
    state: Arc<Mutex<I2sState>>,
}

impl Samd21I2s {
    /// Constructs a reset I2S controller and its host sample handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21I2sHandle) {
        let state = Arc::new(Mutex::new(I2sState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Samd21I2sHandle(state),
        )
    }

    fn read_register(state: &I2sState, register: Samd21I2sRegister) -> u64 {
        match register {
            Samd21I2sRegister::Ctrla => u64::from(state.ctrla),
            Samd21I2sRegister::Clkctrl0 => u64::from(state.clock[0]),
            Samd21I2sRegister::Clkctrl1 => u64::from(state.clock[1]),
            Samd21I2sRegister::Intenclr | Samd21I2sRegister::Intenset => {
                u64::from(state.interrupt_enable)
            }
            Samd21I2sRegister::Intflag => u64::from(state.interrupt_flags),
            Samd21I2sRegister::Syncbusy => u64::from(state.syncbusy & SYNCBUSY_MASK),
            Samd21I2sRegister::Serctrl0 => u64::from(state.serializer[0]),
            Samd21I2sRegister::Serctrl1 => u64::from(state.serializer[1]),
            Samd21I2sRegister::Data0 => u64::from(state.data[0]),
            Samd21I2sRegister::Data1 => u64::from(state.data[1]),
        }
    }

    fn reset_state(state: &mut I2sState) {
        *state = I2sState::default();
    }

    fn write_register(state: &mut I2sState, register: Samd21I2sRegister, value: u64) {
        let value32 = value as u32;
        match register {
            Samd21I2sRegister::Ctrla => {
                if value & 1 != 0 {
                    Self::reset_state(state);
                } else {
                    state.ctrla = value as u8 & CTRLA_MASK & !1;
                    state.refresh_ready_flags();
                }
            }
            Samd21I2sRegister::Clkctrl0 => {
                state.clock[0] = value32 & CLKCTRL_MASK;
                state.refresh_ready_flags();
            }
            Samd21I2sRegister::Clkctrl1 => {
                state.clock[1] = value32 & CLKCTRL_MASK;
                state.refresh_ready_flags();
            }
            Samd21I2sRegister::Intenclr => {
                state.interrupt_enable &= !(value as u16 & INTERRUPT_MASK)
            }
            Samd21I2sRegister::Intenset => state.interrupt_enable |= value as u16 & INTERRUPT_MASK,
            Samd21I2sRegister::Intflag => state.interrupt_flags &= !(value as u16 & INTERRUPT_MASK),
            Samd21I2sRegister::Syncbusy => {}
            Samd21I2sRegister::Serctrl0 => {
                state.serializer[0] = value32 & SERCTRL_MASK;
                state.refresh_ready_flags();
            }
            Samd21I2sRegister::Serctrl1 => {
                state.serializer[1] = value32 & SERCTRL_MASK;
                state.refresh_ready_flags();
            }
            Samd21I2sRegister::Data0 => state.write_data(0, value32),
            Samd21I2sRegister::Data1 => state.write_data(1, value32),
        }
    }
}

impl Device for Samd21I2s {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let Some((register, base, size)) = Samd21I2sRegister::locate(offset) else {
            return if offset < REGISTER_LIMIT {
                Ok(0)
            } else {
                Err(DeviceError::new(format!(
                    "unmodeled I2S read at {offset:#x}"
                )))
            };
        };
        let end = offset
            .checked_add(u64::from(width.bytes()))
            .ok_or_else(|| DeviceError::new("I2S read offset overflow"))?;
        if end > base + u64::from(size) {
            return Err(DeviceError::new(format!("I2S read crosses {register:?}")));
        }
        let mut state = self.state.lock().expect("I2S lock poisoned");
        let value = if let Some(serializer) = register.serializer() {
            if matches!(
                register,
                Samd21I2sRegister::Data0 | Samd21I2sRegister::Data1
            ) {
                u64::from(state.read_data(serializer))
            } else {
                Self::read_register(&state, register)
            }
        } else {
            Self::read_register(&state, register)
        };
        Ok((value >> ((offset - base) * 8)) & width.value_mask())
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let Some((register, base, size)) = Samd21I2sRegister::locate(offset) else {
            if offset < REGISTER_LIMIT {
                return Ok(());
            }
            return Err(DeviceError::new(format!(
                "unmodeled I2S write at {offset:#x}"
            )));
        };
        let end = offset
            .checked_add(u64::from(width.bytes()))
            .ok_or_else(|| DeviceError::new("I2S write offset overflow"))?;
        if end > base + u64::from(size) {
            return Err(DeviceError::new(format!("I2S write crosses {register:?}")));
        }
        let mut state = self.state.lock().expect("I2S lock poisoned");
        let shift = (offset - base) * 8;
        let payload = (value & width.value_mask()) << shift;
        if register.is_write_one_register() {
            Self::write_register(&mut state, register, payload);
        } else {
            let old = Self::read_register(&state, register);
            let mask = width.value_mask() << shift;
            let merged = (old & !mask) | payload;
            Self::write_register(&mut state, register, merged);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        Self::reset_state(&mut self.state.lock().expect("I2S lock poisoned"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_registers_and_vendor_masks_are_enforced() {
        assert_eq!(
            Samd21I2sRegister::from_offset(0x00),
            Some(Samd21I2sRegister::Ctrla)
        );
        assert_eq!(Samd21I2sRegister::from_offset(0x1c), None);
        assert_eq!(Samd21I2sRegister::Data1.offset(), 0x34);

        let (mut i2s, _) = Samd21I2s::new("i2s");
        i2s.write(0x04, AccessWidth::Word, u64::MAX, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            i2s.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(CLKCTRL_MASK)
        );
        i2s.write(0x20, AccessWidth::Word, u64::MAX, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            i2s.read(0x20, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(SERCTRL_MASK)
        );
        i2s.write(0x10, AccessWidth::HalfWord, u64::MAX, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            i2s.read(0x10, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            u64::from(INTERRUPT_MASK)
        );
    }

    #[test]
    fn interrupt_w1c_uses_raw_payload_without_clearing_on_zero() {
        let (mut i2s, _) = Samd21I2s::new("i2s");
        i2s.write(0x04, AccessWidth::Word, 1 << 2, SimTime::ZERO)
            .unwrap();
        i2s.write(0x20, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        i2s.write(
            0x00,
            AccessWidth::Byte,
            (1 << 4) | (1 << 2) | (1 << 1),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            i2s.read(0x14, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            1 << 8
        );
        i2s.write(0x14, AccessWidth::HalfWord, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            i2s.read(0x14, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            1 << 8
        );
        i2s.write(0x14, AccessWidth::HalfWord, 1 << 8, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            i2s.read(0x14, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0
        );
    }

    #[test]
    fn named_clock_serializer_and_tx_registers_capture_samples() {
        let (mut i2s, handle) = Samd21I2s::new("i2s");
        i2s.write(
            0x04,
            AccessWidth::Word,
            (1 << 7) | (1 << 5) | (1 << 2) | 1,
            SimTime::ZERO,
        )
        .unwrap();
        i2s.write(0x20, AccessWidth::Word, (4 << 8) | 1, SimTime::ZERO)
            .unwrap();
        i2s.write(
            0x00,
            AccessWidth::Byte,
            (1 << 4) | (1 << 2) | (1 << 1),
            SimTime::ZERO,
        )
        .unwrap();
        i2s.write(0x10, AccessWidth::HalfWord, 1 << 8, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            i2s.read(0x18, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0
        );
        assert_eq!(
            i2s.read(0x14, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            1 << 8
        );
        i2s.write(0x30, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.transmitted(), vec![(0, 0x1234_5678)]);
        assert_eq!(
            i2s.read(0x30, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x1234_5678
        );
        assert!(handle.interrupt_pending());
        i2s.write(0x14, AccessWidth::HalfWord, 1 << 8, SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn host_rx_injection_latches_ready_and_overrun_until_read_or_w1c() {
        let (mut i2s, handle) = Samd21I2s::new("i2s");
        i2s.write(0x20, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        i2s.write(
            0x00,
            AccessWidth::Byte,
            (1 << 4) | (1 << 2) | (1 << 1),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.inject_rx(0, 0xaabb_ccdd));
        assert!(handle.inject_rx(0, 0x1122_3344));
        assert_eq!(
            i2s.read(0x14, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            1 | (1 << 4)
        );
        assert_eq!(
            i2s.read(0x30, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xaabb_ccdd
        );
        assert_eq!(
            i2s.read(0x14, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            1 << 4
        );
        i2s.write(0x14, AccessWidth::HalfWord, 1 << 4, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            i2s.read(0x14, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0
        );
    }
}
