use remu_core::SimTime;
use serde::Serialize;
use thiserror::Error;

/// M5Stack M5PM1 default seven-bit I²C address.
pub const M5PM1_ADDRESS: u8 = 0x6e;

const REGISTER_COUNT: usize = 256;
const NANOSECONDS_PER_SECOND: u64 = 1_000_000_000;

// System registers.
const REG_DEVICE_ID: u8 = 0x00;
const REG_DEVICE_MODEL: u8 = 0x01;
const REG_HW_REVISION: u8 = 0x02;
const REG_SW_REVISION: u8 = 0x03;
const REG_WAKE_SOURCE: u8 = 0x05;
const REG_POWER_CONFIG: u8 = 0x06;
const REG_HOLD_CONFIG: u8 = 0x07;
const REG_WATCHDOG_COUNT: u8 = 0x0a;
const REG_WATCHDOG_KEY: u8 = 0x0b;
const REG_SYSTEM_COMMAND: u8 = 0x0c;

// GPIO registers.
const REG_GPIO_MODE: u8 = 0x10;
const REG_GPIO_OUTPUT: u8 = 0x11;
const REG_GPIO_INPUT: u8 = 0x12;
const REG_GPIO_FUNCTION_0: u8 = 0x16;
const REG_GPIO_FUNCTION_1: u8 = 0x17;
const REG_GPIO_WAKE_EDGE: u8 = 0x19;

// ADC registers.
const REG_ADC_RESULT_LOW: u8 = 0x28;
const REG_ADC_RESULT_HIGH: u8 = 0x29;
const REG_ADC_CONTROL: u8 = 0x2a;

// Timer registers.
const REG_TIMER_COUNT_0: u8 = 0x38;
const REG_TIMER_COUNT_3: u8 = 0x3b;
const REG_TIMER_CONFIG: u8 = 0x3c;
const REG_TIMER_KEY: u8 = 0x3d;

// Interrupt and button registers.
const REG_IRQ_GPIO: u8 = 0x40;
const REG_IRQ_BUTTON: u8 = 0x42;
const REG_BUTTON_STATUS: u8 = 0x48;

// NeoPixel and retained RAM windows.
const REG_NEO_CONFIG: u8 = 0x50;
const REG_NEO_DATA_START: u8 = 0x60;
const REG_NEO_DATA_END: u8 = 0x9f;
const REG_RTC_RAM_START: u8 = 0xa0;
const REG_RTC_RAM_END: u8 = 0xbf;

/// M5PM1 transaction or register-model error.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum M5Pm1Error {
    /// A transaction did not include a register pointer.
    #[error("M5PM1 transaction must include a register pointer")]
    MissingRegister,
    /// A read transaction included register data as well as its pointer.
    #[error("M5PM1 read transaction may contain only its register pointer")]
    ReadWriteOverlap,
    /// The requested register span crosses the 8-bit register space.
    #[error("M5PM1 register span {register:#04x}+{length} exceeds the register space")]
    RegisterRange {
        /// First register in the transfer.
        register: u8,
        /// Number of bytes requested.
        length: usize,
    },
    /// A GPIO number is outside GPIO0..GPIO4.
    #[error("M5PM1 GPIO {0} is outside GPIO0..GPIO4")]
    Gpio(u8),
}

/// Observable M5PM1 companion-MCU state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct M5Pm1Snapshot {
    /// Whether a shutdown/download command has put the companion to sleep.
    pub sleeping: bool,
    /// Number of accepted system reset commands.
    pub reset_count: u64,
    /// Number of I²C transactions accepted by the model.
    pub transactions: u64,
    /// Power rail enable bits: charge, DCDC, LDO and BOOST/Grove.
    pub power_config: u8,
    /// Power-hold bits for rails and GPIO outputs.
    pub hold_config: u8,
    /// GPIO direction bitmap, one bit per GPIO.
    pub gpio_mode: u8,
    /// GPIO output latch bitmap, one bit per GPIO.
    pub gpio_output: u8,
    /// GPIO input bitmap, one bit per GPIO.
    pub gpio_input: u8,
    /// GPIO function fields, encoded as two-bit values.
    pub gpio_function_0: u8,
    /// GPIO4 function field.
    pub gpio_function_1: u8,
    /// ADC result in the device's millivolt/count register units.
    pub adc_result: u16,
    /// Current button level.
    pub button_pressed: bool,
    /// Whether a button event has occurred since the status was read.
    pub button_flag: bool,
    /// Configured NeoPixel count.
    pub neopixel_count: u8,
    /// Number of retained RTC-RAM bytes that have been written.
    pub rtc_bytes_written: usize,
}

