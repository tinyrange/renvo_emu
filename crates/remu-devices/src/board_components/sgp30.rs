use remu_core::SimTime;
use serde::Serialize;
use thiserror::Error;

/// Fixed SGP30 I2C address.
pub const SGP30_ADDRESS: u8 = 0x58;

/// SGP30 protocol-model error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum Sgp30Error {
    /// A command was not recognized.
    #[error("unsupported SGP30 command {0:#06x}")]
    UnsupportedCommand(u16),
    /// A command had the wrong number of parameter bytes.
    #[error("SGP30 command {command:#06x} expected {expected} parameter bytes, received {actual}")]
    ParameterLength {
        /// Command word.
        command: u16,
        /// Expected byte count after the command.
        expected: usize,
        /// Actual byte count.
        actual: usize,
    },
    /// A parameter CRC was invalid.
    #[error("SGP30 parameter CRC mismatch: expected {expected:#04x}, received {actual:#04x}")]
    Crc {
        /// Calculated CRC.
        expected: u8,
        /// Supplied CRC.
        actual: u8,
    },
    /// Air-quality measurement was requested before initialization.
    #[error("SGP30 air-quality measurement requested before init_air_quality")]
    NotInitialized,
    /// Read length did not match the command response.
    #[error("SGP30 response is {expected} bytes, but the controller requested {actual}")]
    ReadLength {
        /// Available bytes.
        expected: usize,
        /// Requested bytes.
        actual: usize,
    },
}

/// Observable SGP30 state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Sgp30Snapshot {
    /// Whether IAQ measurement has been initialized.
    pub initialized: bool,
    /// Configured CO2-equivalent value in ppm.
    pub eco2: u16,
    /// Configured total VOC value in ppb.
    pub tvoc: u16,
    /// Current CO2-equivalent baseline word.
    pub eco2_baseline: u16,
    /// Current TVOC baseline word.
    pub tvoc_baseline: u16,
    /// Last absolute-humidity compensation word.
    pub absolute_humidity: u16,
    /// Number of accepted commands.
    pub commands: u64,
}

/// Command-level deterministic SGP30 I2C model.
#[derive(Clone, Debug)]
pub struct Sgp30 {
    initialized_at: Option<SimTime>,
    eco2: u16,
    tvoc: u16,
    eco2_baseline: u16,
    tvoc_baseline: u16,
    absolute_humidity: u16,
    serial: [u16; 3],
    commands: u64,
}

impl Sgp30 {
    /// The datasheet's initial IAQ stabilization interval in nanosecond ticks.
    pub const WARMUP_TICKS: u64 = 15_000_000_000;

    /// Creates a sensor with deterministic identity and environmental values.
    pub const fn new(eco2: u16, tvoc: u16) -> Self {
        Self {
            initialized_at: None,
            eco2,
            tvoc,
            eco2_baseline: 0x8973,
            tvoc_baseline: 0x8aae,
            absolute_humidity: 0,
            serial: [0x0123, 0x4567, 0x89ab],
            commands: 0,
        }
    }

    /// Updates the simulated air-quality inputs.
    pub const fn set_air_quality(&mut self, eco2: u16, tvoc: u16) {
        self.eco2 = eco2;
        self.tvoc = tvoc;
    }

