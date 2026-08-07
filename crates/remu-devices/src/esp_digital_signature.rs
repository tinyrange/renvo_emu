use super::*;
use sha2::{Digest, Sha256};

const DS_C_WORDS: usize = 396;
const DS_IV_WORDS: usize = 4;
const DS_OPERAND_WORDS: usize = 128;
const DS_DATE_RESET: u32 = 0x2019_1217;
const DS_CHECK_INVALID_DIGEST: u32 = 1 << 0;
const DS_CHECK_INVALID_PADDING: u32 = 1 << 1;

/// State of the dependent HMAC-to-DS key handoff.
///
/// The native `DS_QUERY_KEY_WRONG` register reports this handshake, not the
/// validity of the encrypted `C` parameter block. The default device fixture
/// assumes that HMAC has already supplied a usable key; tests that need to
/// exercise the handshake can select one of the other states.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EspDigitalSignatureHmacStatus {
    /// HMAC supplied the DS key and the DS block can accept `SET_ME`.
    Ready,
    /// HMAC has not been activated. Native status is zero while busy remains set.
    NotActivated,
    /// HMAC was activated but failed to deliver the key (native values 1..=15).
    Error(u8),
}

impl EspDigitalSignatureHmacStatus {
    fn key_wrong(self) -> u32 {
        match self {
            Self::Ready | Self::NotActivated => 0,
            Self::Error(value) => u32::from(value & 0xf),
        }
    }

    fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Native ESP32-S3 digital-signature register identifiers from Espressif's
/// `hwcrypto_reg.h` and the Digital Signature chapter of the TRM.
///
/// The encrypted C input is one contiguous 396-word write-only memory block
/// (Y, M, RB, and BOX subranges), while IV, X, Z, command, status, and date
/// registers are represented by distinct typed variants. Reserved holes in the
/// native page therefore fail explicitly instead of becoming generic storage.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Esp32S3DigitalSignatureRegister {
    /// One word of the contiguous encrypted C/Y/M/RB/BOX input block.
    C(u16),
    /// One word of the 128-bit initialization vector.
    Iv(u8),
    /// One word of the write-only X message block.
    X(u8),
    /// One word of the read-only signed result Z block.
    Z(u8),
    /// Activates the digital-signature peripheral.
    SetStart,
    /// Starts a digital-signature operation.
    SetMe,
    /// Ends the operation and clears its data windows.
    SetFinish,
    /// Busy status.
    QueryBusy,
    /// HMAC/DS key handoff status.
    QueryKeyWrong,
    /// Padding and message-digest check status.
    QueryCheck,
    /// Hardware version register.
    Date,
}

impl Esp32S3DigitalSignatureRegister {
    /// Returns the native byte offset in the digital-signature page.
    pub const fn offset(self) -> u64 {
        match self {
            Self::C(index) => index as u64 * 4,
            Self::Iv(index) => 0x630 + index as u64 * 4,
            Self::X(index) => 0x800 + index as u64 * 4,
            Self::Z(index) => 0xa00 + index as u64 * 4,
            Self::SetStart => 0xe00,
            Self::SetMe => 0xe04,
            Self::SetFinish => 0xe08,
            Self::QueryBusy => 0xe0c,
            Self::QueryKeyWrong => 0xe10,
            Self::QueryCheck => 0xe14,
            Self::Date => 0xe20,
        }
    }

