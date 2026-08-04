use super::*;
use num_bigint::BigUint;

const RSA_WINDOW_WORDS: usize = 128;
const RSA_DATE_RESET: u32 = 0x2019_0425;

/// Native ESP32-S3 RSA register identifiers from Espressif's
/// `hwcrypto_reg.h` and RSA accelerator register contract.
///
/// The four operand memories are represented by typed window variants. Their
/// indices are validated by [`Esp32S3RsaRegister::from_offset`], so accesses
/// to the native reserved holes cannot become accidental backing storage.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Esp32S3RsaRegister {
    /// Write-only modulus memory word.
    M(u8),
    /// Read/write result (RB/Z) memory word.
    RbZ(u8),
    /// Write-only exponent/input Y memory word.
    Y(u8),
    /// Write-only base/input X memory word.
    X(u8),
    /// Montgomery M-prime configuration register.
    MPrime,
    /// Operand length minus one, in 32-bit words.
    Length,
    /// Memory-initialization complete status.
    Clean,
    /// Modular exponentiation start strobe.
    ModeExpStart,
    /// Modular multiplication start strobe.
    ModMultStart,
    /// Multiplication start strobe.
    MultStart,
    /// Accelerator idle status.
    Idle,
    /// Interrupt clear strobe.
    ClearInterrupt,
    /// Constant-time acceleration option.
    ConstantTime,
    /// Search acceleration enable option.
    SearchEnable,
    /// Search acceleration position.
    SearchPos,
    /// Completion interrupt enable.
    InterruptEna,
    /// Hardware version register.
    Date,
}

impl Esp32S3RsaRegister {
    /// Returns the native byte offset in the RSA peripheral page.
    pub const fn offset(self) -> u64 {
        match self {
            Self::M(index) => index as u64 * 4,
            Self::RbZ(index) => 0x200 + index as u64 * 4,
            Self::Y(index) => 0x400 + index as u64 * 4,
            Self::X(index) => 0x600 + index as u64 * 4,
            Self::MPrime => 0x800,
            Self::Length => 0x804,
            Self::Clean => 0x808,
            Self::ModeExpStart => 0x80c,
            Self::ModMultStart => 0x810,
            Self::MultStart => 0x814,
            Self::Idle => 0x818,
            Self::ClearInterrupt => 0x81c,
            Self::ConstantTime => 0x820,
            Self::SearchEnable => 0x824,
            Self::SearchPos => 0x828,
            Self::InterruptEna => 0x82c,
            Self::Date => 0x830,
        }
    }

    /// Resolves an aligned native byte offset. Reserved holes return `None`.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        if offset & 3 != 0 {
            return None;
        }
        match offset {
            0x000..=0x1fc => Some(Self::M((offset / 4) as u8)),
            0x200..=0x3fc => Some(Self::RbZ(((offset - 0x200) / 4) as u8)),
            0x400..=0x5fc => Some(Self::Y(((offset - 0x400) / 4) as u8)),
            0x600..=0x7fc => Some(Self::X(((offset - 0x600) / 4) as u8)),
            0x800 => Some(Self::MPrime),
            0x804 => Some(Self::Length),
            0x808 => Some(Self::Clean),
            0x80c => Some(Self::ModeExpStart),
            0x810 => Some(Self::ModMultStart),
            0x814 => Some(Self::MultStart),
            0x818 => Some(Self::Idle),
            0x81c => Some(Self::ClearInterrupt),
            0x820 => Some(Self::ConstantTime),
            0x824 => Some(Self::SearchEnable),
            0x828 => Some(Self::SearchPos),
            0x82c => Some(Self::InterruptEna),
            0x830 => Some(Self::Date),
            _ => None,
        }
    }

    /// Bits returned by a native read of this register.
    pub const fn read_mask(self) -> u32 {
        match self {
            Self::M(_) | Self::Y(_) | Self::X(_) => 0,
            Self::Length => 0x7f,
            Self::Clean | Self::Idle | Self::ConstantTime | Self::SearchEnable => 1,
            Self::SearchPos => 0x0fff,
            Self::InterruptEna => 1,
            Self::Date => 0x3fff_ffff,
            _ => u32::MAX,
        }
    }

    /// Bits accepted by a native write of this register.
    pub const fn write_mask(self) -> u32 {
        match self {
            Self::M(_) | Self::RbZ(_) | Self::Y(_) | Self::X(_) | Self::MPrime => u32::MAX,
            Self::Length => 0x7f,
            Self::ModeExpStart | Self::ModMultStart | Self::MultStart | Self::ClearInterrupt => 1,
            Self::ConstantTime | Self::SearchEnable | Self::InterruptEna => 1,
            Self::SearchPos => 0x0fff,
            Self::Date => 0x3fff_ffff,
            Self::Clean | Self::Idle => 0,
        }
    }

    fn is_m(self) -> bool {
        matches!(self, Self::M(_))
    }

    fn is_y(self) -> bool {
        matches!(self, Self::Y(_))
    }

    fn is_x(self) -> bool {
        matches!(self, Self::X(_))
    }

    fn index(self) -> Option<usize> {
        match self {
            Self::M(index) | Self::RbZ(index) | Self::Y(index) | Self::X(index) => {
                Some(index as usize)
            }
            _ => None,
        }
    }
}

