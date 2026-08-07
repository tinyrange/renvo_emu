use remu_core::SimTime;
use serde::Serialize;
use thiserror::Error;

/// BMI270 I2C address selected by the `M5StickS3` board strap.
pub const BMI270_ADDRESS: u8 = 0x68;

const CHIP_ID: u8 = 0x00;
const STATUS: u8 = 0x03;
const ACC_X: u8 = 0x0c;
const GYR_X: u8 = 0x12;
const SENSOR_TIME: u8 = 0x18;
const INT_STATUS_1: u8 = 0x1d;
const INTERNAL_STATUS: u8 = 0x21;
const TEMPERATURE: u8 = 0x22;
const INIT_CTRL: u8 = 0x59;
const INIT_DATA: u8 = 0x5e;
const PWR_CONF: u8 = 0x7c;
const PWR_CTRL: u8 = 0x7d;
const COMMAND: u8 = 0x7e;

/// BMI270 register-transaction error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum Bmi270Error {
    /// No register pointer was supplied.
    #[error("BMI270 transaction must include a register pointer")]
    MissingRegister,
    /// A combined read also supplied write payload bytes.
    #[error("BMI270 read transaction may contain only its register pointer")]
    ReadWriteOverlap,
    /// A transaction crossed the byte register space.
    #[error("BMI270 register span {register:#04x}+{length} exceeds the register space")]
    RegisterRange {
        /// First requested register.
        register: u8,
        /// Requested transfer length.
        length: usize,
    },
}

/// Stable, host-visible BMI270 state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct Bmi270Snapshot {
    /// Whether the configuration upload was committed.
    pub initialized: bool,
    /// Whether accelerometer sampling is enabled.
    pub accelerometer_enabled: bool,
    /// Whether gyroscope sampling is enabled.
    pub gyroscope_enabled: bool,
    /// Raw XYZ acceleration sample.
    pub accel: [i16; 3],
    /// Raw XYZ angular-rate sample.
    pub gyro: [i16; 3],
    /// Raw temperature ADC sample.
    pub temperature: i16,
    /// Whether a sensor data-ready event is latched.
    pub data_ready: bool,
    /// Accepted I2C transaction count.
    pub transactions: u64,
    /// Accepted soft-reset count.
    pub resets: u64,
    /// Number of uploaded configuration bytes.
    pub config_bytes: usize,
}

/// Functional BMI270 surface used by `M5Unified` and direct ESP-IDF firmware.
#[derive(Clone, Debug)]
pub struct Bmi270 {
    registers: [u8; 256],
    pointer: u8,
    initialized: bool,
    accel: [i16; 3],
    gyro: [i16; 3],
    temperature: i16,
    transactions: u64,
    resets: u64,
    config_bytes: usize,
}

impl Bmi270 {
    /// Creates a sensor with stable power-on identity and one-g acceleration.
    pub fn new() -> Self {
        let mut model = Self {
            registers: [0; 256],
            pointer: 0,
            initialized: false,
            accel: [0, 0, 16_384],
            gyro: [0; 3],
            temperature: 0,
            transactions: 0,
            resets: 0,
            config_bytes: 0,
        };
        model.reset_registers();
        model
    }

    /// Executes a register-pointer I2C transaction.
    pub fn transact(
        &mut self,
        write: &[u8],
        read_len: usize,
        at: SimTime,
    ) -> Result<Vec<u8>, Bmi270Error> {
        let Some(&register) = write.first() else {
            return Err(Bmi270Error::MissingRegister);
        };
        if read_len != 0 && write.len() != 1 {
            return Err(Bmi270Error::ReadWriteOverlap);
        }
        let span = if read_len != 0 {
            read_len
        } else {
            write.len() - 1
        };
        if usize::from(register).saturating_add(span) > 256 {
            return Err(Bmi270Error::RegisterRange {
                register,
                length: span,
            });
        }
        self.transactions = self.transactions.saturating_add(1);
        self.pointer = register;
        self.refresh_samples(at);
        if read_len != 0 {
            let result = (0..read_len)
                .map(|offset| self.registers[usize::from(register) + offset])
                .collect();
            self.registers[usize::from(INT_STATUS_1)] = 0;
            return Ok(result);
        }
        if register == INIT_DATA {
            self.config_bytes = self
                .config_bytes
                .saturating_add(write.len().saturating_sub(1));
            if let Some(value) = write.last() {
                self.registers[usize::from(INIT_DATA)] = *value;
            }
            return Ok(Vec::new());
        }
        for (offset, value) in write[1..].iter().copied().enumerate() {
            self.write_register(
                register.wrapping_add(u8::try_from(offset).expect("BMI270 span fits u8")),
                value,
            );
        }
        Ok(Vec::new())
    }

