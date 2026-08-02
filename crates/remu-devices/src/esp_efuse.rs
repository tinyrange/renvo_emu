use super::*;

const EFUSE_PGM_DATA_BASE: u64 = 0x00;
const EFUSE_PGM_DATA_WORDS: usize = 8;
const EFUSE_RD_WR_DIS: u64 = 0x2c;
const EFUSE_RD_REPEAT_DATA_BASE: u64 = 0x30;
const EFUSE_RD_MAC_BASE: u64 = 0x44;
const EFUSE_RD_SYS_PART1_BASE: u64 = 0x5c;
const EFUSE_RD_USER_BASE: u64 = 0x7c;
const EFUSE_RD_KEY_BASE: u64 = 0x9c;
const EFUSE_RD_SYS_PART2_BASE: u64 = 0x15c;
const EFUSE_OTP_READONLY_END: u64 = EFUSE_RD_SYS_PART2_BASE + 0x1c;
const EFUSE_STATUS: u64 = 0x1d0;
const EFUSE_CMD: u64 = 0x1d4;
const EFUSE_INT_RAW: u64 = 0x1d8;
const EFUSE_INT_STATUS: u64 = 0x1dc;
const EFUSE_INT_ENABLE: u64 = 0x1e0;
const EFUSE_INT_CLEAR: u64 = 0x1e4;
const EFUSE_DATE: u64 = 0x1fc;
const EFUSE_READ_CMD: u32 = 1;
const EFUSE_PROGRAM_CMD: u32 = 1 << 1;

struct EspEfuseState {
    registers: Vec<u32>,
    program_data: [u32; EFUSE_PGM_DATA_WORDS],
    status: u32,
    interrupt_raw: u32,
    interrupt_enable: u32,
}

impl EspEfuseState {
    fn new() -> Self {
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            program_data: [0; EFUSE_PGM_DATA_WORDS],
            status: 0,
            interrupt_raw: 0,
            interrupt_enable: 0,
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.program_data = [0; EFUSE_PGM_DATA_WORDS];
        self.status = 0;
        self.interrupt_raw = 0;
        self.interrupt_enable = 0;
        // A stable locally-administered factory identity gives network and
        // SDK startup code a useful value without pretending to know a board's
        // physical eFuse contents.
        self.registers[(EFUSE_RD_MAC_BASE / 4) as usize..(EFUSE_RD_MAC_BASE / 4) as usize + 6]
            .copy_from_slice(&[0x3322_1100, 0x0000_5544, 0x0000_0000, 0, 0, 0]);
        self.registers[(EFUSE_DATE / 4) as usize] = 0x2025_0001;
    }

    fn set_interrupt(&mut self, bit: u32) {
        self.interrupt_raw |= bit;
    }

    fn status_interrupts(&self) -> u32 {
        self.interrupt_raw & self.interrupt_enable & 0x3
    }

    fn program_destination(block: u32) -> Option<(u64, usize)> {
        match block {
            0 => Some((EFUSE_RD_REPEAT_DATA_BASE, 5)),
            1 => Some((EFUSE_RD_MAC_BASE, 6)),
            2 => Some((EFUSE_RD_SYS_PART1_BASE, 8)),
            3 => Some((EFUSE_RD_USER_BASE, 8)),
            4..=9 => Some((EFUSE_RD_KEY_BASE + (block - 4) as u64 * 0x20, 8)),
            10 => Some((EFUSE_RD_SYS_PART2_BASE, 8)),
            _ => None,
        }
    }

    fn program(&mut self, block: u32) {
        if let Some((destination, words)) = Self::program_destination(block) {
            let start = (destination / 4) as usize;
            for (index, value) in self.program_data.iter().copied().enumerate().take(words) {
                self.registers[start + index] |= value;
            }
            self.set_interrupt(1 << 1);
        } else {
            self.status |= 1 << 18;
        }
    }

    fn key_is_read_disabled(&self, key: usize) -> bool {
        key < 7 && self.registers[(EFUSE_RD_REPEAT_DATA_BASE / 4) as usize] & (1 << key) != 0
    }

