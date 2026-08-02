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
    en: u32,
    intr: u32,
    inte: u32,
    intf: u32,
}

impl RpPwm {
    /// Creates a reset PWM block for an RP2040-compatible target.
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        let slice_count = if name.starts_with("rp2040") {
            8
        } else {
            SLICE_COUNT
        };
        Self {
            name,
            slice_count,
            slices: [PwmSlice {
                top: u16::MAX.into(),
                ..PwmSlice::default()
            }; SLICE_COUNT],
            en: 0,
            intr: 0,
            inte: 0,
            intf: 0,
        }
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
                0x00 => self.en,
                0x04 => self.intr,
                0x08 => self.inte,
                0x0c => self.intf,
                0x10 => (self.intr | self.intf) & (self.inte | self.intf),
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
                0x00 => state.csr = value & 0x1f,
                0x04 => state.div = value & 0x00ff_ffff,
                0x08 => state.ctr = value & 0xffff,
                0x0c => state.cc = value,
                0x10 => state.top = value & 0xffff,
                _ => unreachable!("PWM slice stride is register aligned"),
            }
            return Ok(());
        }
        match offset - self.global_base() {
            0x00 => self.en = value & 0x0fff,
            0x04 => self.intr &= !value,
            0x08 => self.inte = value & 0x0fff,
            0x0c => self.intf = value & 0x0fff,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP PWM write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_and_global_interrupt_registers_round_trip() {
        let mut pwm = RpPwm::new("pwm");
        pwm.write(0x00, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pwm.write(0x0c, AccessWidth::Word, 0x0200_0100, SimTime::ZERO)
            .unwrap();
        pwm.write(0x10, AccessWidth::Word, 999, SimTime::ZERO)
            .unwrap();
        pwm.write(0xf0, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        pwm.write(0xf8, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(pwm.read(0x00, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
        assert_eq!(
            pwm.read(0x0c, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x0200_0100
        );
        assert_eq!(
            pwm.read(0x10, AccessWidth::Word, SimTime::ZERO).unwrap(),
            999
        );
        assert_eq!(pwm.read(0xf0, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
    }
}
