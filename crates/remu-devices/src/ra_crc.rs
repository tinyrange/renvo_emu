//! RA4M1 CRC calculator peripheral.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const CRCCR0: u64 = 0x00;
const CRCCR1: u64 = 0x01;
const CRCDIR: u64 = 0x04;
const CRCDOR: u64 = 0x08;
const CRCDOR_END: u64 = 0x0b;
const CRCSAR: u64 = 0x0c;
const CRCSAR_HI: u64 = 0x0d;
const GPS_MASK: u8 = 0x07;
const LMS: u8 = 1 << 6;
const DORCLR: u8 = 1 << 7;

#[derive(Default)]
struct CrcState {
    control0: u8,
    control1: u8,
    snoop_address: u16,
    value: u32,
}

impl CrcState {
    fn width(&self) -> u8 {
        match self.control0 & GPS_MASK {
            1 => 8,
            2 | 3 => 16,
            4 | 5 => 32,
            _ => 0,
        }
    }

    fn polynomial(&self) -> u32 {
        match self.control0 & GPS_MASK {
            1 => 0x07,
            2 => 0x8005,
            3 => 0x1021,
            4 => 0x04c1_1db7,
            5 => 0x1edc_6f41,
            _ => 0,
        }
    }

    fn reflected_polynomial(&self) -> u32 {
        match self.control0 & GPS_MASK {
            1 => 0xe0,
            2 => 0xa001,
            3 => 0x8408,
            4 => 0xedb8_8320,
            5 => 0x82f6_3b78,
            _ => 0,
        }
    }

    fn feed(&mut self, byte: u8) {
        let width = self.width();
        if width == 0 {
            return;
        }
        let mask = if width == 32 {
            u32::MAX
        } else {
            (1_u32 << width) - 1
        };
        if self.control0 & LMS != 0 {
            self.value ^= u32::from(byte) << (width - 8);
            for _ in 0..8 {
                if self.value & (1_u32 << (width - 1)) != 0 {
                    self.value = (self.value << 1) ^ self.polynomial();
                } else {
                    self.value <<= 1;
                }
                self.value &= mask;
            }
        } else {
            self.value ^= u32::from(byte);
            for _ in 0..8 {
                if self.value & 1 != 0 {
                    self.value = (self.value >> 1) ^ self.reflected_polynomial();
                } else {
                    self.value >>= 1;
                }
            }
            self.value &= mask;
        }
    }

    fn output(&self) -> u32 {
        let width = self.width();
        if width == 32 {
            self.value
        } else if width == 0 {
            0
        } else {
            self.value & ((1_u32 << width) - 1)
        }
    }
}

/// Host-facing RA4M1 CRC state.
#[derive(Clone)]
pub struct RaCrcHandle(Arc<Mutex<CrcState>>);

impl RaCrcHandle {
    /// Returns the currently selected CRC result.
    pub fn value(&self) -> u32 {
        self.0.lock().expect("RA CRC lock poisoned").output()
    }
}

/// Functional RA4M1 CRC-8/16/CCITT/32/32C calculator.
pub struct RaCrc {
    name: String,
    state: Arc<Mutex<CrcState>>,
}

impl RaCrc {
    /// Creates a reset CRC calculator.
    pub fn new(name: impl Into<String>) -> (Self, RaCrcHandle) {
        let state = Arc::new(Mutex::new(CrcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaCrcHandle(state),
        )
    }

    fn read_byte(&self, offset: u64) -> u8 {
        let state = self.state.lock().expect("RA CRC lock poisoned");
        match offset {
            CRCCR0 => state.control0,
            CRCCR1 => state.control1,
            CRCDOR..=CRCDOR_END => (state.output() >> ((offset - CRCDOR) * 8)) as u8,
            CRCSAR => state.snoop_address as u8,
            CRCSAR_HI => (state.snoop_address >> 8) as u8,
            _ => 0,
        }
    }
}

impl Device for RaCrc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width == AccessWidth::DoubleWord {
            return Err(DeviceError::new(
                "RA CRC does not support double-word accesses",
            ));
        }
        let mut value = 0_u64;
        for byte in 0..u64::from(width.bytes()) {
            value |= u64::from(self.read_byte(offset + byte)) << (byte * 8);
        }
        Ok(value)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width == AccessWidth::DoubleWord {
            return Err(DeviceError::new(
                "RA CRC does not support double-word accesses",
            ));
        }
        let mut state = self.state.lock().expect("RA CRC lock poisoned");
        match offset {
            CRCCR0 => {
                let value = value as u8;
                if value & DORCLR != 0 {
                    state.value = 0;
                }
                state.control0 = value & !DORCLR;
            }
            CRCCR1 => state.control1 = value as u8,
            CRCDIR => {
                for byte in 0..width.bytes() {
                    state.feed((value >> (u64::from(byte) * 8)) as u8);
                }
            }
            CRCDOR => state.value = value as u32,
            CRCSAR => state.snoop_address = value as u16,
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA CRC lock poisoned") = CrcState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc8_msb_and_lsb_modes_feed_data_register() {
        let (mut crc, handle) = RaCrc::new("crc");
        crc.write(CRCCR0, AccessWidth::Byte, (1 | LMS).into(), SimTime::ZERO)
            .unwrap();
        crc.write(CRCDIR, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), 0x07);
        crc.write(CRCCR0, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        crc.write(CRCDOR, AccessWidth::Byte, 0, SimTime::ZERO)
            .unwrap();
        crc.write(CRCDIR, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), 0x91);
    }

    #[test]
    fn crc16_ccitt_result_can_be_seeded_and_cleared() {
        let (mut crc, handle) = RaCrc::new("crc");
        crc.write(CRCCR0, AccessWidth::Byte, (3 | LMS).into(), SimTime::ZERO)
            .unwrap();
        crc.write(CRCDOR, AccessWidth::HalfWord, 0xffff, SimTime::ZERO)
            .unwrap();
        crc.write(CRCDIR, AccessWidth::Byte, b'A'.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.value(), 0xb915);
        crc.write(
            CRCCR0,
            AccessWidth::Byte,
            (3 | LMS | DORCLR).into(),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.value(), 0);
    }
}
