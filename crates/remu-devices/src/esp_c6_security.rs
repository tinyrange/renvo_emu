use super::*;
use num_bigint::BigUint;

const EFUSE_CONF: u64 = 0x1cc;
const EFUSE_CMD: u64 = 0x1d4;
const EFUSE_INT_RAW: u64 = 0x1d8;
const EFUSE_INT_STATUS: u64 = 0x1dc;
const EFUSE_INT_ENABLE: u64 = 0x1e0;
const EFUSE_INT_CLEAR: u64 = 0x1e4;
const EFUSE_DATE: u64 = 0x1fc;

/// Functional ESP32-C6 eFuse controller with persistent one-time-programmable blocks.
pub struct EspC6Efuse {
    name: String,
    staging: [u32; 8],
    blocks: [[u32; 8]; 11],
    opcode: u16,
    interrupts: u32,
    interrupt_enable: u32,
    registers: [u32; 0x200 / 4],
}

impl EspC6Efuse {
    /// Creates an eFuse controller with a deterministic locally administered MAC identity.
    pub fn new(name: impl Into<String>) -> Self {
        let mut device = Self {
            name: name.into(),
            staging: [0; 8],
            blocks: [[0; 8]; 11],
            opcode: 0,
            interrupts: 0,
            interrupt_enable: 0,
            registers: [0; 0x200 / 4],
        };
        device.blocks[1][0] = 0x0200_0000;
        device.blocks[1][1] = 0x00c6_5245;
        device.registers[0x1e8 / 4] = 0x0001_fe1c;
        device.registers[0x1ec / 4] = 0x1201_0201;
        device.registers[0x1f0 / 4] = 0x0130_0001;
        device.registers[0x1f4 / 4] = 0x00c8_0190;
        device.registers[0x1f8 / 4] = 1 << 13;
        device.registers[EFUSE_DATE as usize / 4] = 35_676_928;
        device
    }

    fn read_word(&self, offset: u64) -> Option<u32> {
        let mapping = match offset {
            0x2c..=0x40 => Some((0, (offset - 0x2c) / 4)),
            0x44..=0x58 => Some((1, (offset - 0x44) / 4)),
            0x5c..=0x78 => Some((2, (offset - 0x5c) / 4)),
            0x7c..=0x98 => Some((3, (offset - 0x7c) / 4)),
            0x9c..=0xb8 => Some((4, (offset - 0x9c) / 4)),
            0xbc..=0xd8 => Some((5, (offset - 0xbc) / 4)),
            0xdc..=0xf8 => Some((6, (offset - 0xdc) / 4)),
            0xfc..=0x118 => Some((7, (offset - 0xfc) / 4)),
            0x11c..=0x138 => Some((8, (offset - 0x11c) / 4)),
            0x13c..=0x158 => Some((9, (offset - 0x13c) / 4)),
            0x15c..=0x178 => Some((10, (offset - 0x15c) / 4)),
            _ => None,
        }?;
        let (block, word) = mapping;
        let read_disabled = block >= 4 && self.blocks[0][1] & (1 << (block - 4)) != 0;
        Some(if read_disabled {
            0
        } else {
            self.blocks[block][word as usize]
        })
    }

    fn program(&mut self, block: usize) -> Result<(), DeviceError> {
        if self.opcode != 0x5a5a || block >= self.blocks.len() {
            return Err(DeviceError::new(format!(
                "{} rejected eFuse program command",
                self.name
            )));
        }
        for (fuse, staged) in self.blocks[block].iter_mut().zip(self.staging) {
            *fuse |= staged;
        }
        self.staging.fill(0);
        self.interrupts |= 2;
        Ok(())
    }
}