#[derive(Debug)]
struct EspRsaState {
    m: [u32; RSA_WINDOW_WORDS],
    z: [u32; RSA_WINDOW_WORDS],
    y: [u32; RSA_WINDOW_WORDS],
    x: [u32; RSA_WINDOW_WORDS],
    m_prime: u32,
    length: u32,
    clean: bool,
    idle: bool,
    interrupt_pending: bool,
    interrupt_enable: bool,
    constant_time: bool,
    search_enable: bool,
    search_pos: u32,
    date: u32,
}

impl Default for EspRsaState {
    fn default() -> Self {
        Self {
            m: [0; RSA_WINDOW_WORDS],
            z: [0; RSA_WINDOW_WORDS],
            y: [0; RSA_WINDOW_WORDS],
            x: [0; RSA_WINDOW_WORDS],
            m_prime: 0,
            length: 0,
            // The hardware performs memory initialization during reset. This
            // functional model completes that deterministic action at reset.
            clean: true,
            idle: true,
            interrupt_pending: false,
            // RSA interrupts are enabled by default in the native reset map.
            interrupt_enable: true,
            constant_time: true,
            search_enable: false,
            search_pos: 0,
            date: RSA_DATE_RESET,
        }
    }
}

impl EspRsaState {
    fn operand_words(&self) -> usize {
        (self.length as usize + 1).clamp(1, RSA_WINDOW_WORDS)
    }

    fn read_integer(words: &[u32]) -> BigUint {
        let bytes = words
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>();
        BigUint::from_bytes_le(&bytes)
    }

    fn write_integer(words: &mut [u32], value: &BigUint) {
        words.fill(0);
        for (index, chunk) in value.to_bytes_le().chunks(4).enumerate().take(words.len()) {
            let mut bytes = [0_u8; 4];
            bytes[..chunk.len()].copy_from_slice(chunk);
            words[index] = u32::from_le_bytes(bytes);
        }
    }

    fn complete(&mut self, result: BigUint, words: usize) {
        Self::write_integer(&mut self.z[..words], &result);
        self.finish();
    }

    fn finish(&mut self) {
        self.idle = true;
        if self.interrupt_enable {
            self.interrupt_pending = true;
        }
    }

    fn calculate(&mut self, operation: Esp32S3RsaRegister) {
        self.idle = false;
        match operation {
            Esp32S3RsaRegister::ModeExpStart | Esp32S3RsaRegister::ModMultStart => {
                let words = self.operand_words();
                let modulus = Self::read_integer(&self.m[..words]);
                if modulus == BigUint::from(0_u8) {
                    // There is no valid result, but the hardware still
                    // reaches the completion/idle state for a started
                    // operation. Keep the completion interrupt observable
                    // rather than leaving firmware polling forever.
                    self.finish();
                    return;
                }
                let x = Self::read_integer(&self.x[..words]);
                let y = Self::read_integer(&self.y[..words]);
                let result = if operation == Esp32S3RsaRegister::ModeExpStart {
                    x.modpow(&y, &modulus)
                } else {
                    (x * y) % modulus
                };
                self.complete(result, words);
            }
            Esp32S3RsaRegister::MultStart => {
                // In multiply mode MODE+1 is the result length. X and the
                // upper half of Z are the two half-length inputs.
                let output_words = self.operand_words();
                if output_words < 2 || output_words % 2 != 0 {
                    self.finish();
                    return;
                }
                let input_words = output_words / 2;
                let x = Self::read_integer(&self.x[..input_words]);
                let y = Self::read_integer(&self.z[input_words..output_words]);
                self.complete(x * y, output_words);
            }
            _ => {}
        }
    }