    fn read_register(&self, offset: u64) -> u32 {
        if (EFUSE_RD_KEY_BASE..EFUSE_RD_KEY_BASE + 6 * 0x20).contains(&offset) {
            let key = ((offset - EFUSE_RD_KEY_BASE) / 0x20) as usize;
            if self.key_is_read_disabled(key) {
                return 0;
            }
        }
        match offset {
            EFUSE_STATUS => self.status,
            EFUSE_INT_RAW => self.interrupt_raw,
            EFUSE_INT_STATUS => self.status_interrupts(),
            EFUSE_INT_ENABLE => self.interrupt_enable,
            _ => self.registers[(offset / 4) as usize],
        }
    }
}

/// Functional ESP32-S3 eFuse controller and one-time-programmable data view.
///
/// Reads, staging writes, block selection, OTP bitwise programming, read
/// redaction, command completion, and interrupt status are modelled. Voltage,
/// Reed-Solomon timing, secure boot policy, and physical fuse characteristics
/// are intentionally outside this deterministic functional slice.
pub struct EspEfuse {
    name: String,
    state: EspEfuseState,
}

impl EspEfuse {
    /// Creates an unprogrammed deterministic eFuse bank.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: EspEfuseState::new(),
        }
    }
}

impl Device for EspEfuse {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP eFuse requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("eFuse offset fits");
        if index >= self.state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} read at {offset:#x}",
                self.name
            )));
        }
        Ok(u64::from(self.state.read_register(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP eFuse requires aligned word access"));
        }
        let index = usize::try_from(offset / 4).expect("eFuse offset fits");
        if index >= self.state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = value as u32;
        match offset {
            EFUSE_PGM_DATA_BASE..=0x1c => {
                self.state.program_data[index] = value;
                self.state.registers[index] = value;
            }
            EFUSE_CMD => {
                self.state.registers[index] = value & 0x3f;
                if value & EFUSE_READ_CMD != 0 {
                    self.state.set_interrupt(1);
                }
                if value & EFUSE_PROGRAM_CMD != 0 {
                    self.state.program((value >> 2) & 0xf);
                }
            }
            EFUSE_INT_ENABLE => self.state.interrupt_enable = value & 0x3,
            EFUSE_INT_CLEAR => {
                self.state.interrupt_raw &= !(value & 0x3);
                self.state.registers[index] = 0;
            }
            EFUSE_STATUS | EFUSE_INT_RAW | EFUSE_INT_STATUS => {}
            EFUSE_RD_WR_DIS..=EFUSE_OTP_READONLY_END => {}
            _ => self.state.registers[index] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_factory_identity_and_programs_otp_bits_once() {
        let mut device = EspEfuse::new("efuse");
        assert_eq!(
            device.read(EFUSE_RD_MAC_BASE, AccessWidth::Word, SimTime::ZERO),
            Ok(0x3322_1100)
        );
        device
            .write(
                EFUSE_PGM_DATA_BASE,
                AccessWidth::Word,
                1 << 7,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                EFUSE_CMD,
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_CMD),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(EFUSE_RD_REPEAT_DATA_BASE, AccessWidth::Word, SimTime::ZERO),
            Ok(1 << 7)
        );
        device
            .write(EFUSE_INT_ENABLE, AccessWidth::Word, 1 << 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(EFUSE_INT_STATUS, AccessWidth::Word, SimTime::ZERO),
            Ok(1 << 1)
        );
        device
            .write(EFUSE_INT_CLEAR, AccessWidth::Word, 1 << 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(EFUSE_INT_STATUS, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
    }

    #[test]
    fn disabled_key_blocks_read_as_zero() {
        let mut device = EspEfuse::new("efuse");
        device
            .write(EFUSE_PGM_DATA_BASE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(
                EFUSE_CMD,
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_CMD | (4 << 2)),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(EFUSE_PGM_DATA_BASE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(
                EFUSE_CMD,
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_CMD),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(EFUSE_RD_KEY_BASE, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
    }
}