/// Functional M5Stack M5PM1 power-management companion MCU.
///
/// This model intentionally implements the register-level surface used by
/// the official M5PM1 driver: identity, power rails, GPIO/IRQ configuration,
/// ADC conversion results, timer control, button status, NeoPixel data and
/// retained RTC RAM. It is deterministic and does not claim analogue or
/// power-integrity fidelity.
#[derive(Clone, Debug)]
pub struct M5Pm1 {
    registers: [u8; REGISTER_COUNT],
    sleeping: bool,
    reset_count: u64,
    transactions: u64,
    timer_started_at: Option<SimTime>,
    timer_seconds: u32,
}

impl M5Pm1 {
    /// Creates an M5PM1 with the M5StickS3 power-on defaults.
    pub fn new() -> Self {
        let mut registers = [0_u8; REGISTER_COUNT];
        // The public driver only requires stable identity values. Firmware
        // revisions are deliberately synthetic because the PM1 firmware is
        // not distributed as part of the board documentation.
        registers[usize::from(REG_DEVICE_ID)] = 0x01;
        registers[usize::from(REG_DEVICE_MODEL)] = 0x01;
        registers[usize::from(REG_HW_REVISION)] = 0x01;
        registers[usize::from(REG_SW_REVISION)] = b'A';
        // CHG, DCDC and LDO are enabled at power-on; Grove/BOOST is off on
        // the StickS3 until the host explicitly enables external power.
        registers[usize::from(REG_POWER_CONFIG)] = 0x07;
        registers[usize::from(REG_GPIO_INPUT)] = 0x1f;
        registers[usize::from(REG_ADC_RESULT_LOW)] = 0x00;
        registers[usize::from(REG_ADC_RESULT_HIGH)] = 0x04;
        Self {
            registers,
            sleeping: false,
            reset_count: 0,
            transactions: 0,
            timer_started_at: None,
            timer_seconds: 0,
        }
    }

    /// Executes a register-pointer I²C transaction.
    ///
    /// A write is encoded as `[register, data...]`. A read uses
    /// `[register]` plus `read_len > 0`, matching the repeated-start access
    /// pattern used by `M5PM1.cpp`. Register reads are little-endian only
    /// where the PM1 register map defines adjacent low/high bytes; the model
    /// otherwise returns bytes in address order.
    pub fn transact(
        &mut self,
        write: &[u8],
        read_len: usize,
        at: SimTime,
    ) -> Result<Vec<u8>, M5Pm1Error> {
        let Some(&register) = write.first() else {
            return Err(M5Pm1Error::MissingRegister);
        };
        if read_len > 0 && write.len() != 1 {
            return Err(M5Pm1Error::ReadWriteOverlap);
        }
        let span = if read_len > 0 {
            read_len
        } else {
            write.len().saturating_sub(1)
        };
        Self::check_range(register, span)?;
        self.advance(at);
        self.transactions = self.transactions.saturating_add(1);
        if read_len > 0 {
            return Ok((0..read_len)
                .map(|offset| self.read_register(register.wrapping_add(offset as u8)))
                .collect());
        }
        for (offset, value) in write[1..].iter().copied().enumerate() {
            self.write_register(register.wrapping_add(offset as u8), value, at);
        }
        Ok(Vec::new())
    }

    /// Reads one register without an I²C transaction wrapper.
    pub fn register(&self, register: u8) -> u8 {
        self.registers[usize::from(register)]
    }

    /// Supplies an external digital level to one M5PM1 GPIO.
    pub fn set_gpio_input(&mut self, pin: u8, high: bool) -> Result<(), M5Pm1Error> {
        Self::check_gpio(pin)?;
        let bit = 1_u8 << pin;
        let old = self.registers[usize::from(REG_GPIO_INPUT)] & bit != 0;
        if high {
            self.registers[usize::from(REG_GPIO_INPUT)] |= bit;
        } else {
            self.registers[usize::from(REG_GPIO_INPUT)] &= !bit;
        }
        if old != high && self.irq_function(pin) {
            let rising = !old && high;
            let rising_edge = self.registers[usize::from(REG_GPIO_WAKE_EDGE)] & bit != 0;
            if rising == rising_edge {
                self.registers[usize::from(REG_IRQ_GPIO)] |= bit;
            }
        }
        Ok(())
    }

    /// Sets the companion's power-button level and latches a button event.
    pub fn set_button_pressed(&mut self, pressed: bool) {
        let status = &mut self.registers[usize::from(REG_BUTTON_STATUS)];
        if (*status & 1 != 0) != pressed {
            *status = (*status & !1) | u8::from(pressed) | 0x80;
            self.registers[usize::from(REG_IRQ_BUTTON)] |= 1;
        }
    }

    /// Returns whether the M5StickS3 Grove/BOOST rail is enabled.
    pub const fn grove_powered(&self) -> bool {
        self.registers[REG_POWER_CONFIG as usize] & 0x08 != 0
    }

