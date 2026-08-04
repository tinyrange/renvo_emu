use super::*;

const IO_MUX_PIN_COUNT: usize = 49;
const IO_MUX_PIN_MASK: u32 = 0x0000_ffff;
const IO_MUX_PIN_CTRL_MASK: u32 = 0x0000_ffff;
const IO_MUX_DATE_RESET: u32 = 0x0190_7160;

/// Native ESP32-S3 IO MUX register identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32S3IoMuxRegister {
    /// Clock-output and pad-power control.
    PinCtrl,
    /// Per-pad configuration for a bonded GPIO number.
    Gpio(u8),
    /// Hardware version/date word.
    Date,
}

impl Esp32S3IoMuxRegister {
    /// Returns the native byte offset within the IO MUX page.
    pub const fn offset(self) -> u64 {
        match self {
            Self::PinCtrl => 0,
            Self::Gpio(pin) => 4 + pin as u64 * 4,
            Self::Date => 0xfc,
        }
    }

    /// Resolves an aligned native offset. GPIO22..GPIO25 and reserved holes fail.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        if offset & 3 != 0 {
            return None;
        }
        match offset {
            0 => Some(Self::PinCtrl),
            0xfc => Some(Self::Date),
            0x04..=0xc4 => {
                let pin = ((offset - 4) / 4) as u8;
                if pin <= 21 || (pin >= 26 && pin <= 48) {
                    Some(Self::Gpio(pin))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Bits returned by native reads.
    pub const fn read_mask(self) -> u32 {
        match self {
            Self::PinCtrl => IO_MUX_PIN_CTRL_MASK,
            Self::Gpio(_) => IO_MUX_PIN_MASK,
            Self::Date => u32::MAX,
        }
    }

    /// Bits accepted by native writes.
    pub const fn write_mask(self) -> u32 {
        self.read_mask()
    }
}

/// Decoded host-side view of one ESP32-S3 pad configuration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Esp32S3IoMuxPinConfig {
    /// Output enable while the pad is in sleep configuration.
    pub sleep_output_enable: bool,
    /// Selects the sleep configuration and wakeup behavior.
    pub sleep_select: bool,
    /// Pulldown enable in sleep configuration.
    pub sleep_pulldown: bool,
    /// Pullup enable in sleep configuration.
    pub sleep_pullup: bool,
    /// Input enable in sleep configuration.
    pub sleep_input_enable: bool,
    /// Two-bit sleep drive strength.
    pub sleep_drive: u8,
    /// Functional pulldown enable.
    pub pulldown: bool,
    /// Functional pullup enable.
    pub pullup: bool,
    /// Functional input enable.
    pub input_enable: bool,
    /// Two-bit functional drive strength.
    pub drive: u8,
    /// Three-bit direct IO MUX function selector.
    pub function: u8,
    /// Rejects input pulses shorter than two IO MUX clock cycles.
    pub filter_enable: bool,
}

impl Esp32S3IoMuxPinConfig {
    fn decode(value: u32) -> Self {
        Self {
            sleep_output_enable: value & (1 << 0) != 0,
            sleep_select: value & (1 << 1) != 0,
            sleep_pulldown: value & (1 << 2) != 0,
            sleep_pullup: value & (1 << 3) != 0,
            sleep_input_enable: value & (1 << 4) != 0,
            sleep_drive: ((value >> 5) & 0x3) as u8,
            pulldown: value & (1 << 7) != 0,
            pullup: value & (1 << 8) != 0,
            input_enable: value & (1 << 9) != 0,
            drive: ((value >> 10) & 0x3) as u8,
            function: ((value >> 12) & 0x7) as u8,
            filter_enable: value & (1 << 15) != 0,
        }
    }
}

#[derive(Clone)]
/// Host-side inspection handle for the ESP32-S3 IO MUX.
pub struct Esp32S3IoMuxHandle {
    state: Rc<RefCell<Esp32S3IoMuxState>>,
}

impl Esp32S3IoMuxHandle {
    /// Returns the decoded configuration for a bonded pad.
    pub fn pin_config(&self, pin: u8) -> Option<Esp32S3IoMuxPinConfig> {
        if pin > 48 || (22..=25).contains(&pin) {
            return None;
        }
        Some(Esp32S3IoMuxPinConfig::decode(
            self.state.borrow().pins[usize::from(pin)],
        ))
    }

    /// Returns the clock-output and pad-power control word.
    pub fn pin_ctrl(&self) -> u32 {
        self.state.borrow().pin_ctrl
    }
}

struct Esp32S3IoMuxState {
    pin_ctrl: u32,
    pins: [u32; IO_MUX_PIN_COUNT],
    date: u32,
}

impl Esp32S3IoMuxState {
    fn new() -> Self {
        Self {
            pin_ctrl: 0,
            pins: [0; IO_MUX_PIN_COUNT],
            date: IO_MUX_DATE_RESET,
        }
    }
}

/// Functional ESP32-S3 IO MUX register block.
pub struct Esp32S3IoMux {
    name: String,
    state: Rc<RefCell<Esp32S3IoMuxState>>,
}

impl Esp32S3IoMux {
    /// Creates a reset IO MUX and its host inspection handle.
    pub fn new(name: impl Into<String>) -> (Self, Esp32S3IoMuxHandle) {
        let state = Rc::new(RefCell::new(Esp32S3IoMuxState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3IoMuxHandle { state },
        )
    }

    fn register_value(state: &Esp32S3IoMuxState, register: Esp32S3IoMuxRegister) -> u32 {
        match register {
            Esp32S3IoMuxRegister::PinCtrl => state.pin_ctrl,
            Esp32S3IoMuxRegister::Gpio(pin) => state.pins[usize::from(pin)],
            Esp32S3IoMuxRegister::Date => state.date,
        }
    }
}

impl Device for Esp32S3IoMux {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 IO MUX requires aligned word access",
            ));
        }
        let register = Esp32S3IoMuxRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!("{} read at reserved offset {offset:#x}", self.name))
        })?;
        Ok(u64::from(
            Self::register_value(&self.state.borrow(), register) & register.read_mask(),
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
            return Err(DeviceError::new(
                "ESP32-S3 IO MUX requires aligned word access",
            ));
        }
        let register = Esp32S3IoMuxRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write at reserved offset {offset:#x}",
                self.name
            ))
        })?;
        let value = u32::try_from(value).map_err(|_| {
            DeviceError::new(format!(
                "{} word write exceeds 32 bits: {value:#x}",
                self.name
            ))
        })? & register.write_mask();
        let mut state = self.state.borrow_mut();
        match register {
            Esp32S3IoMuxRegister::PinCtrl => state.pin_ctrl = value,
            Esp32S3IoMuxRegister::Gpio(pin) => state.pins[usize::from(pin)] = value,
            Esp32S3IoMuxRegister::Date => state.date = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = Esp32S3IoMuxState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_register_ids_cover_bonded_pads_and_reject_holes() {
        for pin in (0..=21).chain(26..=48) {
            let register = Esp32S3IoMuxRegister::Gpio(pin);
            assert_eq!(
                Esp32S3IoMuxRegister::from_offset(register.offset()),
                Some(register)
            );
        }
        for pin in 22_u8..=25 {
            assert_eq!(
                Esp32S3IoMuxRegister::from_offset(4 + u64::from(pin) * 4),
                None
            );
        }
        assert_eq!(Esp32S3IoMuxRegister::from_offset(0xc8), None);
        assert_eq!(Esp32S3IoMuxRegister::from_offset(0xfd), None);
    }

    #[test]
    fn masks_writes_and_decodes_the_official_pad_fields() {
        let (mut device, handle) = Esp32S3IoMux::new("io-mux");
        let value: u32 =
            1 | (1 << 1) | (2 << 5) | (1 << 8) | (1 << 9) | (3 << 10) | (4 << 12) | (1 << 15);
        device
            .write(0x0c, AccessWidth::Word, u64::from(value), SimTime::ZERO)
            .unwrap();
        let config = handle.pin_config(2).unwrap();
        assert!(config.sleep_output_enable);
        assert!(config.sleep_select);
        assert_eq!(config.sleep_drive, 2);
        assert!(config.pullup);
        assert!(config.input_enable);
        assert_eq!(config.drive, 3);
        assert_eq!(config.function, 4);
        assert!(config.filter_enable);
        assert_eq!(handle.pin_config(22), None);

        device
            .write(0, AccessWidth::Word, u64::from(u32::MAX), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.pin_ctrl(), IO_MUX_PIN_CTRL_MASK);
    }

    #[test]
    fn reset_restores_pad_defaults_and_official_date() {
        let (mut device, handle) = Esp32S3IoMux::new("io-mux");
        device
            .write(0xb0, AccessWidth::Word, 0xffff, SimTime::ZERO)
            .unwrap();
        device.reset(ResetKind::Software);
        assert_eq!(
            handle.pin_config(43),
            Some(Esp32S3IoMuxPinConfig::default())
        );
        assert_eq!(
            device.read(0xfc, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(IO_MUX_DATE_RESET)
        );
    }

    #[test]
    fn rejects_reserved_and_non_word_accesses() {
        let (mut device, _) = Esp32S3IoMux::new("io-mux");
        assert!(device.read(0x5c, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(
            device
                .write(0x04, AccessWidth::HalfWord, 0, SimTime::ZERO)
                .is_err()
        );
        assert!(
            device
                .write(0x04, AccessWidth::Word, 1_u64 << 32, SimTime::ZERO)
                .is_err()
        );
    }
}
