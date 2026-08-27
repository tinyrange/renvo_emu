use super::*;

const REGISTER_COUNT: usize = 0xf0 / 4;
const TIMER_RUN: u32 = 1 << 1;
const TIMER_CLEAR: u32 = 1 << 2;
const TIMER_ALARM_ENABLE: u32 = 1 << 4;
const TIMER_PWRUP_ON_ALARM: u32 = 1 << 5;
const TIMER_ALARM: u32 = 1 << 6;
const TIMER_SOURCE_LPOSC: u32 = 1 << 8;
const TIMER_SOURCE_XOSC: u32 = 1 << 9;
const TIMER_SOURCE_GPIO_1KHZ: u32 = 1 << 10;
const TIMER_SOURCE_GPIO_1HZ: u32 = 1 << 13;
const TIMER_SOURCE_STATUS: u32 = 0x000f_0000;
const TIMER_CONTROL_MASK: u32 = TIMER_RUN | TIMER_ALARM_ENABLE | TIMER_PWRUP_ON_ALARM | 1;
const TIMER_SOURCE_SELECT: u32 =
    TIMER_SOURCE_LPOSC | TIMER_SOURCE_XOSC | TIMER_SOURCE_GPIO_1KHZ | TIMER_SOURCE_GPIO_1HZ;
const TIMER_COMMAND_MASK: u32 = TIMER_CLEAR | TIMER_ALARM | TIMER_SOURCE_SELECT;
const CHIP_RESET_RO: u32 = 0x1fef_0000;
const CHIP_RESET_RESCUE_FLAG: u32 = 1 << 4;
const CHIP_RESET_DOUBLE_TAP: u32 = 1;
const STATE_WC_MASK: u32 = 0x0000_0300;
const STATE_REQ_MASK: u32 = 0x0000_00f0;
const PWRUP_STATUS: u32 = 1 << 9;
const PWRUP_RAW_STATUS: u32 = 1 << 10;

/// Named RP2350 POWMAN register identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rp2350PowmanRegister {
    /// Bad-password write-clear status.
    BadPasswd,
    /// Voltage-regulator control.
    VregCtrl,
    /// Voltage-regulator status.
    VregSts,
    /// Voltage-regulator settings.
    Vreg,
    /// Low-power-entry voltage-regulator settings.
    VregLpEntry,
    /// Low-power-exit voltage-regulator settings.
    VregLpExit,
    /// Brown-out detector control.
    BodCtrl,
    /// Brown-out detector settings.
    Bod,
    /// Low-power-entry brown-out detector settings.
    BodLpEntry,
    /// Low-power-exit brown-out detector settings.
    BodLpExit,
    /// Low-power oscillator control.
    Lposc,
    /// Chip reset status and control.
    ChipReset,
    /// Watchdog reset selection.
    Wdsel,
    /// Power-sequencer configuration.
    SeqCfg,
    /// Power-domain state and request.
    State,
    /// Power-sequencer clock divider.
    PowFastdiv,
    /// Power-sequencer delay settings.
    PowDelay,
    /// External power-control output 0.
    ExtCtrl0,
    /// External power-control output 1.
    ExtCtrl1,
    /// External GPIO time-reference selection.
    ExtTimeRef,
    /// LPOSC integer frequency metadata.
    LposcFreqKhzInt,
    /// LPOSC fractional frequency metadata.
    LposcFreqKhzFrac,
    /// XOSC integer frequency metadata.
    XoscFreqKhzInt,
    /// XOSC fractional frequency metadata.
    XoscFreqKhzFrac,
    /// Set-time bits 63 through 48.
    SetTime63To48,
    /// Set-time bits 47 through 32.
    SetTime47To32,
    /// Set-time bits 31 through 16.
    SetTime31To16,
    /// Set-time bits 15 through 0.
    SetTime15To0,
    /// Read-only AON timer bits 63 through 32.
    ReadTimeUpper,
    /// Read-only AON timer bits 31 through 0.
    ReadTimeLower,
    /// Alarm-time bits 63 through 48.
    AlarmTime63To48,
    /// Alarm-time bits 47 through 32.
    AlarmTime47To32,
    /// Alarm-time bits 31 through 16.
    AlarmTime31To16,
    /// Alarm-time bits 15 through 0.
    AlarmTime15To0,
    /// AON timer control and status.
    Timer,
    /// GPIO power-up source 0.
    Pwrup0,
    /// GPIO power-up source 1.
    Pwrup1,
    /// GPIO power-up source 2.
    Pwrup2,
    /// GPIO power-up source 3.
    Pwrup3,
    /// Current power-up request status.
    CurrentPwrupReq,
    /// Last switched-core power-up source.
    LastSwcorePwrup,
    /// Debugger power-request configuration.
    DbgPwrcfg,
    /// Boot-disabling flags.
    Bootdis,
    /// Debug-port instance configuration.
    Dbgconfig,
    /// General-purpose scratch register 0.
    Scratch0,
    /// General-purpose scratch register 1.
    Scratch1,
    /// General-purpose scratch register 2.
    Scratch2,
    /// General-purpose scratch register 3.
    Scratch3,
    /// General-purpose scratch register 4.
    Scratch4,
    /// General-purpose scratch register 5.
    Scratch5,
    /// General-purpose scratch register 6.
    Scratch6,
    /// General-purpose scratch register 7.
    Scratch7,
    /// Boot scratch register 0.
    Boot0,
    /// Boot scratch register 1.
    Boot1,
    /// Boot scratch register 2.
    Boot2,
    /// Boot scratch register 3.
    Boot3,
    /// Raw interrupt status.
    Intr,
    /// Interrupt enable.
    Inte,
    /// Interrupt force.
    Intf,
    /// Masked and forced interrupt status.
    Ints,
}