    /// Resolves an aligned native byte offset. Reserved holes return `None`.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        if offset & 3 != 0 {
            return None;
        }
        match offset {
            0x000..=0x62c => Some(Self::C((offset / 4) as u16)),
            0x630..=0x63c => Some(Self::Iv(((offset - 0x630) / 4) as u8)),
            0x800..=0x9fc => Some(Self::X(((offset - 0x800) / 4) as u8)),
            0xa00..=0xbfc => Some(Self::Z(((offset - 0xa00) / 4) as u8)),
            0xe00 => Some(Self::SetStart),
            0xe04 => Some(Self::SetMe),
            0xe08 => Some(Self::SetFinish),
            0xe0c => Some(Self::QueryBusy),
            0xe10 => Some(Self::QueryKeyWrong),
            0xe14 => Some(Self::QueryCheck),
            0xe20 => Some(Self::Date),
            _ => None,
        }
    }

    /// Returns a typed C-memory word in the Y subrange.
    pub const fn c_y(index: u8) -> Option<Self> {
        if index < 128 {
            Some(Self::C(index as u16))
        } else {
            None
        }
    }

    /// Returns a typed C-memory word in the M subrange.
    pub const fn c_m(index: u8) -> Option<Self> {
        if index < 128 {
            Some(Self::C(128 + index as u16))
        } else {
            None
        }
    }

    /// Returns a typed C-memory word in the RB subrange.
    pub const fn c_rb(index: u8) -> Option<Self> {
        if index < 128 {
            Some(Self::C(256 + index as u16))
        } else {
            None
        }
    }

    /// Returns a typed C-memory word in the BOX subrange.
    pub const fn c_box(index: u8) -> Option<Self> {
        if index < 12 {
            Some(Self::C(384 + index as u16))
        } else {
            None
        }
    }

    /// Bits returned by a native read of this register.
    pub const fn read_mask(self) -> u32 {
        match self {
            Self::C(_) | Self::Iv(_) | Self::X(_) => 0,
            Self::Z(_) | Self::Date => {
                if matches!(self, Self::Date) {
                    0x3fff_ffff
                } else {
                    u32::MAX
                }
            }
            Self::QueryBusy => 1,
            Self::QueryKeyWrong => 0xf,
            Self::QueryCheck => 0x3,
            Self::SetStart | Self::SetMe | Self::SetFinish => 0,
        }
    }

    /// Bits accepted by a native write of this register.
    pub const fn write_mask(self) -> u32 {
        match self {
            Self::C(_) | Self::Iv(_) | Self::X(_) => u32::MAX,
            Self::SetStart | Self::SetMe | Self::SetFinish => 1,
            Self::Date => 0x3fff_ffff,
            Self::Z(_) | Self::QueryBusy | Self::QueryKeyWrong | Self::QueryCheck => 0,
        }
    }

    fn c_index(self) -> Option<usize> {
        match self {
            Self::C(index) => Some(index as usize),
            _ => None,
        }
    }

    fn iv_index(self) -> Option<usize> {
        match self {
            Self::Iv(index) => Some(index as usize),
            _ => None,
        }
    }

    fn x_index(self) -> Option<usize> {
        match self {
            Self::X(index) => Some(index as usize),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct EspDigitalSignatureState {
    c: [u32; DS_C_WORDS],
    iv: [u32; DS_IV_WORDS],
    x: [u32; DS_OPERAND_WORDS],
    z: [u32; DS_OPERAND_WORDS],
    started: bool,
    finished: bool,
    busy: bool,
    hmac_status: EspDigitalSignatureHmacStatus,
    key_wrong: u32,
    check: u32,
    date: u32,
}

impl Default for EspDigitalSignatureState {
    fn default() -> Self {
        Self {
            c: [0; DS_C_WORDS],
            iv: [0; DS_IV_WORDS],
            x: [0; DS_OPERAND_WORDS],
            z: [0; DS_OPERAND_WORDS],
            started: false,
            finished: false,
            busy: false,
            hmac_status: EspDigitalSignatureHmacStatus::Ready,
            key_wrong: 0,
            check: 0,
            date: DS_DATE_RESET,
        }
    }
}

impl EspDigitalSignatureState {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn c_is_present(&self) -> bool {
        self.c.iter().any(|word| *word != 0)
    }

    fn set_hmac_status(&mut self, status: EspDigitalSignatureHmacStatus) {
        self.hmac_status = status;
        self.key_wrong = status.key_wrong();
        if self.started {
            self.busy = !status.is_ready();
        }
    }

    fn clear_data_windows(&mut self) {
        self.c.fill(0);
        self.iv.fill(0);
        self.x.fill(0);
        self.z.fill(0);
    }

    fn calculate(&mut self) {
        if !self.started {
            self.check |= DS_CHECK_INVALID_PADDING;
            return;
        }
        if !self.hmac_status.is_ready() {
            // The native block remains busy while it waits for HMAC. The
            // caller can model the handoff by changing the status to Ready.
            self.busy = true;
            return;
        }
        if !self.c_is_present() {
            // A malformed/absent C block is reported through the MD check;
            // QUERY_KEY_WRONG is reserved for the HMAC key handoff above.
            self.check |= DS_CHECK_INVALID_DIGEST;
            return;
        }
        self.busy = true;
        let mut hasher = Sha256::new();
        hasher.update(
            self.x
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        hasher.update(
            self.iv
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        let digest = hasher.finalize();
        for (index, chunk) in digest.chunks_exact(4).enumerate() {
            self.z[index] =
                u32::from_le_bytes(chunk.try_into().expect("digest word is four bytes"));
        }
        self.busy = false;
    }
}

/// Functional ESP32-S3 digital-signature command and data-window block.
///
/// The native 396-word C input, four-word IV, 128-word X/Z windows, command,
/// status, and date registers are available with their documented access
/// direction and masks. A deterministic SHA-256-backed operation provides a
/// useful protocol and fault baseline; secure HMAC-derived RSA-PSS keys,
/// signature padding, provisioning, and hardware timing are intentionally not
/// claimed. The HMAC handoff status is modeled independently so
/// `QueryKeyWrong` retains its native meaning.
pub struct EspDigitalSignature {
    name: String,
    state: EspDigitalSignatureState,
}

impl EspDigitalSignature {
    /// Creates an idle digital-signature block.
    pub fn new(name: impl Into<String>) -> Self {
        Self::with_hmac_status(name, EspDigitalSignatureHmacStatus::Ready)
    }

    /// Creates the register-identical ESP32-C6 DS block with its native version word.
    pub fn new_esp32c6(name: impl Into<String>) -> Self {
        let mut device = Self::new(name);
        device.state.date = 538_969_624;
        device
    }

    /// Creates a digital-signature block with an explicit HMAC handoff state.
    pub fn with_hmac_status(
        name: impl Into<String>,
        hmac_status: EspDigitalSignatureHmacStatus,
    ) -> Self {
        let mut state = EspDigitalSignatureState::default();
        state.set_hmac_status(hmac_status);
        Self {
            name: name.into(),
            state,
        }
    }

    /// Changes the modeled HMAC handoff state.
    ///
    /// This is useful for tests and board models that include an HMAC device;
    /// normal firmware still observes the native busy and key-error registers.
    pub fn set_hmac_status(&mut self, hmac_status: EspDigitalSignatureHmacStatus) {
        self.state.set_hmac_status(hmac_status);
    }
}

impl Device for EspDigitalSignature {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP digital-signature requires aligned word access",
            ));
        }
        let register = Esp32S3DigitalSignatureRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 digital-signature register offset {offset:#x}"
            ))
        })?;
        if register.read_mask() == 0 {
            return Err(DeviceError::new(format!(
                "ESP32-S3 digital-signature register {register:?} is write-only"
            )));
        }
        let value = match register {
            Esp32S3DigitalSignatureRegister::Z(index) => self.state.z[index as usize],
            Esp32S3DigitalSignatureRegister::QueryBusy => u32::from(self.state.busy),
            Esp32S3DigitalSignatureRegister::QueryKeyWrong => self.state.key_wrong,
            Esp32S3DigitalSignatureRegister::QueryCheck => self.state.check,
            Esp32S3DigitalSignatureRegister::Date => self.state.date,
            _ => unreachable!("readable digital-signature register handled above"),
        };
        Ok(u64::from(value & register.read_mask()))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP digital-signature requires aligned word access",
            ));
        }
        let register = Esp32S3DigitalSignatureRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 digital-signature register offset {offset:#x}"
            ))
        })?;
        let value = u32::try_from(value).map_err(|_| {
            DeviceError::new(format!(
                "ESP32-S3 digital-signature word write exceeds 32 bits: {value:#x}"
            ))
        })?;
        let write_mask = register.write_mask();
        if write_mask == 0 {
            return Err(DeviceError::new(format!(
                "ESP32-S3 digital-signature register {register:?} is read-only"
            )));
        }
        let value = value & write_mask;

        if let Some(index) = register.c_index() {
            self.state.c[index] = value;
            return Ok(());
        }
        if let Some(index) = register.iv_index() {
            self.state.iv[index] = value;
            return Ok(());
        }
        if let Some(index) = register.x_index() {
            self.state.x[index] = value;
            return Ok(());
        }

        match register {
            Esp32S3DigitalSignatureRegister::SetStart => {
                if value != 0 {
                    self.state.started = true;
                    self.state.finished = false;
                    self.state.busy = !self.state.hmac_status.is_ready();
                    self.state.key_wrong = self.state.hmac_status.key_wrong();
                    self.state.check = 0;
                }
            }
            Esp32S3DigitalSignatureRegister::SetMe => {
                if value != 0 {
                    self.state.calculate();
                }
            }
            Esp32S3DigitalSignatureRegister::SetFinish => {
                if value != 0 {
                    if !self.state.started {
                        self.state.check |= DS_CHECK_INVALID_PADDING;
                    }
                    self.state.finished = true;
                    self.state.started = false;
                    self.state.busy = false;
                    self.state.clear_data_windows();
                }
            }
            Esp32S3DigitalSignatureRegister::Date => self.state.date = value,
            Esp32S3DigitalSignatureRegister::Z(_)
            | Esp32S3DigitalSignatureRegister::QueryBusy
            | Esp32S3DigitalSignatureRegister::QueryKeyWrong
            | Esp32S3DigitalSignatureRegister::QueryCheck => {
                unreachable!("read-only digital-signature register handled above")
            }
            Esp32S3DigitalSignatureRegister::C(_)
            | Esp32S3DigitalSignatureRegister::Iv(_)
            | Esp32S3DigitalSignatureRegister::X(_) => {
                unreachable!("digital-signature data window handled above")
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

    fn write_word(
        device: &mut EspDigitalSignature,
        register: Esp32S3DigitalSignatureRegister,
        value: u64,
    ) {
        device
            .write(register.offset(), AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn runs_native_signature_command_sequence_with_a_deterministic_result() {
        let mut device = EspDigitalSignature::new("digital-signature");
        write_word(&mut device, Esp32S3DigitalSignatureRegister::C(0), 1);
        for index in 0..16_u8 {
            write_word(
                &mut device,
                Esp32S3DigitalSignatureRegister::X(index),
                u64::from(index) + 1,
            );
        }
        write_word(&mut device, Esp32S3DigitalSignatureRegister::SetStart, 1);
        write_word(&mut device, Esp32S3DigitalSignatureRegister::SetMe, 1);
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryBusy.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryKeyWrong.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryCheck.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
        assert_ne!(
            device.read(
                Esp32S3DigitalSignatureRegister::Z(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
        write_word(&mut device, Esp32S3DigitalSignatureRegister::SetFinish, 1);
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::Z(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
    }

    #[test]
    fn reports_invalid_ciphertext_without_mislabeling_hmac_key() {
        let mut device = EspDigitalSignature::new("digital-signature");
        write_word(&mut device, Esp32S3DigitalSignatureRegister::SetStart, 1);
        write_word(&mut device, Esp32S3DigitalSignatureRegister::SetMe, 1);
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryBusy.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryKeyWrong.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryCheck.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(u64::from(DS_CHECK_INVALID_DIGEST))
        );
        write_word(&mut device, Esp32S3DigitalSignatureRegister::SetFinish, 1);
        write_word(&mut device, Esp32S3DigitalSignatureRegister::SetMe, 1);
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryCheck.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(u64::from(
                DS_CHECK_INVALID_DIGEST | DS_CHECK_INVALID_PADDING
            ))
        );
    }

    #[test]
    fn models_hmac_key_handoff_separately_from_parameter_validation() {
        let mut device = EspDigitalSignature::with_hmac_status(
            "digital-signature",
            EspDigitalSignatureHmacStatus::NotActivated,
        );
        write_word(&mut device, Esp32S3DigitalSignatureRegister::SetStart, 1);
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryBusy.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1)
        );
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryKeyWrong.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );

        device.set_hmac_status(EspDigitalSignatureHmacStatus::Error(7));
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryKeyWrong.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(7)
        );
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryBusy.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1)
        );

        device.set_hmac_status(EspDigitalSignatureHmacStatus::Ready);
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryBusy.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
        write_word(&mut device, Esp32S3DigitalSignatureRegister::C(0), 1);
        write_word(&mut device, Esp32S3DigitalSignatureRegister::SetMe, 1);
        assert_eq!(
            device.read(
                Esp32S3DigitalSignatureRegister::QueryCheck.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
    }

    #[test]
    fn register_contract_covers_native_sizes_masks_and_reserved_holes() {
        assert_eq!(
            Esp32S3DigitalSignatureRegister::from_offset(0x62c),
            Some(Esp32S3DigitalSignatureRegister::C(395))
        );
        assert_eq!(
            Esp32S3DigitalSignatureRegister::from_offset(0x630),
            Some(Esp32S3DigitalSignatureRegister::Iv(0))
        );
        assert_eq!(Esp32S3DigitalSignatureRegister::from_offset(0x640), None);
        assert_eq!(Esp32S3DigitalSignatureRegister::QueryCheck.read_mask(), 0x3);
        assert_eq!(
            Esp32S3DigitalSignatureRegister::Date.write_mask(),
            0x3fff_ffff
        );

        let mut device = EspDigitalSignature::new("digital-signature");
        assert!(
            device
                .read(
                    Esp32S3DigitalSignatureRegister::C(0).offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .is_err()
        );
        assert!(
            device
                .write(
                    Esp32S3DigitalSignatureRegister::Z(0).offset(),
                    AccessWidth::Word,
                    1,
                    SimTime::ZERO,
                )
                .is_err()
        );
        assert!(
            device
                .read(0x640, AccessWidth::Word, SimTime::ZERO)
                .is_err()
        );
        assert!(
            device
                .write(
                    Esp32S3DigitalSignatureRegister::Date.offset(),
                    AccessWidth::Word,
                    u64::from(u32::MAX) + 1,
                    SimTime::ZERO,
                )
                .is_err()
        );
    }
}