    /// Executes one complete write/read command transaction.
    pub fn transact(
        &mut self,
        write: &[u8],
        read_len: usize,
        at: SimTime,
    ) -> Result<Vec<u8>, Sgp30Error> {
        if write.len() < 2 {
            return Err(Sgp30Error::ParameterLength {
                command: 0,
                expected: 2,
                actual: write.len(),
            });
        }
        let command = u16::from_be_bytes([write[0], write[1]]);
        let params = &write[2..];
        self.commands = self.commands.saturating_add(1);
        let words = match command {
            0x2003 => {
                expect_params(command, params, 0)?;
                self.initialized_at = Some(at);
                Vec::new()
            }
            0x2008 => {
                expect_params(command, params, 0)?;
                let Some(initialized_at) = self.initialized_at else {
                    return Err(Sgp30Error::NotInitialized);
                };
                if at.ticks().saturating_sub(initialized_at.ticks()) < Self::WARMUP_TICKS {
                    vec![400, 0]
                } else {
                    vec![self.eco2, self.tvoc]
                }
            }
            0x2015 => {
                expect_params(command, params, 0)?;
                vec![self.eco2_baseline, self.tvoc_baseline]
            }
            0x201e => {
                expect_params(command, params, 6)?;
                self.eco2_baseline = decode_word(&params[0..3])?;
                self.tvoc_baseline = decode_word(&params[3..6])?;
                Vec::new()
            }
            0x202f => {
                expect_params(command, params, 0)?;
                vec![0x0022]
            }
            0x2032 => {
                expect_params(command, params, 0)?;
                vec![0xd400]
            }
            0x2050 => {
                expect_params(command, params, 0)?;
                vec![self.tvoc.saturating_mul(4), self.eco2.saturating_mul(2)]
            }
            0x2061 => {
                expect_params(command, params, 3)?;
                self.absolute_humidity = decode_word(params)?;
                Vec::new()
            }
            0x3682 => {
                expect_params(command, params, 0)?;
                self.serial.to_vec()
            }
            _ => return Err(Sgp30Error::UnsupportedCommand(command)),
        };
        let response = encode_words(&words);
        if response.len() != read_len {
            return Err(Sgp30Error::ReadLength {
                expected: response.len(),
                actual: read_len,
            });
        }
        Ok(response)
    }

    /// Current observable state.
    pub fn snapshot(&self) -> Sgp30Snapshot {
        Sgp30Snapshot {
            initialized: self.initialized_at.is_some(),
            eco2: self.eco2,
            tvoc: self.tvoc,
            eco2_baseline: self.eco2_baseline,
            tvoc_baseline: self.tvoc_baseline,
            absolute_humidity: self.absolute_humidity,
            commands: self.commands,
        }
    }
}

/// Sensirion CRC-8, polynomial 0x31 and initial value 0xff.
pub fn sensirion_crc(bytes: &[u8]) -> u8 {
    let mut crc = 0xff_u8;
    for byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn expect_params(command: u16, params: &[u8], expected: usize) -> Result<(), Sgp30Error> {
    if params.len() == expected {
        Ok(())
    } else {
        Err(Sgp30Error::ParameterLength {
            command,
            expected,
            actual: params.len(),
        })
    }
}

fn decode_word(bytes: &[u8]) -> Result<u16, Sgp30Error> {
    let expected = sensirion_crc(&bytes[..2]);
    if bytes[2] != expected {
        return Err(Sgp30Error::Crc {
            expected,
            actual: bytes[2],
        });
    }
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn encode_words(words: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(words.len() * 3);
    for word in words {
        let encoded = word.to_be_bytes();
        bytes.extend_from_slice(&encoded);
        bytes.push(sensirion_crc(&encoded));
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implements_identity_warmup_measurement_and_crc() {
        let mut sensor = Sgp30::new(900, 77);
        assert_eq!(
            sensor.transact(&[0x36, 0x82], 9, SimTime::ZERO).unwrap(),
            encode_words(&[0x0123, 0x4567, 0x89ab])
        );
        sensor.transact(&[0x20, 0x03], 0, SimTime::ZERO).unwrap();
        assert_eq!(
            sensor
                .transact(&[0x20, 0x08], 6, SimTime::from_ticks(1_000))
                .unwrap(),
            encode_words(&[400, 0])
        );
        assert_eq!(
            sensor
                .transact(&[0x20, 0x08], 6, SimTime::from_ticks(Sgp30::WARMUP_TICKS))
                .unwrap(),
            encode_words(&[900, 77])
        );
    }

    #[test]
    fn rejects_bad_parameter_crc() {
        let mut sensor = Sgp30::new(400, 0);
        let error = sensor
            .transact(&[0x20, 0x61, 0x12, 0x34, 0], 0, SimTime::ZERO)
            .unwrap_err();
        assert!(matches!(error, Sgp30Error::Crc { .. }));
    }
}