impl TryFrom<u64> for Rp2350PowmanRegister {
    type Error = DeviceError;

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        let register = offset & 0x0fff;
        let decoded = match register {
            0x00 => Self::BadPasswd,
            0x04 => Self::VregCtrl,
            0x08 => Self::VregSts,
            0x0c => Self::Vreg,
            0x10 => Self::VregLpEntry,
            0x14 => Self::VregLpExit,
            0x18 => Self::BodCtrl,
            0x1c => Self::Bod,
            0x20 => Self::BodLpEntry,
            0x24 => Self::BodLpExit,
            0x28 => Self::Lposc,
            0x2c => Self::ChipReset,
            0x30 => Self::Wdsel,
            0x34 => Self::SeqCfg,
            0x38 => Self::State,
            0x3c => Self::PowFastdiv,
            0x40 => Self::PowDelay,
            0x44 => Self::ExtCtrl0,
            0x48 => Self::ExtCtrl1,
            0x4c => Self::ExtTimeRef,
            0x50 => Self::LposcFreqKhzInt,
            0x54 => Self::LposcFreqKhzFrac,
            0x58 => Self::XoscFreqKhzInt,
            0x5c => Self::XoscFreqKhzFrac,
            0x60 => Self::SetTime63To48,
            0x64 => Self::SetTime47To32,
            0x68 => Self::SetTime31To16,
            0x6c => Self::SetTime15To0,
            0x70 => Self::ReadTimeUpper,
            0x74 => Self::ReadTimeLower,
            0x78 => Self::AlarmTime63To48,
            0x7c => Self::AlarmTime47To32,
            0x80 => Self::AlarmTime31To16,
            0x84 => Self::AlarmTime15To0,
            0x88 => Self::Timer,
            0x8c => Self::Pwrup0,
            0x90 => Self::Pwrup1,
            0x94 => Self::Pwrup2,
            0x98 => Self::Pwrup3,
            0x9c => Self::CurrentPwrupReq,
            0xa0 => Self::LastSwcorePwrup,
            0xa4 => Self::DbgPwrcfg,
            0xa8 => Self::Bootdis,
            0xac => Self::Dbgconfig,
            0xb0 => Self::Scratch0,
            0xb4 => Self::Scratch1,
            0xb8 => Self::Scratch2,
            0xbc => Self::Scratch3,
            0xc0 => Self::Scratch4,
            0xc4 => Self::Scratch5,
            0xc8 => Self::Scratch6,
            0xcc => Self::Scratch7,
            0xd0 => Self::Boot0,
            0xd4 => Self::Boot1,
            0xd8 => Self::Boot2,
            0xdc => Self::Boot3,
            0xe0 => Self::Intr,
            0xe4 => Self::Inte,
            0xe8 => Self::Intf,
            0xec => Self::Ints,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 POWMAN register at offset {register:#x}"
                )));
            }
        };
        Ok(decoded)
    }
}

