//! STM32L4 CRC calculation peripheral subset.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const DR: u64 = 0x00;
const IDR: u64 = 0x04;
const CR: u64 = 0x08;
const INIT: u64 = 0x10;
const POL: u64 = 0x14;
const RESET: u32 = 1;

#[derive(Default)]
struct CrcState {
    data: u32,
    idr: u8,
    control: u32,
    init: u32,
    polynomial: u32,
}

impl CrcState {
    fn polynomial_width(&self) -> u32 {
        match (self.control >> 3) & 3 {
            1 => 16,
            2 => 8,
            3 => 7,
            _ => 32,
        }
    }

    fn polynomial_mask(&self) -> u32 {
        match self.polynomial_width() {
            32 => u32::MAX,
            width => (1_u32 << width) - 1,
        }
    }

    fn reset_data(&mut self) {
        self.data = self.init & self.polynomial_mask();
    }

    fn reverse_bits(value: u32, bits: u32) -> u32 {
        value.reverse_bits() >> (32 - bits)
    }

    fn reverse_input(value: u32, bits: u32, mode: u32) -> u32 {
        match mode {
            1 => {
                let bytes = bits / 8;
                let mut result = 0;
                for index in 0..bytes {
                    let shift = (bytes - index - 1) * 8;
                    let byte = ((value >> shift) & 0xff) as u8;
                    result |= u32::from(byte.reverse_bits()) << shift;
                }
                result
            }
            2 if bits == 32 => {
                Self::reverse_bits(value >> 16, 16) << 16 | Self::reverse_bits(value & 0xffff, 16)
            }
            2 | 3 => Self::reverse_bits(value, bits),
            _ => value,
        }
    }

    fn feed(&mut self, value: u32, width: AccessWidth) {
        let input_bits = match width {
            AccessWidth::Byte => 8,
            AccessWidth::HalfWord => 16,
            AccessWidth::Word => 32,
            AccessWidth::DoubleWord => unreachable!("CRC data register is at most 32 bits"),
        };
        let input_mask = if input_bits == 32 {
            u32::MAX
        } else {
            (1_u32 << input_bits) - 1
        };
        let input = Self::reverse_input(value & input_mask, input_bits, (self.control >> 5) & 3);
        let width = self.polynomial_width();
        let mask = self.polynomial_mask();
        let polynomial = self.polynomial & mask;
        for bit in (0..input_bits).rev() {
            let incoming = (input >> bit) & 1;
            let top = (self.data >> (width - 1)) & 1;
            self.data = (self.data << 1) & mask;
            if top ^ incoming != 0 {
                self.data ^= polynomial;
            }
        }
    }

    fn output(&self) -> u32 {
        let value = self.data & self.polynomial_mask();
        if self.control & (1 << 7) != 0 {
            Self::reverse_bits(value, self.polynomial_width())
        } else {
            value
        }
    }
}

/// Host-facing STM32 CRC state.
#[derive(Clone)]
pub struct Stm32CrcHandle(Arc<Mutex<CrcState>>);

impl Stm32CrcHandle {
    /// Returns the current CRC data register value.
    pub fn value(&self) -> u32 {
        self.0.lock().expect("STM32 CRC lock poisoned").output()
    }

    /// Seeds the CRC data register from a host test.
    pub fn seed(&self, value: u32) {
        let mut state = self.0.lock().expect("STM32 CRC lock poisoned");
        state.data = value & state.polynomial_mask();
    }
}

/// Functional STM32L432 CRC register block.
pub struct Stm32Crc {
    name: String,
    state: Arc<Mutex<CrcState>>,
}