impl Device for EspC6Efuse {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 eFuse requires aligned word access",
            ));
        }
        let value = match offset {
            0x00..=0x1c => self.staging[offset as usize / 4],
            EFUSE_CONF => u32::from(self.opcode),
            EFUSE_CMD => 0,
            EFUSE_INT_RAW => self.interrupts,
            EFUSE_INT_STATUS => self.interrupts & self.interrupt_enable,
            EFUSE_INT_ENABLE => self.interrupt_enable,
            EFUSE_INT_CLEAR => 0,
            _ => self
                .read_word(offset)
                .or_else(|| {
                    self.registers
                        .get(offset as usize / 4)
                        .copied()
                        .filter(|_| matches!(offset, 0x1c8 | 0x1d0 | 0x1e8..=0x1fc))
                })
                .ok_or_else(|| {
                    DeviceError::new(format!("{} reserved read {offset:#x}", self.name))
                })?,
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
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 eFuse requires aligned word access",
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 eFuse rejects wide writes"))?;
        match offset {
            0x00..=0x1c => self.staging[offset as usize / 4] = value,
            EFUSE_CONF => self.opcode = value as u16,
            EFUSE_CMD if value & 1 != 0 => {
                if self.opcode != 0x5aa5 {
                    return Err(DeviceError::new("invalid ESP32-C6 eFuse read opcode"));
                }
                self.interrupts |= 1;
            }
            EFUSE_CMD if value & 2 != 0 => self.program(((value >> 2) & 0xf) as usize)?,
            EFUSE_CMD => {}
            EFUSE_INT_RAW => self.interrupts &= !(value & 3),
            EFUSE_INT_ENABLE => self.interrupt_enable = value & 3,
            EFUSE_INT_CLEAR => self.interrupts &= !(value & 3),
            0x1c8 | 0x1e8..=0x1fc => self.registers[offset as usize / 4] = value,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved write {offset:#x}",
                    self.name
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.staging.fill(0);
        self.opcode = 0;
        self.interrupts = 0;
        self.interrupt_enable = 0;
    }
}

#[derive(Clone, Debug)]
enum EccPoint {
    Infinity,
    Coordinate(BigUint, BigUint),
}

/// Functional ESP32-C6 ECC accelerator for the native P-192/P-256 point operations.
pub struct EspC6Ecc {
    name: String,
    interrupt: bool,
    interrupt_enable: bool,
    config: u32,
    k: [u32; 8],
    x: [u32; 8],
    y: [u32; 8],
    date: u32,
}

impl EspC6Ecc {
    /// Creates an idle ECC accelerator.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            interrupt: false,
            interrupt_enable: false,
            config: 1 << 31,
            k: [0; 8],
            x: [0; 8],
            y: [0; 8],
            date: 35_656_256,
        }
    }

    fn integer(words: &[u32]) -> BigUint {
        BigUint::from_bytes_le(
            &words
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<_>>(),
        )
    }

    fn store(words: &mut [u32], value: &BigUint) {
        words.fill(0);
        for (index, chunk) in value.to_bytes_le().chunks(4).enumerate().take(words.len()) {
            let mut bytes = [0; 4];
            bytes[..chunk.len()].copy_from_slice(chunk);
            words[index] = u32::from_le_bytes(bytes);
        }
    }

    fn parameter(text: &[u8]) -> BigUint {
        BigUint::parse_bytes(text, 16).expect("constant is valid hexadecimal")
    }

    fn curve(&self) -> (BigUint, BigUint) {
        let (p, b) = if self.config & (1 << 2) != 0 {
            (
                b"FFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF".as_slice(),
                b"5AC635D8AA3A93E7B3EBBD55769886BC651D06B0CC53B0F63BCE3C3E27D2604B".as_slice(),
            )
        } else {
            (
                b"FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEFFFFFFFFFFFFFFFF".as_slice(),
                b"64210519E59C80E70FA7E9AB72243049FEB8DEECC146B9B1".as_slice(),
            )
        };
        (Self::parameter(p), Self::parameter(b))
    }

    fn add(left: EccPoint, right: EccPoint, p: &BigUint) -> EccPoint {
        match (left, right) {
            (EccPoint::Infinity, point) | (point, EccPoint::Infinity) => point,
            (EccPoint::Coordinate(x1, y1), EccPoint::Coordinate(x2, y2)) => {
                if x1 == x2 && (&y1 + &y2) % p == BigUint::from(0_u8) {
                    return EccPoint::Infinity;
                }
                let numerator = if x1 == x2 {
                    ((BigUint::from(3_u8) * &x1 * &x1) + p - BigUint::from(3_u8)) % p
                } else {
                    (&y2 + p - &y1) % p
                };
                let denominator = if x1 == x2 {
                    (BigUint::from(2_u8) * &y1) % p
                } else {
                    (&x2 + p - &x1) % p
                };
                if denominator == BigUint::from(0_u8) {
                    return EccPoint::Infinity;
                }
                let slope = numerator * denominator.modpow(&(p - BigUint::from(2_u8)), p) % p;
                let x3 = (&slope * &slope + p + p - &x1 - &x2) % p;
                let y3 = (&slope * (&x1 + p - &x3) + p - &y1) % p;
                EccPoint::Coordinate(x3, y3)
            }
        }
    }

    fn multiply(mut scalar: BigUint, mut point: EccPoint, p: &BigUint) -> EccPoint {
        let mut result = EccPoint::Infinity;
        while scalar != BigUint::from(0_u8) {
            if (&scalar & BigUint::from(1_u8)) != BigUint::from(0_u8) {
                result = Self::add(result, point.clone(), p);
            }
            point = Self::add(point.clone(), point, p);
            scalar >>= 1;
        }
        result
    }

    fn run(&mut self) -> Result<(), DeviceError> {
        let (p, b) = self.curve();
        let x = Self::integer(&self.x);
        let y = Self::integer(&self.y);
        let valid = x < p
            && y < p
            && (&y * &y) % &p == ((&x * &x * &x) + (&p - BigUint::from(3_u8)) * &x + b) % &p;
        self.config = (self.config & !(1 << 8)) | (u32::from(valid) << 8);
        let mode = (self.config >> 5) & 7;
        if matches!(mode, 0 | 3 | 4 | 7) {
            if !valid {
                return Err(DeviceError::new(
                    "ESP32-C6 ECC input point is not on the selected curve",
                ));
            }
            if let EccPoint::Coordinate(x, y) =
                Self::multiply(Self::integer(&self.k), EccPoint::Coordinate(x, y), &p)
            {
                Self::store(&mut self.x, &x);
                Self::store(&mut self.y, &y);
            }
        } else if !matches!(mode, 2 | 6) {
            return Err(DeviceError::new(format!(
                "unsupported ESP32-C6 ECC work mode {mode}"
            )));
        }
        self.config &= !1;
        self.interrupt = true;
        Ok(())
    }
}