impl Rp2350PowmanRegister {
    fn index(self) -> usize {
        match self {
            Self::BadPasswd => 0x00 / 4,
            Self::VregCtrl => 0x04 / 4,
            Self::VregSts => 0x08 / 4,
            Self::Vreg => 0x0c / 4,
            Self::VregLpEntry => 0x10 / 4,
            Self::VregLpExit => 0x14 / 4,
            Self::BodCtrl => 0x18 / 4,
            Self::Bod => 0x1c / 4,
            Self::BodLpEntry => 0x20 / 4,
            Self::BodLpExit => 0x24 / 4,
            Self::Lposc => 0x28 / 4,
            Self::ChipReset => 0x2c / 4,
            Self::Wdsel => 0x30 / 4,
            Self::SeqCfg => 0x34 / 4,
            Self::State => 0x38 / 4,
            Self::PowFastdiv => 0x3c / 4,
            Self::PowDelay => 0x40 / 4,
            Self::ExtCtrl0 => 0x44 / 4,
            Self::ExtCtrl1 => 0x48 / 4,
            Self::ExtTimeRef => 0x4c / 4,
            Self::LposcFreqKhzInt => 0x50 / 4,
            Self::LposcFreqKhzFrac => 0x54 / 4,
            Self::XoscFreqKhzInt => 0x58 / 4,
            Self::XoscFreqKhzFrac => 0x5c / 4,
            Self::SetTime63To48 => 0x60 / 4,
            Self::SetTime47To32 => 0x64 / 4,
            Self::SetTime31To16 => 0x68 / 4,
            Self::SetTime15To0 => 0x6c / 4,
            Self::ReadTimeUpper => 0x70 / 4,
            Self::ReadTimeLower => 0x74 / 4,
            Self::AlarmTime63To48 => 0x78 / 4,
            Self::AlarmTime47To32 => 0x7c / 4,
            Self::AlarmTime31To16 => 0x80 / 4,
            Self::AlarmTime15To0 => 0x84 / 4,
            Self::Timer => 0x88 / 4,
            Self::Pwrup0 => 0x8c / 4,
            Self::Pwrup1 => 0x90 / 4,
            Self::Pwrup2 => 0x94 / 4,
            Self::Pwrup3 => 0x98 / 4,
            Self::CurrentPwrupReq => 0x9c / 4,
            Self::LastSwcorePwrup => 0xa0 / 4,
            Self::DbgPwrcfg => 0xa4 / 4,
            Self::Bootdis => 0xa8 / 4,
            Self::Dbgconfig => 0xac / 4,
            Self::Scratch0 => 0xb0 / 4,
            Self::Scratch1 => 0xb4 / 4,
            Self::Scratch2 => 0xb8 / 4,
            Self::Scratch3 => 0xbc / 4,
            Self::Scratch4 => 0xc0 / 4,
            Self::Scratch5 => 0xc4 / 4,
            Self::Scratch6 => 0xc8 / 4,
            Self::Scratch7 => 0xcc / 4,
            Self::Boot0 => 0xd0 / 4,
            Self::Boot1 => 0xd4 / 4,
            Self::Boot2 => 0xd8 / 4,
            Self::Boot3 => 0xdc / 4,
            Self::Intr => 0xe0 / 4,
            Self::Inte => 0xe4 / 4,
            Self::Intf => 0xe8 / 4,
            Self::Ints => 0xec / 4,
        }
    }