    /// Returns the most recent NeoPixel RGB565 data bytes.
    pub fn neopixel_data(&self) -> &[u8] {
        &self.registers[REG_NEO_DATA_START as usize..=REG_NEO_DATA_END as usize]
    }

    /// Returns a stable, serializable state snapshot.
    pub fn snapshot(&self) -> M5Pm1Snapshot {
        let rtc_bytes_written = self.registers
            [REG_RTC_RAM_START as usize..=REG_RTC_RAM_END as usize]
            .iter()
            .filter(|byte| **byte != 0)
            .count();
        M5Pm1Snapshot {
            sleeping: self.sleeping,
            reset_count: self.reset_count,
            transactions: self.transactions,
            power_config: self.register(REG_POWER_CONFIG),
            hold_config: self.register(REG_HOLD_CONFIG),
            gpio_mode: self.register(REG_GPIO_MODE),
            gpio_output: self.register(REG_GPIO_OUTPUT),
            gpio_input: self.register(REG_GPIO_INPUT),
            gpio_function_0: self.register(REG_GPIO_FUNCTION_0),
            gpio_function_1: self.register(REG_GPIO_FUNCTION_1),
            adc_result: u16::from_le_bytes([
                self.register(REG_ADC_RESULT_LOW),
                self.register(REG_ADC_RESULT_HIGH),
            ]),
            button_pressed: self.register(REG_BUTTON_STATUS) & 1 != 0,
            button_flag: self.register(REG_BUTTON_STATUS) & 0x80 != 0,
            neopixel_count: self.register(REG_NEO_CONFIG) & 0x3f,
            rtc_bytes_written,
        }
    }

    fn check_range(register: u8, length: usize) -> Result<(), M5Pm1Error> {
        if usize::from(register).saturating_add(length) > REGISTER_COUNT {
            return Err(M5Pm1Error::RegisterRange { register, length });
        }
        Ok(())
    }

    fn check_gpio(pin: u8) -> Result<(), M5Pm1Error> {
        if pin < 5 {
            Ok(())
        } else {
            Err(M5Pm1Error::Gpio(pin))
        }
    }

    fn irq_function(&self, pin: u8) -> bool {
        let function = if pin < 4 {
            (self.register(REG_GPIO_FUNCTION_0) >> (pin * 2)) & 0x03
        } else {
            self.register(REG_GPIO_FUNCTION_1) & 0x03
        };
        function == 1
    }

    fn advance(&mut self, at: SimTime) {
        let Some(started) = self.timer_started_at else {
            return;
        };
        let elapsed = at.ticks().saturating_sub(started.ticks());
        let remaining = self
            .timer_seconds
            .saturating_sub(u32::try_from(elapsed / NANOSECONDS_PER_SECOND).unwrap_or(u32::MAX));
        self.registers[usize::from(REG_TIMER_COUNT_0)..=usize::from(REG_TIMER_COUNT_3)]
            .copy_from_slice(&remaining.to_le_bytes());
        if remaining != 0 {
            return;
        }
        let action = self.register(REG_TIMER_CONFIG) & 0x07;
        self.timer_started_at = None;
        match action {
            0b001 => self.registers[usize::from(REG_WAKE_SOURCE)] |= 1,
            0b010 => self.reset_runtime(),
            0b011 => self.sleeping = false,
            0b100 => self.sleeping = true,
            _ => {}
        }
    }

    fn read_register(&mut self, register: u8) -> u8 {
        if register == REG_BUTTON_STATUS {
            let value = self.registers[usize::from(register)];
            self.registers[usize::from(register)] &= !0x80;
            return value;
        }
        self.register(register)
    }

    fn write_register(&mut self, register: u8, value: u8, at: SimTime) {
        let index = usize::from(register);
        match register {
            REG_DEVICE_ID | REG_DEVICE_MODEL | REG_HW_REVISION | REG_SW_REVISION
            | REG_GPIO_INPUT => {}
            REG_WAKE_SOURCE => self.registers[index] &= !value,
            REG_SYSTEM_COMMAND => {
                if value & 0xf0 == 0xa0 {
                    match value & 0x03 {
                        0x01 | 0x03 => self.sleeping = true,
                        0x02 => {
                            self.reset_count = self.reset_count.saturating_add(1);
                            self.reset_runtime();
                        }
                        _ => {}
                    }
                }
            }
            REG_WATCHDOG_KEY if value == 0xa5 => {
                self.registers[usize::from(REG_WATCHDOG_COUNT)] = 0;
            }
            REG_ADC_CONTROL => {
                let channel = (value >> 1) & 0x07;
                self.registers[index] = value & !1;
                if value & 1 != 0 {
                    let result = match channel {
                        1 => self.registers[usize::from(REG_GPIO_INPUT)] as u16 * 100,
                        2 => self.registers[usize::from(REG_GPIO_INPUT)] as u16 * 75,
                        6 => 250,
                        _ => 0,
                    };
                    let [low, high] = result.to_le_bytes();
                    self.registers[usize::from(REG_ADC_RESULT_LOW)] = low;
                    self.registers[usize::from(REG_ADC_RESULT_HIGH)] = high;
                }
            }
            REG_TIMER_COUNT_0..=REG_TIMER_COUNT_3 => {
                self.registers[index] = value;
            }
            REG_TIMER_CONFIG => {
                self.registers[index] = value & 0x0f;
                if value & 0x07 == 0 {
                    self.timer_started_at = None;
                }
            }
            REG_TIMER_KEY if value == 0xa5 => {
                self.timer_seconds = u32::from_le_bytes([
                    self.register(REG_TIMER_COUNT_0),
                    self.register(REG_TIMER_COUNT_0 + 1),
                    self.register(REG_TIMER_COUNT_0 + 2),
                    self.register(REG_TIMER_COUNT_0 + 3),
                ]);
                self.timer_started_at = (self.timer_seconds != 0).then_some(at);
            }
            REG_NEO_CONFIG => self.registers[index] = value & 0x7f,
            _ => self.registers[index] = value,
        }
    }

