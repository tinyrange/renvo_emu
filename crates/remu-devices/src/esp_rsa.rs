use super::*;
use num_bigint::BigUint;

const RSA_M_BASE: u64 = 0x000;
const RSA_RB_Z_BASE: u64 = 0x200;
const RSA_Y_BASE: u64 = 0x400;
const RSA_X_BASE: u64 = 0x600;
const RSA_WINDOW_BYTES: usize = 0x200;
const RSA_M_DASH: u64 = 0x800;
const RSA_LENGTH: u64 = 0x804;
const RSA_QUERY_CLEAN: u64 = 0x808;
const RSA_MODEXP_START: u64 = 0x80c;
const RSA_MOD_MULT_START: u64 = 0x810;
const RSA_MULT_START: u64 = 0x814;
const RSA_QUERY_INTERRUPT: u64 = 0x818;
const RSA_CLEAR_INTERRUPT: u64 = 0x81c;
const RSA_CONSTANT_TIME: u64 = 0x820;
const RSA_SEARCH_OPEN: u64 = 0x824;
const RSA_SEARCH_POS: u64 = 0x828;
const RSA_INTERRUPT: u64 = 0x82c;
const RSA_MAX_WORDS: usize = RSA_WINDOW_BYTES / 4;

struct EspRsaState {
    registers: Vec<u32>,
    interrupt: bool,
    clean: bool,
}

impl EspRsaState {
    fn new() -> Self {
        Self {
            registers: vec![0; 0x1000 / 4],
            interrupt: false,
            clean: true,
        }
    }

    fn word_count(&self) -> usize {
        self.registers[(RSA_LENGTH / 4) as usize]
            .saturating_add(1)
            .try_into()
            .unwrap_or(RSA_MAX_WORDS)
            .clamp(1, RSA_MAX_WORDS)
    }

    fn read_integer(&self, base: u64) -> BigUint {
        let words = self.word_count();
        let start = (base / 4) as usize;
        let end = start + words;
        let bytes = self.registers[start..end]
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect::<Vec<_>>();
        BigUint::from_bytes_le(&bytes)
    }

    fn write_integer(&mut self, base: u64, value: &BigUint) {
        let start = (base / 4) as usize;
        let words = self.word_count();
        let end = start + words;
        self.registers[start..end].fill(0);
        for (index, chunk) in value.to_bytes_le().chunks(4).take(words).enumerate() {
            let mut bytes = [0_u8; 4];
            bytes[..chunk.len()].copy_from_slice(chunk);
            self.registers[start + index] = u32::from_le_bytes(bytes);
        }
    }

    fn complete(&mut self, result: BigUint) {
        self.write_integer(RSA_RB_Z_BASE, &result);
        self.interrupt = true;
        self.clean = true;
    }

    fn calculate(&mut self, operation: u64) {
        let modulus = self.read_integer(RSA_M_BASE);
        if modulus == BigUint::from(0_u8) {
            self.clean = false;
            return;
        }
        let x = self.read_integer(RSA_X_BASE);
        let y = self.read_integer(RSA_Y_BASE);
        let result = match operation {
            RSA_MODEXP_START => x.modpow(&y, &modulus),
            RSA_MOD_MULT_START => (x * y) % modulus,
            RSA_MULT_START => x * y,
            _ => return,
        };
        self.complete(result);
    }
}

/// Functional ESP32-S3 RSA multiple-precision accelerator.
///
/// The native M/RB-Z/Y/X memory windows and operation registers are exposed.
/// Modular exponentiation, modular multiplication, and multiplication use
/// arbitrary-precision host arithmetic while preserving the configured native
/// limb length. The model is functional rather than cycle-accurate and does
/// not claim hardware blinding or accelerator timing.
pub struct EspRsa {
    name: String,
    state: EspRsaState,
}

impl EspRsa {
    /// Creates an idle RSA accelerator.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: EspRsaState::new(),
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
        let index = usize::try_from(offset / 4).expect("RSA offset fits");
        if index >= self.state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} read at {offset:#x}",
                self.name
            )));
        }
        match offset {
            RSA_QUERY_CLEAN => Ok(u64::from(self.state.clean)),
            RSA_QUERY_INTERRUPT => Ok(u64::from(self.state.interrupt)),
            _ => Ok(u64::from(self.state.registers[index])),
        }
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
        let index = usize::try_from(offset / 4).expect("RSA offset fits");
        if index >= self.state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = value as u32;
        match offset {
            RSA_MODEXP_START | RSA_MOD_MULT_START | RSA_MULT_START => {
                self.state.registers[index] = value;
                self.state.clean = false;
                self.state.calculate(offset);
            }
            RSA_QUERY_CLEAN => self.state.clean = true,
            RSA_CLEAR_INTERRUPT => {
                self.state.interrupt = false;
                self.state.registers[index] = 0;
            }
            RSA_QUERY_INTERRUPT => {}
            RSA_M_DASH | RSA_CONSTANT_TIME | RSA_SEARCH_OPEN | RSA_SEARCH_POS => {
                self.state.registers[index] = value;
            }
            RSA_INTERRUPT => self.state.registers[index] = value,
            _ => self.state.registers[index] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state = EspRsaState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_word(device: &mut EspRsa, base: u64, value: u64) {
        device
            .write(base, AccessWidth::Word, value, SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn performs_native_modular_exponentiation_and_interrupt_completion() {
        let mut device = EspRsa::new("rsa");
        write_word(&mut device, RSA_LENGTH, 0);
        write_word(&mut device, RSA_M_BASE, 55);
        write_word(&mut device, RSA_X_BASE, 7);
        write_word(&mut device, RSA_Y_BASE, 13);
        write_word(&mut device, RSA_MODEXP_START, 1);
        assert_eq!(
            device.read(RSA_RB_Z_BASE, AccessWidth::Word, SimTime::ZERO),
            Ok(2)
        );
        assert_eq!(
            device.read(RSA_QUERY_CLEAN, AccessWidth::Word, SimTime::ZERO),
            Ok(1)
        );
        assert_eq!(
            device.read(RSA_QUERY_INTERRUPT, AccessWidth::Word, SimTime::ZERO),
            Ok(1)
        );
        write_word(&mut device, RSA_CLEAR_INTERRUPT, 1);
        assert_eq!(
            device.read(RSA_QUERY_INTERRUPT, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
    }

    #[test]
    fn supports_modular_multiply_and_multiword_limbs() {
        let mut device = EspRsa::new("rsa");
        write_word(&mut device, RSA_LENGTH, 1);
        write_word(&mut device, RSA_M_BASE, u64::from(u32::MAX - 4));
        write_word(&mut device, RSA_M_BASE + 4, 1);
        write_word(&mut device, RSA_X_BASE, u64::from(u32::MAX - 1));
        write_word(&mut device, RSA_X_BASE + 4, 1);
        write_word(&mut device, RSA_Y_BASE, 3);
        write_word(&mut device, RSA_MOD_MULT_START, 1);
        assert_eq!(
            device.read(RSA_RB_Z_BASE, AccessWidth::Word, SimTime::ZERO),
            Ok(9)
        );
        assert_eq!(
            device.read(RSA_RB_Z_BASE + 4, AccessWidth::Word, SimTime::ZERO),
            Ok(0)
        );
    }
}