    fn read_word(&self, register: Esp32S3RsaRegister) -> Result<u32, DeviceError> {
        if register.is_m() || register.is_y() || register.is_x() {
            return Err(DeviceError::new(
                "ESP32-S3 RSA M, Y, and X memory windows are write-only",
            ));
        }
        if let Some(index) = register.index() {
            return Ok(match register {
                Esp32S3RsaRegister::RbZ(_) => self.z[index],
                _ => unreachable!("write-only RSA window handled above"),
            });
        }
        Ok(match register {
            Esp32S3RsaRegister::MPrime => self.m_prime,
            Esp32S3RsaRegister::Length => self.length,
            Esp32S3RsaRegister::Clean => u32::from(self.clean),
            Esp32S3RsaRegister::Idle => u32::from(self.idle),
            Esp32S3RsaRegister::ModeExpStart
            | Esp32S3RsaRegister::ModMultStart
            | Esp32S3RsaRegister::MultStart
            | Esp32S3RsaRegister::ClearInterrupt => 0,
            Esp32S3RsaRegister::ConstantTime => u32::from(self.constant_time),
            Esp32S3RsaRegister::SearchEnable => u32::from(self.search_enable),
            Esp32S3RsaRegister::SearchPos => self.search_pos,
            Esp32S3RsaRegister::InterruptEna => u32::from(self.interrupt_enable),
            Esp32S3RsaRegister::Date => self.date,
            Esp32S3RsaRegister::M(_)
            | Esp32S3RsaRegister::RbZ(_)
            | Esp32S3RsaRegister::Y(_)
            | Esp32S3RsaRegister::X(_) => unreachable!("RSA memory handled above"),
        })
    }
}

/// Functional ESP32-S3 RSA multiple-precision accelerator.
///
/// The native M/RB-Z/Y/X memory windows and configuration/status registers are
/// exposed with their documented access masks. Modular exponentiation,
/// modular multiplication, and multiplication use deterministic arbitrary-
/// precision host arithmetic while preserving the configured native limb
/// length. The model is functional rather than cycle-accurate and does not
/// claim hardware blinding, constant-time timing, DMA, or secure-key fidelity.
pub struct EspRsa {
    name: String,
    state: EspRsaState,
}

impl EspRsa {
    /// Creates an idle RSA accelerator.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: EspRsaState::default(),
        }
    }
}