impl Stm32Crc {
    /// Creates a CRC block with the STM32L4 reset polynomial and seed.
    pub fn new(name: impl Into<String>) -> (Self, Stm32CrcHandle) {
        let state = Arc::new(Mutex::new(CrcState {
            init: u32::MAX,
            polynomial: 0x04c1_1db7,
            data: u32::MAX,
            ..CrcState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32CrcHandle(state),
        )
    }
}

impl Device for Stm32Crc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if (offset == DR && width == AccessWidth::DoubleWord)
            || (offset != DR && (width != AccessWidth::Word || offset & 3 != 0))
        {
            return Err(DeviceError::new(
                "STM32 CRC control registers require aligned word access",
            ));
        }
        let state = self.state.lock().expect("STM32 CRC lock poisoned");
        let value = match offset {
            DR => {
                let value = state.output();
                match width {
                    AccessWidth::Byte => value & 0xff,
                    AccessWidth::HalfWord => value & 0xffff,
                    AccessWidth::Word => value,
                    AccessWidth::DoubleWord => unreachable!("validated above"),
                }
            }
            IDR => u32::from(state.idr),
            CR => state.control,
            INIT => state.init,
            POL => state.polynomial & state.polynomial_mask(),
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32 CRC read at {offset:#x}"
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
        if (offset == DR && width == AccessWidth::DoubleWord)
            || (offset != DR && (width != AccessWidth::Word || offset & 3 != 0))
        {
            return Err(DeviceError::new(
                "STM32 CRC control registers require aligned word access",
            ));
        }
        let mut state = self.state.lock().expect("STM32 CRC lock poisoned");
        let value = value as u32;
        match offset {
            DR => state.feed(value, width),
            IDR => state.idr = value as u8,
            CR => {
                state.control = value & 0x0f8;
                if value & RESET != 0 {
                    state.reset_data();
                }
            }
            INIT => state.init = value,
            POL => state.polynomial = value,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32 CRC write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("STM32 CRC lock poisoned");
        state.control = 0;
        state.init = u32::MAX;
        state.polynomial = 0x04c1_1db7;
        state.data = state.init;
        state.idr = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_and_feed_update_crc_data() {
        let (mut crc, handle) = Stm32Crc::new("crc");
        crc.write(CR, AccessWidth::Word, RESET.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), u32::MAX);
        crc.write(DR, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), 0xdf8a_8a2b);
        assert_eq!(
            crc.read(DR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32,
            handle.value()
        );
    }

    #[test]
    fn custom_seed_and_polynomial_are_visible() {
        let (mut crc, handle) = Stm32Crc::new("crc");
        crc.write(INIT, AccessWidth::Word, 0xfeed_beef, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), u32::MAX);
        crc.write(CR, AccessWidth::Word, RESET.into(), SimTime::ZERO)
            .unwrap();
        crc.write(POL, AccessWidth::Word, 0x1d, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), 0xfeed_beef);
        assert_eq!(
            crc.read(POL, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x1d
        );
    }

    #[test]
    fn data_register_accepts_right_aligned_byte_and_halfword_writes() {
        let (mut crc, handle) = Stm32Crc::new("crc");
        crc.write(DR, AccessWidth::Byte, 0x12, SimTime::ZERO)
            .unwrap();
        let after_byte = handle.value();
        crc.write(CR, AccessWidth::Word, RESET.into(), SimTime::ZERO)
            .unwrap();
        crc.write(DR, AccessWidth::HalfWord, 0x1234, SimTime::ZERO)
            .unwrap();
        assert_ne!(after_byte, handle.value());
        assert_eq!(
            crc.read(DR, AccessWidth::HalfWord, SimTime::ZERO).unwrap() as u32,
            handle.value() & 0xffff
        );
    }

    #[test]
    fn input_and_output_reversal_follow_st_hal_definitions() {
        let (mut crc, handle) = Stm32Crc::new("crc");
        // REV_IN=half-word maps 0x1A2B3C4D to 0xD458B23C.
        crc.write(
            CR,
            AccessWidth::Word,
            u64::from((2 << 5) | RESET),
            SimTime::ZERO,
        )
        .unwrap();
        crc.write(DR, AccessWidth::Word, 0x1a2b_3c4d, SimTime::ZERO)
            .unwrap();
        let normal = handle.value();
        crc.write(CR, AccessWidth::Word, (2 << 5) | (1 << 7), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), normal.reverse_bits());
        crc.write(CR, AccessWidth::Word, 1 << 5, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), normal);
    }
}