    fn reset_runtime(&mut self) {
        self.sleeping = false;
        self.timer_started_at = None;
        self.timer_seconds = 0;
        self.registers[usize::from(REG_POWER_CONFIG)] = 0x07;
        self.registers[usize::from(REG_HOLD_CONFIG)] = 0;
        self.registers[usize::from(REG_IRQ_GPIO)] = 0;
        self.registers[usize::from(REG_IRQ_BUTTON)] = 0;
        self.registers[usize::from(REG_BUTTON_STATUS)] &= 1;
        self.registers[usize::from(REG_TIMER_CONFIG)] = 0;
        self.registers[usize::from(REG_TIMER_COUNT_0)..=usize::from(REG_TIMER_COUNT_3)].fill(0);
    }
}

impl Default for M5Pm1 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_driver_identity_and_power_defaults_are_stable() {
        let mut pm1 = M5Pm1::new();
        assert_eq!(
            pm1.transact(&[REG_DEVICE_ID], 4, SimTime::ZERO).unwrap(),
            [0x01, 0x01, 0x01, b'A']
        );
        assert_eq!(pm1.register(REG_POWER_CONFIG), 0x07);
        assert!(!pm1.grove_powered());
    }

    #[test]
    fn gpio_configuration_and_external_irq_are_observable() {
        let mut pm1 = M5Pm1::new();
        pm1.transact(&[REG_GPIO_FUNCTION_0, 0x01], 0, SimTime::ZERO)
            .unwrap();
        pm1.transact(&[REG_GPIO_WAKE_EDGE, 0x01], 0, SimTime::ZERO)
            .unwrap();
        pm1.set_gpio_input(0, false).unwrap();
        pm1.set_gpio_input(0, true).unwrap();
        assert_eq!(pm1.register(REG_IRQ_GPIO), 0x01);
        pm1.transact(&[REG_POWER_CONFIG, 0x0f], 0, SimTime::ZERO)
            .unwrap();
        assert!(pm1.grove_powered());
    }

    #[test]
    fn adc_start_captures_a_deterministic_result_and_button_status_clears_flag() {
        let mut pm1 = M5Pm1::new();
        pm1.set_gpio_input(1, true).unwrap();
        pm1.transact(&[REG_ADC_CONTROL, 0x03], 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            pm1.transact(&[REG_ADC_RESULT_LOW], 2, SimTime::ZERO)
                .unwrap(),
            [0x1c, 0x0c]
        );
        pm1.set_button_pressed(true);
        assert_eq!(
            pm1.transact(&[REG_BUTTON_STATUS], 1, SimTime::ZERO)
                .unwrap(),
            [0x81]
        );
        assert_eq!(
            pm1.transact(&[REG_BUTTON_STATUS], 1, SimTime::ZERO)
                .unwrap(),
            [0x01]
        );
    }

    #[test]
    fn timer_flag_is_raised_after_abstract_seconds() {
        let mut pm1 = M5Pm1::new();
        pm1.transact(&[REG_TIMER_COUNT_0, 1, 0, 0, 0], 0, SimTime::ZERO)
            .unwrap();
        pm1.transact(&[REG_TIMER_CONFIG, 0x01], 0, SimTime::ZERO)
            .unwrap();
        pm1.transact(&[REG_TIMER_KEY, 0xa5], 0, SimTime::ZERO)
            .unwrap();
        pm1.transact(
            &[REG_WAKE_SOURCE],
            1,
            SimTime::from_ticks(NANOSECONDS_PER_SECOND),
        )
        .unwrap();
        assert_eq!(pm1.register(REG_WAKE_SOURCE) & 1, 1);
    }
}