impl Device for EspC6Ecc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 ECC requires aligned word access",
            ));
        }
        let value = match offset {
            0x0c => u32::from(self.interrupt),
            0x10 => u32::from(self.interrupt && self.interrupt_enable),
            0x14 => u32::from(self.interrupt_enable),
            0x18 => 0,
            0x1c => self.config,
            0xfc => self.date,
            0x100..=0x11c => self.k[(offset as usize - 0x100) / 4],
            0x120..=0x13c => self.x[(offset as usize - 0x120) / 4],
            0x140..=0x15c => self.y[(offset as usize - 0x140) / 4],
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved read {offset:#x}",
                    self.name
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
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 ECC requires aligned word access",
            ));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-C6 ECC rejects wide writes"))?;
        match offset {
            0x0c => self.interrupt &= value & 1 == 0,
            0x14 => self.interrupt_enable = value & 1 != 0,
            0x18 => self.interrupt &= value & 1 == 0,
            0x1c if value & 2 != 0 => {
                self.config = 1 << 31;
                self.interrupt = false;
                self.k.fill(0);
                self.x.fill(0);
                self.y.fill(0);
            }
            0x1c => {
                self.config = value & 0x8000_00fd;
                if value & 1 != 0 {
                    self.run()?;
                }
            }
            0xfc => self.date = value & 0x0fff_ffff,
            0x100..=0x11c => self.k[(offset as usize - 0x100) / 4] = value,
            0x120..=0x13c => self.x[(offset as usize - 0x120) / 4] = value,
            0x140..=0x15c => self.y[(offset as usize - 0x140) / 4] = value,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved write {offset:#x}",
                    self.name
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.config = 1 << 31;
        self.interrupt = false;
        self.interrupt_enable = false;
        self.k.fill(0);
        self.x.fill(0);
        self.y.fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn efuse_programming_is_one_way_persistent_and_interruptible() {
        let mut efuse = EspC6Efuse::new("efuse");
        efuse
            .write(0, AccessWidth::Word, 0x55aa, SimTime::ZERO)
            .unwrap();
        efuse
            .write(EFUSE_CONF, AccessWidth::Word, 0x5a5a, SimTime::ZERO)
            .unwrap();
        efuse
            .write(EFUSE_CMD, AccessWidth::Word, 3 << 2 | 2, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            efuse.read(0x7c, AccessWidth::Word, SimTime::ZERO),
            Ok(0x55aa)
        );
        assert_eq!(
            efuse.read(EFUSE_INT_RAW, AccessWidth::Word, SimTime::ZERO),
            Ok(2)
        );
        efuse.reset(ResetKind::Watchdog);
        assert_eq!(
            efuse.read(0x7c, AccessWidth::Word, SimTime::ZERO),
            Ok(0x55aa)
        );
    }

    #[test]
    fn ecc_p256_scalar_one_preserves_the_standard_base_point() {
        let mut ecc = EspC6Ecc::new("ecc");
        let gx = EspC6Ecc::parameter(
            b"6B17D1F2E12C4247F8BCE6E563A440F277037D812DEB33A0F4A13945D898C296",
        );
        let gy = EspC6Ecc::parameter(
            b"4FE342E2FE1A7F9B8EE7EB4A7C0F9E162BCE33576B315ECECBB6406837BF51F5",
        );
        EspC6Ecc::store(&mut ecc.k, &BigUint::from(1_u8));
        EspC6Ecc::store(&mut ecc.x, &gx);
        EspC6Ecc::store(&mut ecc.y, &gy);
        ecc.write(0x14, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        ecc.write(
            0x1c,
            AccessWidth::Word,
            (1 << 31) | (1 << 2) | 1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(EspC6Ecc::integer(&ecc.x), gx);
        assert_eq!(EspC6Ecc::integer(&ecc.y), gy);
        assert_eq!(ecc.read(0x10, AccessWidth::Word, SimTime::ZERO), Ok(1));
    }
}
