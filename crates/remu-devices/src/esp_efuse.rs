use super::*;

const EFUSE_PGM_DATA_WORDS: usize = 8;
const EFUSE_READ_CMD: u32 = 1 << 0;
const EFUSE_PROGRAM_CMD: u32 = 1 << 1;
const EFUSE_INTERRUPT_MASK: u32 = 0x3;
const EFUSE_READ_OPCODE: u32 = 0x5aa5;
const EFUSE_PROGRAM_OPCODE: u32 = 0x5a5a;

/// Native ESP32-S3 eFuse register identifiers from Espressif's
/// `efuse_reg.h` map.
///
/// Register IDs are deliberately named rather than represented by ad-hoc
/// offsets.  This makes device and machine tests self-documenting and lets
/// reserved holes in the native page fail explicitly.
macro_rules! efuse_registers {
    ($($name:ident = $offset:expr),* $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
        #[repr(u16)]
        #[allow(missing_docs)]
        pub enum Esp32S3EfuseRegister {
            $($name = $offset,)*
        }

        impl Esp32S3EfuseRegister {
            /// Returns the native byte offset in the eFuse peripheral page.
            pub const fn offset(self) -> u64 {
                self as u64
            }

            /// Resolves a native byte offset. Reserved holes return `None`.
            pub const fn from_offset(offset: u64) -> Option<Self> {
                match offset {
                    $($offset => Some(Self::$name),)*
                    _ => None,
                }
            }
        }
    };
}

efuse_registers! {
    PgmData0 = 0x000,
    PgmData1 = 0x004,
    PgmData2 = 0x008,
    PgmData3 = 0x00c,
    PgmData4 = 0x010,
    PgmData5 = 0x014,
    PgmData6 = 0x018,
    PgmData7 = 0x01c,
    PgmCheckValue0 = 0x020,
    PgmCheckValue1 = 0x024,
    PgmCheckValue2 = 0x028,
    RdWrDis = 0x02c,
    RdRepeatData0 = 0x030,
    RdRepeatData1 = 0x034,
    RdRepeatData2 = 0x038,
    RdRepeatData3 = 0x03c,
    RdRepeatData4 = 0x040,
    RdMac0 = 0x044,
    RdMac1 = 0x048,
    RdMac2 = 0x04c,
    RdMac3 = 0x050,
    RdMac4 = 0x054,
    RdMac5 = 0x058,
    RdSysPart1Data0 = 0x05c,
    RdSysPart1Data1 = 0x060,
    RdSysPart1Data2 = 0x064,
    RdSysPart1Data3 = 0x068,
    RdSysPart1Data4 = 0x06c,
    RdSysPart1Data5 = 0x070,
    RdSysPart1Data6 = 0x074,
    RdSysPart1Data7 = 0x078,
    RdUsrData0 = 0x07c,
    RdUsrData1 = 0x080,
    RdUsrData2 = 0x084,
    RdUsrData3 = 0x088,
    RdUsrData4 = 0x08c,
    RdUsrData5 = 0x090,
    RdUsrData6 = 0x094,
    RdUsrData7 = 0x098,
    RdKey0Data0 = 0x09c,
    RdKey0Data1 = 0x0a0,
    RdKey0Data2 = 0x0a4,
    RdKey0Data3 = 0x0a8,
    RdKey0Data4 = 0x0ac,
    RdKey0Data5 = 0x0b0,
    RdKey0Data6 = 0x0b4,
    RdKey0Data7 = 0x0b8,
    RdKey1Data0 = 0x0bc,
    RdKey1Data1 = 0x0c0,
    RdKey1Data2 = 0x0c4,
    RdKey1Data3 = 0x0c8,
    RdKey1Data4 = 0x0cc,
    RdKey1Data5 = 0x0d0,
    RdKey1Data6 = 0x0d4,
    RdKey1Data7 = 0x0d8,
    RdKey2Data0 = 0x0dc,
    RdKey2Data1 = 0x0e0,
    RdKey2Data2 = 0x0e4,
    RdKey2Data3 = 0x0e8,
    RdKey2Data4 = 0x0ec,
    RdKey2Data5 = 0x0f0,
    RdKey2Data6 = 0x0f4,
    RdKey2Data7 = 0x0f8,
    RdKey3Data0 = 0x0fc,
    RdKey3Data1 = 0x100,
    RdKey3Data2 = 0x104,
    RdKey3Data3 = 0x108,
    RdKey3Data4 = 0x10c,
    RdKey3Data5 = 0x110,
    RdKey3Data6 = 0x114,
    RdKey3Data7 = 0x118,
    RdKey4Data0 = 0x11c,
    RdKey4Data1 = 0x120,
    RdKey4Data2 = 0x124,
    RdKey4Data3 = 0x128,
    RdKey4Data4 = 0x12c,
    RdKey4Data5 = 0x130,
    RdKey4Data6 = 0x134,
    RdKey4Data7 = 0x138,
    RdKey5Data0 = 0x13c,
    RdKey5Data1 = 0x140,
    RdKey5Data2 = 0x144,
    RdKey5Data3 = 0x148,
    RdKey5Data4 = 0x14c,
    RdKey5Data5 = 0x150,
    RdKey5Data6 = 0x154,
    RdKey5Data7 = 0x158,
    RdSysPart2Data0 = 0x15c,
    RdSysPart2Data1 = 0x160,
    RdSysPart2Data2 = 0x164,
    RdSysPart2Data3 = 0x168,
    RdSysPart2Data4 = 0x16c,
    RdSysPart2Data5 = 0x170,
    RdSysPart2Data6 = 0x174,
    RdSysPart2Data7 = 0x178,
    RdRepeatErr0 = 0x17c,
    RdRepeatErr1 = 0x180,
    RdRepeatErr2 = 0x184,
    RdRepeatErr3 = 0x188,
    RdRepeatErr4 = 0x190,
    RdRsErr0 = 0x1c0,
    RdRsErr1 = 0x1c4,
    Clk = 0x1c8,
    Conf = 0x1cc,
    Status = 0x1d0,
    Cmd = 0x1d4,
    IntRaw = 0x1d8,
    IntSt = 0x1dc,
    IntEna = 0x1e0,
    IntClr = 0x1e4,
    DacConf = 0x1e8,
    RdTimConf = 0x1ec,
    WrTimConf1 = 0x1f4,
    WrTimConf2 = 0x1f8,
    Date = 0x1fc,
}