    fn mask(self) -> u32 {
        match self {
            Self::BadPasswd => 0x0000_0001,
            Self::VregCtrl => 0x0000_b170,
            Self::VregSts => 0x0000_0011,
            Self::Vreg => 0x0000_81f2,
            Self::VregLpEntry | Self::VregLpExit => 0x0000_01f6,
            Self::BodCtrl => 0x0000_1000,
            Self::Bod | Self::BodLpEntry | Self::BodLpExit => 0x0000_01f1,
            Self::Lposc => 0x0000_03f3,
            Self::ChipReset => 0x1fef_0011,
            Self::Wdsel => 0x0000_1111,
            Self::SeqCfg => 0x0013_11f3,
            Self::State => 0x0000_3fff,
            Self::PowFastdiv => 0x0000_07ff,
            Self::PowDelay => 0x0000_ffff,
            Self::ExtCtrl0 | Self::ExtCtrl1 => 0x0000_713f,
            Self::ExtTimeRef => 0x0000_0013,
            Self::LposcFreqKhzInt => 0x0000_003f,
            Self::LposcFreqKhzFrac | Self::XoscFreqKhzInt | Self::XoscFreqKhzFrac => 0x0000_ffff,
            Self::SetTime63To48
            | Self::SetTime47To32
            | Self::SetTime31To16
            | Self::SetTime15To0
            | Self::AlarmTime63To48
            | Self::AlarmTime47To32
            | Self::AlarmTime31To16
            | Self::AlarmTime15To0 => 0x0000_ffff,
            Self::ReadTimeUpper | Self::ReadTimeLower => u32::MAX,
            Self::Timer => 0x000f_2777,
            Self::Pwrup0 | Self::Pwrup1 | Self::Pwrup2 | Self::Pwrup3 => 0x0000_07ff,
            Self::CurrentPwrupReq | Self::LastSwcorePwrup => 0x0000_007f,
            Self::DbgPwrcfg => 1,
            Self::Bootdis => 3,
            Self::Dbgconfig => 0xf,
            Self::Scratch0
            | Self::Scratch1
            | Self::Scratch2
            | Self::Scratch3
            | Self::Scratch4
            | Self::Scratch5
            | Self::Scratch6
            | Self::Scratch7
            | Self::Boot0
            | Self::Boot1
            | Self::Boot2
            | Self::Boot3 => u32::MAX,
            Self::Intr | Self::Inte | Self::Intf | Self::Ints => 0xf,
        }
    }

    fn writable_mask(self) -> u32 {
        match self {
            Self::BadPasswd
            | Self::VregSts
            | Self::ReadTimeUpper
            | Self::ReadTimeLower
            | Self::CurrentPwrupReq
            | Self::LastSwcorePwrup
            | Self::Ints => 0,
            Self::Vreg => 0x0000_01f2,
            Self::SeqCfg => 0x0000_11f3,
            Self::State => 0x0000_00f0,
            Self::Pwrup0 | Self::Pwrup1 | Self::Pwrup2 | Self::Pwrup3 => 0x0000_01ff,
            Self::Timer => TIMER_CONTROL_MASK,
            Self::ChipReset => CHIP_RESET_DOUBLE_TAP,
            Self::Bootdis => 1 << 1,
            Self::Intr => 0,
            Self::VregCtrl
            | Self::VregLpEntry
            | Self::VregLpExit
            | Self::BodCtrl
            | Self::Bod
            | Self::BodLpEntry
            | Self::BodLpExit
            | Self::Lposc
            | Self::Wdsel
            | Self::PowFastdiv
            | Self::PowDelay
            | Self::ExtCtrl0
            | Self::ExtCtrl1
            | Self::ExtTimeRef
            | Self::LposcFreqKhzInt
            | Self::LposcFreqKhzFrac
            | Self::XoscFreqKhzInt
            | Self::XoscFreqKhzFrac
            | Self::SetTime63To48
            | Self::SetTime47To32
            | Self::SetTime31To16
            | Self::SetTime15To0
            | Self::AlarmTime63To48
            | Self::AlarmTime47To32
            | Self::AlarmTime31To16
            | Self::AlarmTime15To0
            | Self::DbgPwrcfg
            | Self::Dbgconfig
            | Self::Scratch0
            | Self::Scratch1
            | Self::Scratch2
            | Self::Scratch3
            | Self::Scratch4
            | Self::Scratch5
            | Self::Scratch6
            | Self::Scratch7
            | Self::Boot0
            | Self::Boot1
            | Self::Boot2
            | Self::Boot3
            | Self::Inte
            | Self::Intf => self.mask(),
        }
    }
}