    /// Installs a deterministic physical sample and raises data-ready.
    pub fn set_sample(&mut self, accel: [i16; 3], gyro: [i16; 3], temperature: i16) {
        self.accel = accel;
        self.gyro = gyro;
        self.temperature = temperature;
        self.store_samples();
    }

    /// Returns whether either accel or gyro data-ready is active.
    pub fn interrupt_asserted(&self) -> bool {
        self.registers[usize::from(INT_STATUS_1)] & 0xc0 != 0
    }

    /// Captures stable qualification state.
    pub fn snapshot(&self) -> Bmi270Snapshot {
        Bmi270Snapshot {
            initialized: self.initialized,
            accelerometer_enabled: self.registers[usize::from(PWR_CTRL)] & 0x04 != 0,
            gyroscope_enabled: self.registers[usize::from(PWR_CTRL)] & 0x02 != 0,
            accel: self.accel,
            gyro: self.gyro,
            temperature: self.temperature,
            data_ready: self.interrupt_asserted(),
            transactions: self.transactions,
            resets: self.resets,
            config_bytes: self.config_bytes,
        }
    }

    fn reset_registers(&mut self) {
        self.registers.fill(0);
        self.registers[usize::from(CHIP_ID)] = 0x24;
        self.registers[usize::from(STATUS)] = 0x10;
        self.registers[usize::from(PWR_CONF)] = 0x03;
        self.registers[usize::from(INTERNAL_STATUS)] = 0x01;
        self.store_samples();
    }

    fn store_samples(&mut self) {
        for (base, values) in [(ACC_X, self.accel), (GYR_X, self.gyro)] {
            for (axis, value) in values.into_iter().enumerate() {
                let [low, high] = value.to_le_bytes();
                self.registers[usize::from(base) + axis * 2] = low;
                self.registers[usize::from(base) + axis * 2 + 1] = high;
            }
        }
        let [low, high] = self.temperature.to_le_bytes();
        self.registers[usize::from(TEMPERATURE)] = low;
        self.registers[usize::from(TEMPERATURE) + 1] = high;
        self.registers[usize::from(INT_STATUS_1)] |= 0xc0;
    }

    fn refresh_samples(&mut self, at: SimTime) {
        let ticks = at.ticks();
        let bytes = ticks.to_le_bytes();
        self.registers[usize::from(SENSOR_TIME)..usize::from(SENSOR_TIME) + 3]
            .copy_from_slice(&bytes[..3]);
        self.store_samples();
    }

    fn write_register(&mut self, register: u8, value: u8) {
        match register {
            CHIP_ID | STATUS | INT_STATUS_1 | INTERNAL_STATUS | TEMPERATURE..=0x23 => {}
            COMMAND if value == 0xb6 => {
                self.resets = self.resets.saturating_add(1);
                self.initialized = false;
                self.config_bytes = 0;
                self.reset_registers();
            }
            COMMAND if value == 0xb0 => {}
            INIT_CTRL => {
                self.registers[usize::from(register)] = value;
                if value & 1 != 0 {
                    self.initialized = true;
                    self.registers[usize::from(INTERNAL_STATUS)] = 1;
                }
            }
            INIT_DATA => {
                self.registers[usize::from(register)] = value;
            }
            _ => self.registers[usize::from(register)] = value,
        }
    }
}

impl Default for Bmi270 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_m5unified_probe_initialization_and_samples() {
        let mut imu = Bmi270::new();
        assert_eq!(imu.transact(&[CHIP_ID], 1, SimTime::ZERO).unwrap(), [0x24]);
        imu.transact(&[COMMAND, 0xb6], 0, SimTime::ZERO).unwrap();
        imu.transact(&[INIT_DATA, 1, 2, 3], 0, SimTime::ZERO)
            .unwrap();
        imu.transact(&[INIT_CTRL, 1], 0, SimTime::ZERO).unwrap();
        imu.transact(&[PWR_CTRL, 0x06], 0, SimTime::ZERO).unwrap();
        imu.set_sample([100, -200, 16_000], [5, 6, 7], 512);
        assert_eq!(
            imu.transact(&[ACC_X], 2, SimTime::ZERO).unwrap(),
            100_i16.to_le_bytes()
        );
        let state = imu.snapshot();
        assert!(state.initialized && state.accelerometer_enabled && state.gyroscope_enabled);
        assert_eq!(state.config_bytes, 3);
    }
}