impl Esp32S3EfuseRegister {
    /// Bits returned by a native read of this register.
    pub const fn read_mask(self) -> u32 {
        match self {
            Self::RdRepeatData4 | Self::RdRepeatErr4 => 0x00ff_ffff,
            Self::RdRsErr1 => 0x0000_00ff,
            Self::Clk => 0x0001_0007,
            Self::Conf => 0x0000_ffff,
            Self::Status => 0x0003_ffff,
            // BLK_NUM remains readable while READ_CMD and PGM_CMD are
            // command strobes that self-clear after the operation.
            Self::Cmd => 0x0000_003f,
            Self::IntRaw | Self::IntSt | Self::IntEna => 0x3,
            Self::IntClr => 0,
            Self::DacConf => 0x0003_ffff,
            Self::RdTimConf => 0xff00_0000,
            Self::WrTimConf1 => 0x00ff_ff00,
            Self::WrTimConf2 => 0x0000_ffff,
            Self::Date => 0x0fff_ffff,
            _ => u32::MAX,
        }
    }

    /// Bits accepted by a native write of this register.
    pub const fn write_mask(self) -> u32 {
        match self {
            Self::PgmData0
            | Self::PgmData1
            | Self::PgmData2
            | Self::PgmData3
            | Self::PgmData4
            | Self::PgmData5
            | Self::PgmData6
            | Self::PgmData7
            | Self::PgmCheckValue0
            | Self::PgmCheckValue1
            | Self::PgmCheckValue2 => u32::MAX,
            Self::Clk => 0x0001_0007,
            Self::Conf => 0x0000_ffff,
            Self::Cmd => 0x0000_003f,
            Self::IntRaw | Self::IntEna | Self::IntClr => 0x3,
            Self::DacConf => 0x0003_ffff,
            Self::RdTimConf => 0xff00_0000,
            Self::WrTimConf1 => 0x00ff_ff00,
            Self::WrTimConf2 => 0x0000_ffff,
            Self::Date => 0x0fff_ffff,
            _ => 0,
        }
    }

    fn pgm_data_index(self) -> Option<usize> {
        Some(match self {
            Self::PgmData0 => 0,
            Self::PgmData1 => 1,
            Self::PgmData2 => 2,
            Self::PgmData3 => 3,
            Self::PgmData4 => 4,
            Self::PgmData5 => 5,
            Self::PgmData6 => 6,
            Self::PgmData7 => 7,
            _ => return None,
        })
    }
}