fn atomic_update(current: u32, alias: u64, value: u32) -> u32 {
    match alias {
        0 => value,
        1 => current ^ value,
        2 => current | value,
        3 => current & !value,
        _ => unreachable!("atomic alias is limited to two bits"),
    }
}

/// Functional RP2350 power manager and always-on timer subset.
///
/// This model preserves the documented reset values, power-sequencer and
/// wake-source configuration, scratch/boot words, and the 64-bit AON timer.
/// It does not model analog voltage, actual power-domain transitions, or
/// electrical wake pins; those remain outside a functional firmware baseline.
pub struct Rp2350Powman {
    name: String,
    reset: [u32; REGISTER_COUNT],
    registers: [u32; REGISTER_COUNT],
    timer_source: u32,
    time_value: u64,
    time_epoch: SimTime,
}

impl Rp2350Powman {
    /// Creates the documented reset state.
    pub fn new(name: impl Into<String>) -> Self {
        let mut reset = [0; REGISTER_COUNT];
        for (register, value) in [
            (Rp2350PowmanRegister::VregCtrl, 0x8050),
            (Rp2350PowmanRegister::Vreg, 0x00b0),
            (Rp2350PowmanRegister::VregLpEntry, 0x00b4),
            (Rp2350PowmanRegister::VregLpExit, 0x00b0),
            (Rp2350PowmanRegister::Bod, 0x00b1),
            (Rp2350PowmanRegister::BodLpEntry, 0x00b0),
            (Rp2350PowmanRegister::BodLpExit, 0x00b1),
            (Rp2350PowmanRegister::Lposc, 0x0203),
            (Rp2350PowmanRegister::SeqCfg, 0x1011_f0),
            (Rp2350PowmanRegister::State, 0x000f),
            (Rp2350PowmanRegister::PowFastdiv, 0x0040),
            (Rp2350PowmanRegister::PowDelay, 0x2011),
            (Rp2350PowmanRegister::ExtCtrl0, 0x003f),
            (Rp2350PowmanRegister::ExtCtrl1, 0x003f),
            (Rp2350PowmanRegister::LposcFreqKhzInt, 0x20),
            (Rp2350PowmanRegister::LposcFreqKhzFrac, 0xc49c),
            (Rp2350PowmanRegister::XoscFreqKhzInt, 0x2ee0),
            (Rp2350PowmanRegister::Pwrup0, 0x3f),
            (Rp2350PowmanRegister::Pwrup1, 0x3f),
            (Rp2350PowmanRegister::Pwrup2, 0x3f),
            (Rp2350PowmanRegister::Pwrup3, 0x3f),
        ] {
            reset[register.index()] = value;
        }
        Self {
            name: name.into(),
            registers: reset,
            reset,
            timer_source: 0,
            time_value: 0,
            time_epoch: SimTime::ZERO,
        }
    }

    /// Returns the current AON timer value at a simulation timestamp.
    pub fn aon_time(&self, at: SimTime) -> u64 {
        if self.registers[Rp2350PowmanRegister::Timer.index()] & TIMER_RUN != 0 {
            self.time_value
                .wrapping_add(at.ticks().saturating_sub(self.time_epoch.ticks()))
        } else {
            self.time_value
        }
    }

    /// Returns whether the AON alarm is currently pending.
    pub fn alarm_pending(&self, at: SimTime) -> bool {
        self.registers[Rp2350PowmanRegister::Timer.index()] & TIMER_ALARM_ENABLE != 0
            && self.aon_time(at) >= self.alarm_time()
    }

    fn alarm_time(&self) -> u64 {
        (u64::from(self.registers[Rp2350PowmanRegister::AlarmTime63To48.index()] & 0xffff) << 48)
            | (u64::from(self.registers[Rp2350PowmanRegister::AlarmTime47To32.index()] & 0xffff)
                << 32)
            | (u64::from(self.registers[Rp2350PowmanRegister::AlarmTime31To16.index()] & 0xffff)
                << 16)
            | u64::from(self.registers[Rp2350PowmanRegister::AlarmTime15To0.index()] & 0xffff)
    }

