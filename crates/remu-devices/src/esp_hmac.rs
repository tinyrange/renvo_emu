use super::*;
use sha2::{Digest, Sha256};

const HMAC_KEY_PURPOSE_DOWN_ALL: u32 = 5;
const HMAC_KEY_PURPOSE_UP: u32 = 8;
const HMAC_BLOCK_BYTES: usize = 64;
const HMAC_DIGEST_BYTES: usize = 32;

/// Native ESP32-S3 HMAC register identifiers from Espressif's
/// `hwcrypto_reg.h` map.
///
/// The command registers and data windows are intentionally represented as
/// distinct IDs. This keeps firmware-facing register access typed and makes
/// reserved holes in the native map fail explicitly instead of becoming
/// accidental backing storage.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u16)]
#[allow(missing_docs)]
pub enum Esp32S3HmacRegister {
    SetStart = 0x40,
    SetParaPurpose = 0x44,
    SetParaKey = 0x48,
    SetParaFinish = 0x4c,
    SetMessageOne = 0x50,
    SetMessageIng = 0x54,
    SetMessageEnd = 0x58,
    SetResultFinish = 0x5c,
    SetInvalidateJtag = 0x60,
    SetInvalidateDs = 0x64,
    QueryError = 0x68,
    QueryBusy = 0x6c,
    Wdata0 = 0x80,
    Wdata1 = 0x84,
    Wdata2 = 0x88,
    Wdata3 = 0x8c,
    Wdata4 = 0x90,
    Wdata5 = 0x94,
    Wdata6 = 0x98,
    Wdata7 = 0x9c,
    Wdata8 = 0xa0,
    Wdata9 = 0xa4,
    Wdata10 = 0xa8,
    Wdata11 = 0xac,
    Wdata12 = 0xb0,
    Wdata13 = 0xb4,
    Wdata14 = 0xb8,
    Wdata15 = 0xbc,
    Rdata0 = 0xc0,
    Rdata1 = 0xc4,
    Rdata2 = 0xc8,
    Rdata3 = 0xcc,
    Rdata4 = 0xd0,
    Rdata5 = 0xd4,
    Rdata6 = 0xd8,
    Rdata7 = 0xdc,
    SetMessagePad = 0xf0,
    OneBlock = 0xf4,
    SoftJtagCtrl = 0xf8,
    WrJtag = 0xfc,
}