impl Device for EspRsa {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP RSA requires aligned word access"));
        }
        let register = Esp32S3RsaRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 RSA register offset {offset:#x}"
            ))
        })?;
        Ok(u64::from(
            self.state.read_word(register)? & register.read_mask(),
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP RSA requires aligned word access"));
        }
        let register = Esp32S3RsaRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unsupported ESP32-S3 RSA register offset {offset:#x}"
            ))
        })?;
        let value = u32::try_from(value).map_err(|_| {
            DeviceError::new(format!(
                "ESP32-S3 RSA word write exceeds 32 bits: {value:#x}"
            ))
        })?;
        let write_mask = register.write_mask();
        if write_mask == 0 {
            return Err(DeviceError::new(format!(
                "ESP32-S3 RSA register {register:?} is read-only"
            )));
        }
        let value = value & write_mask;

        if let Some(index) = register.index() {
            match register {
                Esp32S3RsaRegister::M(_) => self.state.m[index] = value,
                Esp32S3RsaRegister::RbZ(_) => self.state.z[index] = value,
                Esp32S3RsaRegister::Y(_) => self.state.y[index] = value,
                Esp32S3RsaRegister::X(_) => self.state.x[index] = value,
                _ => unreachable!("RSA memory index only exists on a memory variant"),
            }
            return Ok(());
        }

        match register {
            Esp32S3RsaRegister::MPrime => self.state.m_prime = value,
            Esp32S3RsaRegister::Length => self.state.length = value,
            Esp32S3RsaRegister::ModeExpStart
            | Esp32S3RsaRegister::ModMultStart
            | Esp32S3RsaRegister::MultStart => {
                if value != 0 {
                    self.state.calculate(register);
                }
            }
            Esp32S3RsaRegister::ClearInterrupt => {
                if value != 0 {
                    self.state.interrupt_pending = false;
                }
            }
            Esp32S3RsaRegister::ConstantTime => self.state.constant_time = value != 0,
            Esp32S3RsaRegister::SearchEnable => self.state.search_enable = value != 0,
            Esp32S3RsaRegister::SearchPos => self.state.search_pos = value,
            Esp32S3RsaRegister::InterruptEna => self.state.interrupt_enable = value != 0,
            Esp32S3RsaRegister::Date => self.state.date = value,
            Esp32S3RsaRegister::Clean
            | Esp32S3RsaRegister::Idle
            | Esp32S3RsaRegister::M(_)
            | Esp32S3RsaRegister::RbZ(_)
            | Esp32S3RsaRegister::Y(_)
            | Esp32S3RsaRegister::X(_) => {
                unreachable!("read-only or memory register handled above")
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state = EspRsaState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_word(device: &mut EspRsa, register: Esp32S3RsaRegister, value: u64) {
        device
            .write(register.offset(), AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn performs_native_modular_exponentiation_and_interrupt_completion() {
        let mut device = EspRsa::new("rsa");
        write_word(&mut device, Esp32S3RsaRegister::Length, 0);
        write_word(&mut device, Esp32S3RsaRegister::M(0), 55);
        write_word(&mut device, Esp32S3RsaRegister::X(0), 7);
        write_word(&mut device, Esp32S3RsaRegister::Y(0), 13);
        write_word(&mut device, Esp32S3RsaRegister::ModeExpStart, 1);
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::RbZ(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(2)
        );
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::Clean.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1)
        );
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::Idle.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1)
        );
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::InterruptEna.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1)
        );
        // The native interrupt is exposed by the enabled completion path.
        assert!(device.state.interrupt_pending);
        write_word(&mut device, Esp32S3RsaRegister::ClearInterrupt, 1);
        assert!(!device.state.interrupt_pending);
    }

    #[test]
    fn supports_modular_multiply_and_multiword_limbs() {
        let mut device = EspRsa::new("rsa");
        write_word(&mut device, Esp32S3RsaRegister::Length, 1);
        write_word(
            &mut device,
            Esp32S3RsaRegister::M(0),
            u64::from(u32::MAX - 4),
        );
        write_word(&mut device, Esp32S3RsaRegister::M(1), 1);
        write_word(
            &mut device,
            Esp32S3RsaRegister::X(0),
            u64::from(u32::MAX - 1),
        );
        write_word(&mut device, Esp32S3RsaRegister::X(1), 1);
        write_word(&mut device, Esp32S3RsaRegister::Y(0), 3);
        write_word(&mut device, Esp32S3RsaRegister::ModMultStart, 1);
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::RbZ(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(9)
        );
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::RbZ(1).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(0)
        );
    }

    #[test]
    fn multiplication_uses_the_upper_result_window_as_the_second_input() {
        let mut device = EspRsa::new("rsa");
        // MODE+1 is the four-word result length, so X and upper-Z are
        // two-word inputs: (2^32 + 1) * 3.
        write_word(&mut device, Esp32S3RsaRegister::Length, 3);
        write_word(&mut device, Esp32S3RsaRegister::X(0), 1);
        write_word(&mut device, Esp32S3RsaRegister::X(1), 1);
        write_word(&mut device, Esp32S3RsaRegister::RbZ(2), 3);
        write_word(&mut device, Esp32S3RsaRegister::MultStart, 1);
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::RbZ(0).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(3)
        );
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::RbZ(1).offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(3)
        );
    }

    #[test]
    fn started_invalid_operations_still_reach_idle_and_signal_completion() {
        let mut device = EspRsa::new("rsa");
        // A zero modulus cannot produce a modular result, but a started
        // operation must not leave firmware waiting forever for idle/IRQ.
        write_word(&mut device, Esp32S3RsaRegister::ModeExpStart, 1);
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::Idle.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1)
        );
        assert!(device.state.interrupt_pending);

        write_word(&mut device, Esp32S3RsaRegister::ClearInterrupt, 1);
        write_word(&mut device, Esp32S3RsaRegister::MultStart, 1);
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::Idle.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1)
        );
        assert!(device.state.interrupt_pending);
    }

    #[test]
    fn register_contract_rejects_holes_wide_access_and_reading_write_only_windows() {
        assert_eq!(
            Esp32S3RsaRegister::from_offset(0x830),
            Some(Esp32S3RsaRegister::Date)
        );
        assert_eq!(Esp32S3RsaRegister::from_offset(0x834), None);
        assert_eq!(Esp32S3RsaRegister::Length.read_mask(), 0x7f);
        assert_eq!(Esp32S3RsaRegister::SearchPos.write_mask(), 0x0fff);

        let mut device = EspRsa::new("rsa");
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::Date.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(u64::from(RSA_DATE_RESET))
        );
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::Clean.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1)
        );
        assert_eq!(
            device.read(
                Esp32S3RsaRegister::InterruptEna.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            ),
            Ok(1)
        );
        assert!(
            device
                .read(
                    Esp32S3RsaRegister::M(0).offset(),
                    AccessWidth::Word,
                    SimTime::ZERO,
                )
                .is_err()
        );
        assert!(
            device
                .read(0x834, AccessWidth::Word, SimTime::ZERO)
                .is_err()
        );
        assert!(
            device
                .write(
                    Esp32S3RsaRegister::Length.offset(),
                    AccessWidth::Word,
                    u64::from(u32::MAX) + 1,
                    SimTime::ZERO,
                )
                .is_err()
        );
    }
}