    fn set_time(&self) -> u64 {
        (u64::from(self.registers[Rp2350PowmanRegister::SetTime63To48.index()] & 0xffff) << 48)
            | (u64::from(self.registers[Rp2350PowmanRegister::SetTime47To32.index()] & 0xffff)
                << 32)
            | (u64::from(self.registers[Rp2350PowmanRegister::SetTime31To16.index()] & 0xffff)
                << 16)
            | u64::from(self.registers[Rp2350PowmanRegister::SetTime15To0.index()] & 0xffff)
    }

    fn update_time_from_set_registers(&mut self, at: SimTime) {
        self.time_value = self.set_time();
        self.time_epoch = at;
    }

    fn timer_readback(&self, at: SimTime) -> u32 {
        let mut value = self.registers[Rp2350PowmanRegister::Timer.index()] & TIMER_CONTROL_MASK;
        if self.alarm_pending(at) {
            value |= TIMER_ALARM;
        }
        value | (self.timer_source & TIMER_SOURCE_STATUS)
    }

    fn reset_registers(&mut self, kind: ResetKind) {
        let mut persistent = [0; 12];
        let scratch_start = Rp2350PowmanRegister::Scratch0.index();
        let boot_start = Rp2350PowmanRegister::Boot0.index();
        persistent[..8].copy_from_slice(&self.registers[scratch_start..scratch_start + 8]);
        persistent[8..].copy_from_slice(&self.registers[boot_start..boot_start + 4]);
        self.registers = self.reset;
        if kind == ResetKind::Watchdog {
            self.registers[scratch_start..scratch_start + 8].copy_from_slice(&persistent[..8]);
            self.registers[boot_start..boot_start + 4].copy_from_slice(&persistent[8..]);
        }
        self.timer_source = 0;
        self.time_value = 0;
        self.time_epoch = SimTime::ZERO;
    }
}

