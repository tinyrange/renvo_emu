use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};

const SLICE_COUNT: usize = 12;
const SLICE_STRIDE: u64 = 0x14;

#[derive(Clone, Copy, Default)]
struct PwmSlice {
    csr: u32,
    div: u32,
    ctr: u32,
    cc: u32,
    top: u32,
}

/// Functional RP2040/RP2350 PWM register block.
///
/// The model preserves the twelve slice control, divider, counter, compare,
/// and wrap registers, plus the global interrupt/status registers. It is a
/// deterministic register model; edge-level PWM waveform scheduling is left
/// to a future signal-connected implementation.
pub struct RpPwm {
    name: String,
    slice_count: usize,
    slices: [PwmSlice; SLICE_COUNT],
    intr: u32,
    inte: u32,
    intf: u32,
    irq1_inte: u32,
    irq1_intf: u32,
    has_irq1: bool,
}

impl RpPwm {
    /// Creates a reset PWM block for an RP2040-compatible target.
    pub fn new(name: impl Into<String>) -> Self {
        Self::with_config(name.into(), 8, false)
    }

    /// Creates a reset PWM block for an RP2350-compatible target.
    pub fn new_rp2350(name: impl Into<String>) -> Self {
        Self::with_config(name.into(), SLICE_COUNT, true)
    }

    fn with_config(name: String, slice_count: usize, has_irq1: bool) -> Self {
        Self {
            name,
            slice_count,
            slices: [PwmSlice {
                div: 0x10,
                top: u16::MAX.into(),
                ..PwmSlice::default()
            }; SLICE_COUNT],
            intr: 0,
            inte: 0,
            intf: 0,
            irq1_inte: 0,
            irq1_intf: 0,
            has_irq1,
        }
    }

    fn enabled_mask(&self) -> u32 {
        self.slices[..self.slice_count]
            .iter()
            .enumerate()
            .fold(0, |mask, (slice, state)| mask | ((state.csr & 1) << slice))
    }

    fn slice_register(&self, offset: u64) -> Option<(usize, u64)> {
        if offset >= self.slice_count as u64 * SLICE_STRIDE {
            return None;
        }
        let slice = usize::try_from(offset / SLICE_STRIDE).expect("PWM slice index fits");
        Some((slice, offset % SLICE_STRIDE))
    }

    fn global_base(&self) -> u64 {
        self.slice_count as u64 * SLICE_STRIDE
    }

    fn access(width: AccessWidth, offset: u64) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RP PWM requires aligned word access"));
        }
        Ok(())
    }
}

impl Device for RpPwm {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::access(width, offset)?;
        let value = if let Some((slice, register)) = self.slice_register(offset) {
            let state = self.slices[slice];
            match register {
                0x00 => state.csr,
                0x04 => state.div,
                0x08 => state.ctr,
                0x0c => state.cc,
                0x10 => state.top,
                _ => unreachable!("PWM slice stride is register aligned"),
            }
        } else {
            match offset - self.global_base() {
                0x00 => self.enabled_mask(),
                0x04 => self.intr,
                0x08 => self.inte,
                0x0c => self.intf,
                0x10 => (self.intr & self.inte) | self.intf,
                0x14 if self.has_irq1 => self.irq1_inte,
                0x18 if self.has_irq1 => self.irq1_intf,
                0x1c if self.has_irq1 => (self.intr & self.irq1_inte) | self.irq1_intf,
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled RP PWM read at {offset:#x}"
                    )));
                }
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
        Self::access(width, offset)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("PWM value fits");
        if let Some((slice, register)) = self.slice_register(offset) {
            let state = &mut self.slices[slice];
            match register {
                0x00 => state.csr = value & 0x3f,
                0x04 => state.div = value & 0x0fff,
                0x08 => state.ctr = value & 0xffff,
                0x0c => state.cc = value,
                0x10 => state.top = value & 0xffff,
                _ => unreachable!("PWM slice stride is register aligned"),
            }
            return Ok(());
        }
        match offset - self.global_base() {
            0x00 => {
                let enabled = value & ((1_u32 << self.slice_count) - 1);
                for (slice, state) in self.slices[..self.slice_count].iter_mut().enumerate() {
                    state.csr = (state.csr & !1) | ((enabled >> slice) & 1);
                }
            }
            0x04 => self.intr &= !value,
            0x08 => self.inte = value & 0x0fff,
            0x0c => self.intf = value & 0x0fff,
            0x14 if self.has_irq1 => self.irq1_inte = value & 0x0fff,
            0x18 if self.has_irq1 => self.irq1_intf = value & 0x0fff,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP PWM write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let name = self.name.clone();
        *self = if self.has_irq1 {
            Self::new_rp2350(name)
        } else {
            Self::new(name)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_and_global_interrupt_registers_round_trip() {
        let mut pwm = RpPwm::new_rp2350("rp2350.pwm");
        pwm.write(0x00, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pwm.write(0x04, AccessWidth::Word, 0x0000_f123, SimTime::ZERO)
            .unwrap();
        pwm.write(0x0c, AccessWidth::Word, 0x0200_0100, SimTime::ZERO)
            .unwrap();
        pwm.write(0x10, AccessWidth::Word, 999, SimTime::ZERO)
            .unwrap();
        pwm.write(0xf0, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pwm.write(0xf8, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pwm.write(0xfc, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pwm.write(0x104, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pwm.write(0x108, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(pwm.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
        assert_eq!(
            pwm.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x123
        );
        assert_eq!(
            pwm.read(0x0c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x0200_0100
        );
        assert_eq!(
            pwm.read(0x10, AccessWidth::Word, SimTime::ZERO).unwrap(),
            999
        );
        assert_eq!(pwm.read(0xf0, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
        assert_eq!(
            pwm.read(0x100, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1
        );
        assert_eq!(
            pwm.read(0x10c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1
        );
    }

    #[test]
    fn rp2040_uses_eight_slices_and_shared_enable_alias() {
        let mut pwm = RpPwm::new("rp2040.pwm");
        pwm.write(0xa0, AccessWidth::Word, 1 << 7, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            pwm.read(0xa0, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 7
        );
        assert_eq!(pwm.read(0x8c, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
        assert!(
            pwm.write(0xf0, AccessWidth::Word, 1, SimTime::ZERO)
                .is_err()
        );
    }
}