const KEY_BASES: [Esp32S3EfuseRegister; 6] = [
    Esp32S3EfuseRegister::RdKey0Data0,
    Esp32S3EfuseRegister::RdKey1Data0,
    Esp32S3EfuseRegister::RdKey2Data0,
    Esp32S3EfuseRegister::RdKey3Data0,
    Esp32S3EfuseRegister::RdKey4Data0,
    Esp32S3EfuseRegister::RdKey5Data0,
];

struct EspEfuseState {
    registers: BTreeMap<Esp32S3EfuseRegister, u32>,
    program_data: [u32; EFUSE_PGM_DATA_WORDS],
    status: u32,
    interrupt_raw: u32,
    interrupt_enable: u32,
    command_block: u32,
}

impl EspEfuseState {
    fn new() -> Self {
        let mut state = Self {
            registers: BTreeMap::new(),
            program_data: [0; EFUSE_PGM_DATA_WORDS],
            status: 0,
            interrupt_raw: 0,
            interrupt_enable: 0,
            command_block: 0,
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.clear();
        self.program_data = [0; EFUSE_PGM_DATA_WORDS];
        self.status = 0;
        self.interrupt_raw = 0;
        self.interrupt_enable = 0;
        self.command_block = 0;

        // The functional model uses a stable locally-administered factory
        // identity.  It is deterministic and useful to network/SDK startup
        // code without claiming to know a physical board's fuse contents.
        for (index, value) in [0x3322_1100, 0x0000_5544, 0x0000_0000, 0, 0, 0]
            .into_iter()
            .enumerate()
        {
            self.registers.insert(
                Esp32S3EfuseRegister::from_offset(0x44 + index as u64 * 4)
                    .expect("factory MAC register exists"),
                value,
            );
        }

        // Defaults are taken from the native register definitions.
        self.registers.insert(Esp32S3EfuseRegister::Clk, 1 << 1);
        self.registers
            .insert(Esp32S3EfuseRegister::DacConf, 28 | (255 << 9));
        self.registers
            .insert(Esp32S3EfuseRegister::RdTimConf, 18 << 24);
        self.registers
            .insert(Esp32S3EfuseRegister::WrTimConf1, 10368 << 8);
        self.registers.insert(Esp32S3EfuseRegister::WrTimConf2, 400);
        self.registers
            .insert(Esp32S3EfuseRegister::Date, 0x0210_1290);
    }

    fn set_interrupt(&mut self, bit: u32) {
        self.interrupt_raw |= bit & EFUSE_INTERRUPT_MASK;
    }

    fn status_interrupts(&self) -> u32 {
        self.interrupt_raw & self.interrupt_enable & EFUSE_INTERRUPT_MASK
    }

    fn program_destination(block: u32) -> Option<(Esp32S3EfuseRegister, usize)> {
        match block {
            // BLOCK0 has a special layout: PGM_DATA0 is WR_DIS and
            // PGM_DATA1..5 are the repeat-data words.  BLOCK1 is factory
            // programmed and cannot be written by user firmware.
            2 => Some((Esp32S3EfuseRegister::RdSysPart1Data0, 8)),
            3 => Some((Esp32S3EfuseRegister::RdUsrData0, 8)),
            4..=9 => Some((KEY_BASES[(block - 4) as usize], 8)),
            10 => Some((Esp32S3EfuseRegister::RdSysPart2Data0, 8)),
            _ => None,
        }
    }

    fn clear_program_staging(&mut self) {
        self.program_data = [0; EFUSE_PGM_DATA_WORDS];
        for index in 0..EFUSE_PGM_DATA_WORDS {
            let register = Esp32S3EfuseRegister::from_offset(index as u64 * 4)
                .expect("eFuse programming register exists");
            self.registers.insert(register, 0);
        }
        for index in 0..3 {
            let register = Esp32S3EfuseRegister::from_offset(0x20 + index as u64 * 4)
                .expect("eFuse check-value register exists");
            self.registers.insert(register, 0);
        }
    }

    fn program_block0(&mut self) {
        // EFUSE_PGM_DATA0 stores EFUSE_WR_DIS.  The repeat-data words start
        // at PGM_DATA1, as specified by the ESP32-S3 TRM.
        let write_disable =
            self.register_value(Esp32S3EfuseRegister::RdWrDis) | self.program_data[0];
        self.registers
            .insert(Esp32S3EfuseRegister::RdWrDis, write_disable);
        for index in 0..5 {
            let destination = Esp32S3EfuseRegister::from_offset(0x30 + index as u64 * 4)
                .expect("eFuse repeat-data register exists");
            let current = self.register_value(destination);
            self.registers
                .insert(destination, current | self.program_data[index + 1]);
        }
        self.set_interrupt(1 << 1);
    }

    fn program(&mut self, block: u32) {
        if block == 0 {
            self.program_block0();
        } else if let Some((destination, words)) = Self::program_destination(block) {
            // BLOCK2..10 are one-time programmable.  Keep the functional
            // model one-way and reject a second command once any bit in the
            // destination block is already set.
            let already_programmed = (0..words).any(|index| {
                let register =
                    Esp32S3EfuseRegister::from_offset(destination.offset() + index as u64 * 4)
                        .expect("eFuse programming destination is contiguous");
                self.register_value(register) != 0
            });
            if already_programmed {
                return;
            }
            for (index, value) in self.program_data.iter().copied().enumerate().take(words) {
                let register =
                    Esp32S3EfuseRegister::from_offset(destination.offset() + index as u64 * 4)
                        .expect("eFuse programming destination is contiguous");
                self.registers.insert(register, value);
            }
            self.set_interrupt(1 << 1);
        }
    }

    fn register_value(&self, register: Esp32S3EfuseRegister) -> u32 {
        self.registers.get(&register).copied().unwrap_or_default()
    }

    fn key_is_read_disabled(&self, key: usize) -> bool {
        key < 7 && self.register_value(Esp32S3EfuseRegister::RdRepeatData0) & (1 << key) != 0
    }

    fn key_index(register: Esp32S3EfuseRegister) -> Option<usize> {
        let offset = register.offset();
        if (0x9c..=0x158).contains(&offset) {
            Some(((offset - 0x9c) / 0x20) as usize)
        } else {
            None
        }
    }

    fn read_register(&self, register: Esp32S3EfuseRegister) -> u32 {
        if let Some(key) = Self::key_index(register) {
            if self.key_is_read_disabled(key) {
                return 0;
            }
        }
        (match register {
            Esp32S3EfuseRegister::Status => self.status,
            Esp32S3EfuseRegister::Cmd => self.command_block << 2,
            Esp32S3EfuseRegister::IntRaw => self.interrupt_raw,
            Esp32S3EfuseRegister::IntSt => self.status_interrupts(),
            Esp32S3EfuseRegister::IntEna => self.interrupt_enable,
            Esp32S3EfuseRegister::IntClr => 0,
            _ => self.register_value(register),
        }) & register.read_mask()
    }
}

/// Functional ESP32-S3 eFuse controller and one-time-programmable data view.
///
/// The native staging, read-data, command, status, error, timing and
/// interrupt windows are represented. OTP programming is one-way (bitwise OR),
/// key read-disable redaction is honoured, and command completion is exposed
/// through the native raw/status interrupt bits. Voltage, Reed-Solomon timing,
/// secure-boot policy, and physical fuse characteristics remain outside this
/// deterministic functional slice.
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
        let register = Esp32S3EfuseRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 eFuse register offset {offset:#x}"
            ))
        })?;
        Ok(u64::from(self.state.read_register(register)))
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
        let register = Esp32S3EfuseRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 eFuse register offset {offset:#x}"
            ))
        })?;
        let value = u32::try_from(value).map_err(|_| {
            DeviceError::new(format!(
                "ESP32-S3 eFuse word write exceeds 32 bits: {value:#x}"
            ))
        })?;
        let write_mask = register.write_mask();
        if write_mask == 0 {
            return Err(DeviceError::new(format!(
                "ESP32-S3 eFuse register {register:?} is read-only"
            )));
        }

        if let Some(index) = register.pgm_data_index() {
            let value = value & write_mask;
            self.state.program_data[index] = value;
            self.state.registers.insert(register, value);
            return Ok(());
        }

        match register {
            Esp32S3EfuseRegister::Cmd => {
                let value = value & write_mask;
                self.state.command_block = (value >> 2) & 0xf;
                if value & EFUSE_READ_CMD != 0 {
                    if self.state.register_value(Esp32S3EfuseRegister::Conf) == EFUSE_READ_OPCODE {
                        self.state.set_interrupt(1 << 0);
                    }
                }
                if value & EFUSE_PROGRAM_CMD != 0 {
                    if self.state.register_value(Esp32S3EfuseRegister::Conf) == EFUSE_PROGRAM_OPCODE
                    {
                        self.state.program(self.state.command_block);
                        // The TRM requires staging registers to be cleared
                        // after every programming attempt to avoid leaking
                        // sensitive key material.
                        self.state.clear_program_staging();
                    }
                }
            }
            Esp32S3EfuseRegister::IntRaw => {
                // The native raw interrupt bits are write-one-to-clear.
                self.state.interrupt_raw &= !(value & write_mask);
            }
            Esp32S3EfuseRegister::IntEna => {
                self.state.interrupt_enable = value & write_mask;
            }
            Esp32S3EfuseRegister::IntClr => {
                self.state.interrupt_raw &= !(value & write_mask);
            }
            _ => {
                let old = self.state.register_value(register);
                self.state
                    .registers
                    .insert(register, (old & !write_mask) | (value & write_mask));
            }
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
            device.read(
                Esp32S3EfuseRegister::RdMac0.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0x3322_1100)
        );
        device
            .write(
                Esp32S3EfuseRegister::PgmData1.offset(),
                AccessWidth::Word,
                1 << 7,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3EfuseRegister::Conf.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_OPCODE),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3EfuseRegister::Cmd.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_CMD),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(
                Esp32S3EfuseRegister::RdRepeatData0.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1 << 7)
        );
        assert_eq!(
            device.read(
                Esp32S3EfuseRegister::PgmData1.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
        device
            .write(
                Esp32S3EfuseRegister::IntEna.offset(),
                AccessWidth::Word,
                1 << 1,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(
                Esp32S3EfuseRegister::IntSt.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1 << 1)
        );
        device
            .write(
                Esp32S3EfuseRegister::IntClr.offset(),
                AccessWidth::Word,
                1 << 1,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(
                Esp32S3EfuseRegister::IntSt.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
    }

    #[test]
    fn disabled_key_blocks_read_as_zero() {
        let mut device = EspEfuse::new("efuse");
        device
            .write(
                Esp32S3EfuseRegister::PgmData0.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3EfuseRegister::Conf.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_OPCODE),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3EfuseRegister::Cmd.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_CMD | (4 << 2)),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3EfuseRegister::PgmData1.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3EfuseRegister::Cmd.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_CMD),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(
                Esp32S3EfuseRegister::RdKey0Data0.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
    }

    #[test]
    fn command_strobes_require_native_opcodes_and_read_completion() {
        let mut device = EspEfuse::new("efuse");
        device
            .write(
                Esp32S3EfuseRegister::PgmData1.offset(),
                AccessWidth::Word,
                0x55,
                SimTime::ZERO,
            )
            .unwrap();
        // A command with the wrong opcode must not alter the fuse bank.
        device
            .write(
                Esp32S3EfuseRegister::Conf.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_READ_OPCODE),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3EfuseRegister::Cmd.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_CMD),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(
                Esp32S3EfuseRegister::RdRepeatData0.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );

        device
            .write(
                Esp32S3EfuseRegister::Conf.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_OPCODE),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3EfuseRegister::Cmd.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_PROGRAM_CMD),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(
                Esp32S3EfuseRegister::RdRepeatData0.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0x55)
        );

        device
            .write(
                Esp32S3EfuseRegister::Conf.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_READ_OPCODE),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3EfuseRegister::Cmd.offset(),
                AccessWidth::Word,
                u64::from(EFUSE_READ_CMD),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device.read(
                Esp32S3EfuseRegister::IntRaw.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(3)
        );
    }

    #[test]
    fn enum_masks_defaults_and_reserved_holes_match_native_map() {
        assert_eq!(
            Esp32S3EfuseRegister::from_offset(0x1c8),
            Some(Esp32S3EfuseRegister::Clk)
        );
        assert_eq!(Esp32S3EfuseRegister::from_offset(0x194), None);
        assert_eq!(Esp32S3EfuseRegister::RdRepeatErr4.read_mask(), 0x00ff_ffff);
        assert_eq!(Esp32S3EfuseRegister::Cmd.read_mask(), 0x3f);
        assert_eq!(Esp32S3EfuseRegister::Cmd.write_mask(), 0x3f);

        let mut device = EspEfuse::new("efuse");
        assert_eq!(
            device.read(
                Esp32S3EfuseRegister::Date.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0x0210_1290)
        );
        assert!(
            device
                .read(0x194, AccessWidth::Word, SimTime::ZERO)
                .is_err()
        );
        assert!(
            device
                .write(
                    Esp32S3EfuseRegister::RdMac0.offset(),
                    AccessWidth::Word,
                    1,
                    SimTime::ZERO,
                )
                .is_err()
        );
        assert!(
            device
                .write(
                    Esp32S3EfuseRegister::PgmData0.offset(),
                    AccessWidth::Word,
                    u64::from(u32::MAX) + 1,
                    SimTime::ZERO,
                )
                .is_err()
        );
    }
}
