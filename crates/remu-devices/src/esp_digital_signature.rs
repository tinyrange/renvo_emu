use super::*;
use sha2::{Digest, Sha256};

const DS_C_BASE: u64 = 0x000;
const DS_C_Y_BASE: u64 = DS_C_BASE;
const DS_C_M_BASE: u64 = 0x200;
const DS_C_RB_BASE: u64 = 0x400;
const DS_C_BOX_BASE: u64 = 0x600;
const DS_IV_BASE: u64 = 0x630;
const DS_X_BASE: u64 = 0x800;
const DS_Z_BASE: u64 = 0xa00;
const DS_WINDOW_BYTES: usize = 0x200;
const DS_SET_START: u64 = 0xe00;
const DS_SET_ME: u64 = 0xe04;
const DS_SET_FINISH: u64 = 0xe08;
const DS_QUERY_BUSY: u64 = 0xe0c;
const DS_QUERY_KEY_WRONG: u64 = 0xe10;
const DS_QUERY_CHECK: u64 = 0xe14;
const DS_DATE: u64 = 0xe20;
const DS_CHECK_INVALID_DIGEST: u32 = 1 << 0;
const DS_CHECK_INVALID_PADDING: u32 = 1 << 1;

struct EspDigitalSignatureState {
    registers: Vec<u32>,
    started: bool,
    finished: bool,
    busy: bool,
    key_wrong: bool,
    check: u32,
}

impl EspDigitalSignatureState {
    fn new() -> Self {
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            started: false,
            finished: false,
            busy: false,
            key_wrong: false,
            check: 0,
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.started = false;
        self.finished = false;
        self.busy = false;
        self.key_wrong = false;
        self.check = 0;
        self.registers[(DS_DATE / 4) as usize] = 0x2025_0001;
    }

    fn window_bytes(&self, base: u64, length: usize) -> Vec<u8> {
        let start = (base / 4) as usize;
        self.registers[start..start + length / 4]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect()
    }

    fn c_is_present(&self) -> bool {
        [DS_C_Y_BASE, DS_C_M_BASE, DS_C_RB_BASE, DS_C_BOX_BASE]
            .into_iter()
            .any(|base| {
                self.window_bytes(base, DS_WINDOW_BYTES)
                    .iter()
                    .any(|byte| *byte != 0)
            })
    }

    fn calculate(&mut self) {
        if !self.started {
            self.check |= DS_CHECK_INVALID_PADDING;
            return;
        }
        if !self.c_is_present() {
            self.key_wrong = true;
            self.check |= DS_CHECK_INVALID_DIGEST;
            return;
        }
        self.busy = true;
        let mut hasher = Sha256::new();
        hasher.update(self.window_bytes(DS_X_BASE, DS_WINDOW_BYTES));
        hasher.update(self.window_bytes(DS_IV_BASE, 16));
        let digest = hasher.finalize();
        let start = (DS_Z_BASE / 4) as usize;
        self.registers[start..start + 8].copy_from_slice(
            &digest
                .chunks_exact(4)
                .map(|chunk| {
                    u32::from_le_bytes(chunk.try_into().expect("digest word is four bytes"))
                })
                .collect::<Vec<_>>(),
        );
        self.busy = false;
    }
}

/// Functional ESP32-S3 digital-signature command and data-window block.
///
/// The native C/Y/M/RB/BOX/IV/X/Z windows and command/status registers are
/// available. A deterministic SHA-256-backed operation provides a useful
/// protocol and fault baseline; secure HMAC-derived RSA-PSS keys, signature
/// padding, and hardware timing are intentionally not claimed.
pub struct EspDigitalSignature {
    name: String,
    state: EspDigitalSignatureState,
}

impl EspDigitalSignature {
    /// Creates an idle digital-signature block.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: EspDigitalSignatureState::new(),
        }
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
        let index = usize::try_from(offset / 4).expect("digital-signature offset fits");
        if index >= self.state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} read at {offset:#x}",
                self.name
            )));
        }
        let value = match offset {
            DS_QUERY_BUSY => u32::from(self.state.busy),
            DS_QUERY_KEY_WRONG => u32::from(self.state.key_wrong),
            DS_QUERY_CHECK => self.state.check,
            _ => self.state.registers[index],
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
            return Err(DeviceError::new(
                "ESP digital-signature requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("digital-signature offset fits");
        if index >= self.state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = value as u32;
        match offset {
            DS_SET_START => {
                self.state.started = value != 0;
                self.state.finished = false;
                self.state.busy = false;
                self.state.key_wrong = false;
                self.state.check = 0;
            }
            DS_SET_ME => self.state.calculate(),
            DS_SET_FINISH => {
                if !self.state.started {
                    self.state.check |= DS_CHECK_INVALID_PADDING;
                } else {
                    self.state.finished = value != 0;
                }
            }
            DS_QUERY_BUSY | DS_QUERY_KEY_WRONG | DS_QUERY_CHECK => {}
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
    fn runs_native_signature_command_sequence_with_a_deterministic_result() {
        let mut device = EspDigitalSignature::new("digital-signature");
        device
            .write(DS_C_BASE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        for index in 0..16_u64 {
            device
                .write(
                    DS_X_BASE + index * 4,
                    AccessWidth::Word,
                    index + 1,
                    SimTime::ZERO,
                )
                .unwrap();
        }
        device
            .write(DS_SET_START, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(DS_SET_ME, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(DS_SET_FINISH, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(DS_QUERY_BUSY, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
        assert_eq!(
            device.read(DS_QUERY_KEY_WRONG, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
        assert_eq!(
            device.read(DS_QUERY_CHECK, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
        assert_ne!(
            device.read(DS_Z_BASE, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
    }

    #[test]
    fn reports_missing_key_and_invalid_sequence() {
        let mut device = EspDigitalSignature::new("digital-signature");
        device
            .write(DS_SET_START, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(DS_SET_ME, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(DS_QUERY_KEY_WRONG, AccessWidth::Word, SimTime::ZERO),
            Ok(1)
        );
        device
            .write(DS_SET_START, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(DS_SET_FINISH, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device.read(DS_QUERY_CHECK, AccessWidth::Word, SimTime::ZERO),
            Ok(u64::from(DS_CHECK_INVALID_PADDING))
        );
    }
}