impl Esp32S3HmacRegister {
    /// Returns the native byte offset in the HMAC peripheral page.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    /// Resolves a native byte offset. Reserved holes return `None`.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x40 => Self::SetStart,
            0x44 => Self::SetParaPurpose,
            0x48 => Self::SetParaKey,
            0x4c => Self::SetParaFinish,
            0x50 => Self::SetMessageOne,
            0x54 => Self::SetMessageIng,
            0x58 => Self::SetMessageEnd,
            0x5c => Self::SetResultFinish,
            0x60 => Self::SetInvalidateJtag,
            0x64 => Self::SetInvalidateDs,
            0x68 => Self::QueryError,
            0x6c => Self::QueryBusy,
            0x80 => Self::Wdata0,
            0x84 => Self::Wdata1,
            0x88 => Self::Wdata2,
            0x8c => Self::Wdata3,
            0x90 => Self::Wdata4,
            0x94 => Self::Wdata5,
            0x98 => Self::Wdata6,
            0x9c => Self::Wdata7,
            0xa0 => Self::Wdata8,
            0xa4 => Self::Wdata9,
            0xa8 => Self::Wdata10,
            0xac => Self::Wdata11,
            0xb0 => Self::Wdata12,
            0xb4 => Self::Wdata13,
            0xb8 => Self::Wdata14,
            0xbc => Self::Wdata15,
            0xc0 => Self::Rdata0,
            0xc4 => Self::Rdata1,
            0xc8 => Self::Rdata2,
            0xcc => Self::Rdata3,
            0xd0 => Self::Rdata4,
            0xd4 => Self::Rdata5,
            0xd8 => Self::Rdata6,
            0xdc => Self::Rdata7,
            0xf0 => Self::SetMessagePad,
            0xf4 => Self::OneBlock,
            0xf8 => Self::SoftJtagCtrl,
            0xfc => Self::WrJtag,
            _ => return None,
        })
    }

    fn wdata_index(self) -> Option<usize> {
        Some(match self {
            Self::Wdata0 => 0,
            Self::Wdata1 => 1,
            Self::Wdata2 => 2,
            Self::Wdata3 => 3,
            Self::Wdata4 => 4,
            Self::Wdata5 => 5,
            Self::Wdata6 => 6,
            Self::Wdata7 => 7,
            Self::Wdata8 => 8,
            Self::Wdata9 => 9,
            Self::Wdata10 => 10,
            Self::Wdata11 => 11,
            Self::Wdata12 => 12,
            Self::Wdata13 => 13,
            Self::Wdata14 => 14,
            Self::Wdata15 => 15,
            _ => return None,
        })
    }

    fn rdata_index(self) -> Option<usize> {
        Some(match self {
            Self::Rdata0 => 0,
            Self::Rdata1 => 1,
            Self::Rdata2 => 2,
            Self::Rdata3 => 3,
            Self::Rdata4 => 4,
            Self::Rdata5 => 5,
            Self::Rdata6 => 6,
            Self::Rdata7 => 7,
            _ => return None,
        })
    }

    /// Bits returned by a native read of this register.
    pub const fn read_mask(self) -> u32 {
        match self {
            Self::QueryError => 0x3,
            Self::QueryBusy => 1,
            Self::Rdata0
            | Self::Rdata1
            | Self::Rdata2
            | Self::Rdata3
            | Self::Rdata4
            | Self::Rdata5
            | Self::Rdata6
            | Self::Rdata7 => u32::MAX,
            _ => 0,
        }
    }

    /// Bits accepted by a native write of this register.
    pub const fn write_mask(self) -> u32 {
        match self {
            Self::SetParaPurpose => 0xf,
            Self::SetParaKey => 0x7,
            Self::SetResultFinish => 0x3,
            Self::SoftJtagCtrl => 1,
            Self::QueryError | Self::QueryBusy => 0,
            Self::Rdata0
            | Self::Rdata1
            | Self::Rdata2
            | Self::Rdata3
            | Self::Rdata4
            | Self::Rdata5
            | Self::Rdata6
            | Self::Rdata7 => 0,
            Self::WrJtag
            | Self::Wdata0
            | Self::Wdata1
            | Self::Wdata2
            | Self::Wdata3
            | Self::Wdata4
            | Self::Wdata5
            | Self::Wdata6
            | Self::Wdata7
            | Self::Wdata8
            | Self::Wdata9
            | Self::Wdata10
            | Self::Wdata11
            | Self::Wdata12
            | Self::Wdata13
            | Self::Wdata14
            | Self::Wdata15 => u32::MAX,
            _ => 1,
        }
    }
}

struct EspHmacState {
    registers: BTreeMap<Esp32S3HmacRegister, u32>,
    wdata: [u32; HMAC_BLOCK_BYTES / 4],
    rdata: [u32; HMAC_DIGEST_BYTES / 4],
    message: Vec<u8>,
    result: [u8; HMAC_DIGEST_BYTES],
    key_id: u32,
    purpose: u32,
    started: bool,
    config_finished: bool,
    busy: bool,
    error: u32,
}

