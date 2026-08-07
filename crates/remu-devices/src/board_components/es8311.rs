use remu_core::SimTime;
use serde::Serialize;
use thiserror::Error;

/// ES8311 address used by the `M5StickS3`.
pub const ES8311_ADDRESS: u8 = 0x18;

/// ES8311 register transaction error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum Es8311Error {
    /// No register pointer was supplied.
    #[error("ES8311 transaction must include a register pointer")]
    MissingRegister,
    /// A combined read also supplied write payload bytes.
    #[error("ES8311 read transaction may contain only its register pointer")]
    ReadWriteOverlap,
    /// A transaction crossed the byte register space.
    #[error("ES8311 register span {register:#04x}+{length} exceeds the register space")]
    RegisterRange {
        /// First requested register.
        register: u8,
        /// Requested transfer length.
        length: usize,
    },
}

/// Stable ES8311 codec state used by board qualification artifacts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Es8311Snapshot {
    /// Whether the codec control state machine is powered.
    pub powered: bool,
    /// Whether the microphone ADC path is enabled.
    pub adc_enabled: bool,
    /// Whether the speaker DAC path is enabled.
    pub dac_enabled: bool,
    /// Raw ADC volume register.
    pub adc_volume: u8,
    /// Raw DAC volume register.
    pub dac_volume: u8,
    /// Accepted I2C transaction count.
    pub transactions: u64,
}

/// Deterministic ES8311 control-plane model.
#[derive(Clone, Debug)]
pub struct Es8311 {
    registers: [u8; 256],
    transactions: u64,
}

impl Es8311 {
    /// Creates a codec in its deterministic powered-down state.
    pub fn new() -> Self {
        Self {
            registers: [0; 256],
            transactions: 0,
        }
    }

    /// Executes a register-pointer I2C transaction.
    pub fn transact(
        &mut self,
        write: &[u8],
        read_len: usize,
        _at: SimTime,
    ) -> Result<Vec<u8>, Es8311Error> {
        let Some(&register) = write.first() else {
            return Err(Es8311Error::MissingRegister);
        };
        if read_len != 0 && write.len() != 1 {
            return Err(Es8311Error::ReadWriteOverlap);
        }
        let span = if read_len != 0 {
            read_len
        } else {
            write.len() - 1
        };
        if usize::from(register).saturating_add(span) > 256 {
            return Err(Es8311Error::RegisterRange {
                register,
                length: span,
            });
        }
        self.transactions = self.transactions.saturating_add(1);
        if read_len != 0 {
            return Ok(
                self.registers[usize::from(register)..usize::from(register) + read_len].to_vec(),
            );
        }
        for (offset, value) in write[1..].iter().copied().enumerate() {
            let address = usize::from(register) + offset;
            if address == 0 && value == 0x80 {
                self.registers.fill(0);
            }
            self.registers[address] = value;
        }
        Ok(Vec::new())
    }

    /// Captures stable codec control state.
    pub fn snapshot(&self) -> Es8311Snapshot {
        Es8311Snapshot {
            powered: self.registers[0x00] & 0x80 != 0,
            adc_enabled: self.registers[0x0e] & 0x02 != 0,
            dac_enabled: self.registers[0x12] == 0 && self.registers[0x13] & 0x10 != 0,
            adc_volume: self.registers[0x17],
            dac_volume: self.registers[0x32],
            transactions: self.transactions,
        }
    }
}

impl Default for Es8311 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_official_microphone_and_speaker_sequences() {
        let mut codec = Es8311::new();
        for write in [[0x00, 0x80], [0x0d, 0x01], [0x0e, 0x02], [0x17, 0xff]] {
            codec.transact(&write, 0, SimTime::ZERO).unwrap();
        }
        assert!(codec.snapshot().adc_enabled);
        for write in [[0x12, 0x00], [0x13, 0x10], [0x32, 0xbf]] {
            codec.transact(&write, 0, SimTime::ZERO).unwrap();
        }
        let state = codec.snapshot();
        assert!(state.powered && state.dac_enabled);
        assert_eq!(state.dac_volume, 0xbf);
    }
}
