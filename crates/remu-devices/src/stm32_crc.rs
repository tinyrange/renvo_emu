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
    fn reset_data(&mut self) {
        self.data = self.init;
    }

    fn feed_word(&mut self, word: u32) {
        let width = match (self.control >> 3) & 3 {
            1 => 16,
            2 => 8,
            3 => 7,
            _ => 32,
        };
        let mask = if width == 32 {
            u32::MAX
        } else {
            (1_u32 << width) - 1
        };
        let polynomial = self.polynomial & mask;
        let reverse_input = (self.control >> 5) & 3;
        let mut value = word;
        let bytes = if reverse_input == 0 { 4 } else { 1 };
        for _ in 0..bytes {
            let byte = (value & 0xff) as u8;
            value >>= 8;
            let input = if reverse_input == 1 {
                byte.reverse_bits()
            } else {
                byte
            };
            for bit in 0..8 {
                let incoming = u32::from((input >> (7 - bit)) & 1);
                let top = (self.data >> (width - 1)) & 1;
                self.data = (self.data << 1) & mask;
                if top ^ incoming != 0 {
                    self.data ^= polynomial;
                }
            }
        }
        if self.control & (1 << 7) != 0 {
            self.data = self.data.reverse_bits() >> (32 - width);
        }
    }
}

/// Host-facing STM32 CRC state.
#[derive(Clone)]
pub struct Stm32CrcHandle(Arc<Mutex<CrcState>>);

impl Stm32CrcHandle {
    /// Returns the current CRC data register value.
    pub fn value(&self) -> u32 {
        self.0.lock().expect("STM32 CRC lock poisoned").data
    }

    /// Seeds the CRC data register from a host test.
    pub fn seed(&self, value: u32) {
        self.0.lock().expect("STM32 CRC lock poisoned").data = value;
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("STM32 CRC requires aligned word access"));
        }
        let state = self.state.lock().expect("STM32 CRC lock poisoned");
        let value = match offset {
            DR => state.data,
            IDR => u32::from(state.idr),
            CR => state.control,
            INIT => state.init,
            POL => state.polynomial,
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("STM32 CRC requires aligned word access"));
        }
        let mut state = self.state.lock().expect("STM32 CRC lock poisoned");
        let value = value as u32;
        match offset {
            DR => state.feed_word(value),
            IDR => state.idr = value as u8,
            CR => {
                state.control = value & 0x0f8;
                if value & RESET != 0 {
                    state.reset_data();
                }
            }
            INIT => {
                state.init = value;
                state.reset_data();
            }
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
        let initial = handle.value();
        crc.write(DR, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        assert_ne!(handle.value(), initial);
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
        crc.write(POL, AccessWidth::Word, 0x1d, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), 0xfeed_beef);
        assert_eq!(
            crc.read(POL, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x1d
        );
    }
}
