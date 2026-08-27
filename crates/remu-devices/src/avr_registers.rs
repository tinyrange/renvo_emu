/// Named ATmega328PB Timer/Counter3 and Timer/Counter4 register identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u16)]
pub enum AtmegaTimerRegister {
    /// Timer/Counter3 interrupt flags (TIFR3).
    Tifr3 = 0x38,
    /// Timer/Counter4 interrupt flags (TIFR4).
    Tifr4 = 0x39,
    /// Timer/Counter3 interrupt mask (TIMSK3).
    Timsk3 = 0x71,
    /// Timer/Counter4 interrupt mask (TIMSK4).
    Timsk4 = 0x72,
    /// Timer/Counter3 control register B (TCCR3B).
    Tccr3b = 0x91,
    /// Timer/Counter3 counter low byte (TCNT3L).
    Tcnt3l = 0x94,
    /// Timer/Counter3 counter high byte (TCNT3H).
    Tcnt3h = 0x95,
    /// Timer/Counter3 output compare A low byte (OCR3AL).
    Ocr3al = 0x98,
    /// Timer/Counter3 output compare A high byte (OCR3AH).
    Ocr3ah = 0x99,
    /// Timer/Counter4 control register B (TCCR4B).
    Tccr4b = 0xa1,
    /// Timer/Counter4 counter low byte (TCNT4L).
    Tcnt4l = 0xa4,
    /// Timer/Counter4 counter high byte (TCNT4H).
    Tcnt4h = 0xa5,
    /// Timer/Counter4 output compare A low byte (OCR4AL).
    Ocr4al = 0xa8,
    /// Timer/Counter4 output compare A high byte (OCR4AH).
    Ocr4ah = 0xa9,
}

impl AtmegaTimerRegister {
    /// Stable list of modeled Timer3/Timer4 register IDs.
    pub const ALL: [Self; 14] = [
        Self::Tifr3,
        Self::Tifr4,
        Self::Timsk3,
        Self::Timsk4,
        Self::Tccr3b,
        Self::Tcnt3l,
        Self::Tcnt3h,
        Self::Ocr3al,
        Self::Ocr3ah,
        Self::Tccr4b,
        Self::Tcnt4l,
        Self::Tcnt4h,
        Self::Ocr4al,
        Self::Ocr4ah,
    ];

    /// Returns the native data-space address.
    pub const fn offset(self) -> u16 {
        self as u16
    }

    /// Returns the I/O-device offset used by `AtmegaIo`.
    pub const fn io_offset(self) -> u16 {
        self.offset() - 0x20
    }

    /// Returns the register-array index used by `AtmegaIo`.
    pub const fn index(self) -> usize {
        self.io_offset() as usize
    }

    /// Returns the vendor register name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tifr3 => "tifr3",
            Self::Tifr4 => "tifr4",
            Self::Timsk3 => "timsk3",
            Self::Timsk4 => "timsk4",
            Self::Tccr3b => "tccr3b",
            Self::Tcnt3l => "tcnt3l",
            Self::Tcnt3h => "tcnt3h",
            Self::Ocr3al => "ocr3al",
            Self::Ocr3ah => "ocr3ah",
            Self::Tccr4b => "tccr4b",
            Self::Tcnt4l => "tcnt4l",
            Self::Tcnt4h => "tcnt4h",
            Self::Ocr4al => "ocr4al",
            Self::Ocr4ah => "ocr4ah",
        }
    }

    /// Resolves a native data-space address to a named Timer3/Timer4 register.
    pub const fn from_data_address(address: u16) -> Option<Self> {
        match address {
            0x38 => Some(Self::Tifr3),
            0x39 => Some(Self::Tifr4),
            0x71 => Some(Self::Timsk3),
            0x72 => Some(Self::Timsk4),
            0x91 => Some(Self::Tccr3b),
            0x94 => Some(Self::Tcnt3l),
            0x95 => Some(Self::Tcnt3h),
            0x98 => Some(Self::Ocr3al),
            0x99 => Some(Self::Ocr3ah),
            0xa1 => Some(Self::Tccr4b),
            0xa4 => Some(Self::Tcnt4l),
            0xa5 => Some(Self::Tcnt4h),
            0xa8 => Some(Self::Ocr4al),
            0xa9 => Some(Self::Ocr4ah),
            _ => None,
        }
    }
}

pub(super) const SMCR_WRITABLE_MASK: u8 = 0x0f;
pub(super) const CLKPR_CHANGE_ENABLE: u8 = 1 << 7;
pub(super) const CLKPR_DIVIDER_MASK: u8 = 0x0f;
pub(super) const PRR1_WRITABLE_MASK: u8 = 0x3d;
pub(super) const PRR0_PRTIM0: u8 = 1 << 5;
pub(super) const PRR0_PRTIM1: u8 = 1 << 3;
pub(super) const PRR0_PRUSART0: u8 = 1 << 1;