impl Device for Rp2350Powman {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2350 POWMAN requires aligned word access",
            ));
        }
        let register = Rp2350PowmanRegister::try_from(offset)?;
        let mask = register.mask();
        let value = match register {
            Rp2350PowmanRegister::ReadTimeUpper => (self.aon_time(at) >> 32) as u32,
            Rp2350PowmanRegister::ReadTimeLower => self.aon_time(at) as u32,
            Rp2350PowmanRegister::Timer => self.timer_readback(at),
            Rp2350PowmanRegister::Intr => {
                self.registers[register.index()] | u32::from(self.alarm_pending(at)) << 1
            }
            Rp2350PowmanRegister::Ints => {
                let raw = self.registers[Rp2350PowmanRegister::Intr.index()]
                    | u32::from(self.alarm_pending(at)) << 1;
                raw & self.registers[Rp2350PowmanRegister::Inte.index()]
                    | self.registers[Rp2350PowmanRegister::Intf.index()]
            }
            _ => self.registers[register.index()] & mask,
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2350 POWMAN requires aligned word access",
            ));
        }
        let register = Rp2350PowmanRegister::try_from(offset)?;
        let index = register.index();
        let mask = register.mask();
        let writable_mask = register.writable_mask();
        let alias = (offset >> 12) & 3;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("POWMAN value fits");
        if matches!(
            register,
            Rp2350PowmanRegister::VregSts
                | Rp2350PowmanRegister::CurrentPwrupReq
                | Rp2350PowmanRegister::LastSwcorePwrup
                | Rp2350PowmanRegister::ReadTimeUpper
                | Rp2350PowmanRegister::ReadTimeLower
                | Rp2350PowmanRegister::Ints
        ) {
            return Err(DeviceError::new("RP2350 POWMAN register is read-only"));
        }
        match register {
            Rp2350PowmanRegister::BadPasswd => self.registers[index] &= !(value & mask),
            Rp2350PowmanRegister::ChipReset => {
                let current = self.registers[index];
                let double_tap = atomic_update(current, alias, value & CHIP_RESET_DOUBLE_TAP)
                    & CHIP_RESET_DOUBLE_TAP;
                let rescue_flag =
                    current & CHIP_RESET_RESCUE_FLAG & !(value & CHIP_RESET_RESCUE_FLAG);
                self.registers[index] = (current & CHIP_RESET_RO) | double_tap | rescue_flag;
            }
            Rp2350PowmanRegister::Timer => {
                let timer = self.registers[index];
                let was_running = timer & TIMER_RUN != 0;
                let now = self.aon_time(at);
                let control_value = value & TIMER_CONTROL_MASK;
                let updated = if value & TIMER_COMMAND_MASK != 0 && control_value == 0 {
                    timer
                } else {
                    atomic_update(timer, alias, control_value)
                };
                if value & TIMER_CLEAR != 0 {
                    self.time_value = 0;
                    self.time_epoch = at;
                } else if was_running && updated & TIMER_RUN == 0 {
                    self.time_value = now;
                    self.time_epoch = at;
                } else if !was_running && updated & TIMER_RUN != 0 {
                    self.time_epoch = at;
                }
                self.registers[index] = updated & TIMER_CONTROL_MASK;
                if let Some(source) = match value & TIMER_SOURCE_SELECT {
                    TIMER_SOURCE_LPOSC => Some(1 << 17),
                    TIMER_SOURCE_XOSC => Some(1 << 16),
                    TIMER_SOURCE_GPIO_1KHZ => Some(1 << 18),
                    TIMER_SOURCE_GPIO_1HZ => Some(1 << 19),
                    _ => None,
                } {
                    self.timer_source = source;
                }
            }
            Rp2350PowmanRegister::Intr => self.registers[index] &= !(value & 1),
            Rp2350PowmanRegister::Bootdis => {
                self.registers[index] |= value & (1 << 1);
                self.registers[index] &= !(value & 1);
            }
            Rp2350PowmanRegister::State => {
                let current = self.registers[index];
                let request =
                    atomic_update(current, alias, value & STATE_REQ_MASK) & STATE_REQ_MASK;
                let sticky = current & STATE_WC_MASK & !(value & STATE_WC_MASK);
                self.registers[index] =
                    (current & !(STATE_REQ_MASK | STATE_WC_MASK)) | request | sticky;
            }
            Rp2350PowmanRegister::Pwrup0
            | Rp2350PowmanRegister::Pwrup1
            | Rp2350PowmanRegister::Pwrup2
            | Rp2350PowmanRegister::Pwrup3 => {
                let current = self.registers[index];
                let settings = atomic_update(current, alias, value & writable_mask) & writable_mask;
                let status = current & PWRUP_STATUS & !(value & PWRUP_STATUS);
                self.registers[index] = settings | status | (current & PWRUP_RAW_STATUS);
            }
            Rp2350PowmanRegister::SetTime63To48
            | Rp2350PowmanRegister::SetTime47To32
            | Rp2350PowmanRegister::SetTime31To16
            | Rp2350PowmanRegister::SetTime15To0 => {
                if self.registers[Rp2350PowmanRegister::Timer.index()] & TIMER_RUN == 0 {
                    self.registers[index] =
                        atomic_update(self.registers[index], alias, value) & writable_mask;
                    self.update_time_from_set_registers(at);
                }
            }
            Rp2350PowmanRegister::AlarmTime63To48
            | Rp2350PowmanRegister::AlarmTime47To32
            | Rp2350PowmanRegister::AlarmTime31To16
            | Rp2350PowmanRegister::AlarmTime15To0 => {
                if self.registers[Rp2350PowmanRegister::Timer.index()] & TIMER_ALARM_ENABLE == 0 {
                    self.registers[index] =
                        atomic_update(self.registers[index], alias, value) & writable_mask;
                }
            }
            _ => {
                self.registers[index] =
                    atomic_update(self.registers[index], alias, value) & writable_mask;
            }
        }
        Ok(())
    }

    fn reset(&mut self, kind: ResetKind) {
        self.reset_registers(kind);
    }
}