impl EspHmacState {
    fn new() -> Self {
        let mut state = Self {
            registers: BTreeMap::new(),
            wdata: [0; HMAC_BLOCK_BYTES / 4],
            rdata: [0; HMAC_DIGEST_BYTES / 4],
            message: Vec::new(),
            result: [0; HMAC_DIGEST_BYTES],
            key_id: 0,
            purpose: 0,
            started: false,
            config_finished: false,
            busy: false,
            error: 0,
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.clear();
        self.wdata.fill(0);
        self.rdata.fill(0);
        self.message.clear();
        self.result = [0; HMAC_DIGEST_BYTES];
        self.key_id = 0;
        self.purpose = 0;
        self.started = false;
        self.config_finished = false;
        self.busy = false;
        self.error = 0;
    }

    fn block(&self) -> [u8; HMAC_BLOCK_BYTES] {
        let mut block = [0_u8; HMAC_BLOCK_BYTES];
        for (index, word) in self.wdata.iter().enumerate() {
            block[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        block
    }

    fn synthetic_key(key_id: u32) -> [u8; HMAC_DIGEST_BYTES] {
        let mut seed = b"renvo-esp32s3-hmac-efuse-key-v1".to_vec();
        seed.extend_from_slice(&key_id.to_le_bytes());
        Sha256::digest(seed).into()
    }

    fn digest(&self, message: &[u8]) -> [u8; HMAC_DIGEST_BYTES] {
        let mut key = [0_u8; HMAC_BLOCK_BYTES];
        key[..HMAC_DIGEST_BYTES].copy_from_slice(&Self::synthetic_key(self.key_id));
        let mut inner_pad = [0_u8; HMAC_BLOCK_BYTES];
        let mut outer_pad = [0_u8; HMAC_BLOCK_BYTES];
        for index in 0..HMAC_BLOCK_BYTES {
            inner_pad[index] = key[index] ^ 0x36;
            outer_pad[index] = key[index] ^ 0x5c;
        }
        let mut inner = Sha256::new();
        inner.update(inner_pad);
        inner.update(message);
        let inner_digest = inner.finalize();
        let mut outer = Sha256::new();
        outer.update(outer_pad);
        outer.update(inner_digest);
        outer.finalize().into()
    }

    fn publish_result(&mut self) {
        self.result = self.digest(&self.message);
        for (index, chunk) in self.result.chunks_exact(4).enumerate() {
            self.rdata[index] =
                u32::from_le_bytes(chunk.try_into().expect("digest word is four bytes"));
        }
    }

    fn clear_result(&mut self) {
        self.message.clear();
        self.result = [0; HMAC_DIGEST_BYTES];
        self.rdata.fill(0);
    }

    fn command_error(&mut self, error: u32) {
        if self.error == 0 {
            self.error = error;
        }
    }

    fn append_block(&mut self) {
        self.message.extend_from_slice(&self.block());
    }

    fn finish_configuration(&mut self) {
        self.config_finished = self.started
            && (HMAC_KEY_PURPOSE_DOWN_ALL..=HMAC_KEY_PURPOSE_UP).contains(&self.purpose);
        if !self.started {
            self.command_error(2);
        } else if !self.config_finished {
            self.command_error(1);
        }
    }

    fn execute_message_command(&mut self, register: Esp32S3HmacRegister) {
        if !self.config_finished {
            self.command_error(3);
            return;
        }
        self.busy = true;
        match register {
            Esp32S3HmacRegister::SetMessageIng => self.append_block(),
            Esp32S3HmacRegister::SetMessageOne
            | Esp32S3HmacRegister::SetMessageEnd
            | Esp32S3HmacRegister::SetMessagePad
            | Esp32S3HmacRegister::OneBlock => {
                self.append_block();
                self.publish_result();
            }
            _ => {}
        }
        self.busy = false;
    }

    fn read_word(&self, register: Esp32S3HmacRegister) -> u32 {
        if let Some(index) = register.rdata_index() {
            return self.rdata[index];
        }
        match register {
            Esp32S3HmacRegister::QueryError => self.error,
            Esp32S3HmacRegister::QueryBusy => u32::from(self.busy),
            _ => self.registers.get(&register).copied().unwrap_or_default() & register.read_mask(),
        }
    }
}

/// Functional ESP32-S3 HMAC-SHA256 accelerator.
///
/// The native command and data-window layout is retained. Since an emulator
/// has no physical eFuse key, each selected key slot derives a stable
/// synthetic 256-bit key; this keeps compiler and firmware tests deterministic
/// while explicitly avoiding a claim of secure-key or eFuse fidelity.
pub struct EspHmac {
    name: String,
    state: EspHmacState,
}

impl EspHmac {
    /// Creates an idle HMAC register block.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: EspHmacState::new(),
        }
    }
}

impl Device for EspHmac {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP HMAC requires aligned word access"));
        }
        let register = Esp32S3HmacRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 HMAC register offset {offset:#x}"
            ))
        })?;
        Ok(u64::from(self.state.read_word(register)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP HMAC requires aligned word access"));
        }
        let register = Esp32S3HmacRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 HMAC register offset {offset:#x}"
            ))
        })?;
        let value = u32::try_from(value).map_err(|_| {
            DeviceError::new(format!(
                "ESP32-S3 HMAC word write exceeds 32 bits: {value:#x}"
            ))
        })?;
        if register.rdata_index().is_some() {
            return Err(DeviceError::new(
                "ESP32-S3 HMAC result registers are read-only",
            ));
        }
        if register == Esp32S3HmacRegister::QueryError || register == Esp32S3HmacRegister::QueryBusy
        {
            return Err(DeviceError::new(
                "ESP32-S3 HMAC query registers are read-only",
            ));
        }
        if let Some(index) = register.wdata_index() {
            self.state.wdata[index] = value & register.write_mask();
            return Ok(());
        }

        let command = value & register.write_mask();
        match register {
            Esp32S3HmacRegister::SetStart => {
                if command != 0 {
                    self.state.started = true;
                    self.state.config_finished = false;
                    self.state.clear_result();
                    self.state.error = 0;
                }
            }
            Esp32S3HmacRegister::SetParaPurpose => self.state.purpose = command,
            Esp32S3HmacRegister::SetParaKey => self.state.key_id = command,
            Esp32S3HmacRegister::SetParaFinish => {
                if command != 0 {
                    self.state.finish_configuration();
                }
            }
            Esp32S3HmacRegister::SetMessageIng
            | Esp32S3HmacRegister::SetMessageOne
            | Esp32S3HmacRegister::SetMessageEnd
            | Esp32S3HmacRegister::SetMessagePad
            | Esp32S3HmacRegister::OneBlock => {
                if command != 0 {
                    self.state.execute_message_command(register);
                }
            }
            Esp32S3HmacRegister::SetResultFinish => {
                if command != 0 {
                    self.state.clear_result();
                }
            }
            Esp32S3HmacRegister::SetInvalidateJtag | Esp32S3HmacRegister::SetInvalidateDs => {
                if command != 0 {
                    self.state.clear_result();
                }
            }
            Esp32S3HmacRegister::SoftJtagCtrl | Esp32S3HmacRegister::WrJtag => {
                self.state.registers.insert(register, command);
            }
            _ => {}
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
    fn computes_native_one_block_hmac_with_a_deterministic_key_slot() {
        let mut device = EspHmac::new("hmac");
        device
            .write(
                Esp32S3HmacRegister::SetStart.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3HmacRegister::SetParaPurpose.offset(),
                AccessWidth::Word,
                8,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3HmacRegister::SetParaKey.offset(),
                AccessWidth::Word,
                2,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3HmacRegister::SetParaFinish.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        let message = (0_u8..64).collect::<Vec<_>>();
        for (index, chunk) in message.chunks_exact(4).enumerate() {
            device
                .write(
                    Esp32S3HmacRegister::Wdata0.offset() + (index as u64 * 4),
                    AccessWidth::Word,
                    u64::from(u32::from_le_bytes(chunk.try_into().unwrap())),
                    SimTime::ZERO,
                )
                .unwrap();
        }
        device
            .write(
                Esp32S3HmacRegister::SetMessageOne.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();

        let expected = device.state.digest(&message);
        let actual = (0..8)
            .flat_map(|index| {
                u32::try_from(
                    device
                        .read(
                            Esp32S3HmacRegister::Rdata0.offset() + index * 4,
                            AccessWidth::Word,
                            SimTime::ZERO,
                        )
                        .unwrap(),
                )
                .unwrap()
                .to_le_bytes()
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        assert_eq!(
            device.read(
                Esp32S3HmacRegister::QueryError.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
        assert_eq!(
            device.read(
                Esp32S3HmacRegister::QueryBusy.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
    }

    #[test]
    fn rejects_hmac_commands_before_a_valid_configuration() {
        let mut device = EspHmac::new("hmac");
        device
            .write(
                Esp32S3HmacRegister::SetStart.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3HmacRegister::SetParaPurpose.offset(),
                AccessWidth::Word,
                4,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3HmacRegister::SetParaFinish.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3HmacRegister::SetMessageOne.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        assert_ne!(
            device.read(
                Esp32S3HmacRegister::QueryError.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
    }

    #[test]
    fn register_enum_covers_native_windows_and_rejects_invalid_access() {
        assert_eq!(Esp32S3HmacRegister::SetStart.offset(), 0x40);
        assert_eq!(Esp32S3HmacRegister::Wdata15.offset(), 0xbc);
        assert_eq!(Esp32S3HmacRegister::Rdata7.offset(), 0xdc);
        assert_eq!(Esp32S3HmacRegister::WrJtag.offset(), 0xfc);
        assert_eq!(Esp32S3HmacRegister::from_offset(0x70), None);
        assert_eq!(Esp32S3HmacRegister::from_offset(0xe0), None);
        assert_eq!(Esp32S3HmacRegister::SetParaPurpose.write_mask(), 0xf);
        assert_eq!(Esp32S3HmacRegister::QueryError.write_mask(), 0);
        assert_eq!(Esp32S3HmacRegister::Rdata0.read_mask(), u32::MAX);

        let mut device = EspHmac::new("hmac");
        assert!(device.read(0x70, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(
            device
                .write(
                    Esp32S3HmacRegister::SetStart.offset(),
                    AccessWidth::Word,
                    1_u64 << 32,
                    SimTime::ZERO,
                )
                .is_err()
        );
        assert!(
            device
                .write(
                    Esp32S3HmacRegister::Rdata0.offset(),
                    AccessWidth::Word,
                    0,
                    SimTime::ZERO,
                )
                .is_err()
        );
        assert!(
            device
                .write(
                    Esp32S3HmacRegister::QueryError.offset(),
                    AccessWidth::Word,
                    0,
                    SimTime::ZERO,
                )
                .is_err()
        );
    }

    #[test]
    fn command_strobes_are_read_zero_and_result_finish_clears_result() {
        let mut device = EspHmac::new("hmac");
        device
            .write(
                Esp32S3HmacRegister::SetStart.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Esp32S3HmacRegister::SetStart.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0
        );
        device
            .write(
                Esp32S3HmacRegister::SetParaPurpose.offset(),
                AccessWidth::Word,
                8,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3HmacRegister::SetParaFinish.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3HmacRegister::Wdata0.offset(),
                AccessWidth::Word,
                0x0403_0201,
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(
                Esp32S3HmacRegister::SetMessageOne.offset(),
                AccessWidth::Word,
                1,
                SimTime::ZERO,
            )
            .unwrap();
        assert_ne!(
            device
                .read(
                    Esp32S3HmacRegister::Rdata0.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0
        );
        device
            .write(
                Esp32S3HmacRegister::SetResultFinish.offset(),
                AccessWidth::Word,
                2,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            device
                .read(
                    Esp32S3HmacRegister::Rdata0.offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .unwrap(),
            0
        );
    }
}
