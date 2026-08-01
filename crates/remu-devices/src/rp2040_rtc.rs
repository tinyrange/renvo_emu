use super::{AccessWidth, Arc, Device, DeviceError, Mutex, ResetKind, Rp2040Resets, SimTime};

/// RP2040 RTC register identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Rp2040RtcRegister {
    /// One-second divider minus one.
    ClockDivider = 0x00,
    /// Year, month, and day setup fields.
    Setup0 = 0x04,
    /// Weekday and time-of-day setup fields.
    Setup1 = 0x08,
    /// Load and enable control.
    Control = 0x0c,
    /// Year, month, and day alarm fields.
    IrqSetup0 = 0x10,
    /// Weekday and time-of-day alarm fields.
    IrqSetup1 = 0x14,
    /// Current year, month, and day.
    Rtc1 = 0x18,
    /// Current weekday and time; read before Rtc1 to latch it.
    Rtc0 = 0x1c,
    /// Raw alarm interrupt.
    Intr = 0x20,
    /// Alarm interrupt enable.
    Inte = 0x24,
    /// Alarm interrupt force.
    Intf = 0x28,
    /// Masked alarm interrupt status.
    Ints = 0x2c,
}

impl TryFrom<u64> for Rp2040RtcRegister {
    type Error = ();

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x00 => Ok(Self::ClockDivider),
            0x04 => Ok(Self::Setup0),
            0x08 => Ok(Self::Setup1),
            0x0c => Ok(Self::Control),
            0x10 => Ok(Self::IrqSetup0),
            0x14 => Ok(Self::IrqSetup1),
            0x18 => Ok(Self::Rtc1),
            0x1c => Ok(Self::Rtc0),
            0x20 => Ok(Self::Intr),
            0x24 => Ok(Self::Inte),
            0x28 => Ok(Self::Intf),
            0x2c => Ok(Self::Ints),
            _ => Err(()),
        }
    }
}

const CTRL_ENABLE: u32 = 1;
const CTRL_ACTIVE: u32 = 1 << 1;
const CTRL_LOAD: u32 = 1 << 4;
const CTRL_FORCE_NOT_LEAP: u32 = 1 << 8;
const MATCH_ACTIVE: u32 = 1 << 29;
const MATCH_ENABLE: u32 = 1 << 28;
const YEAR_ENABLE: u32 = 1 << 26;
const MONTH_ENABLE: u32 = 1 << 25;
const DAY_ENABLE: u32 = 1 << 24;
const DOTW_ENABLE: u32 = 1 << 31;
const HOUR_ENABLE: u32 = 1 << 30;
const MINUTE_ENABLE: u32 = 1 << 29;
const SECOND_ENABLE: u32 = 1 << 28;
const INTERRUPT: u32 = 1;
const SETUP0_MASK: u32 = (0x0fff << 12) | (0x0f << 8) | 0x1f;
const SETUP1_MASK: u32 = (0x07 << 24) | (0x1f << 16) | (0x3f << 8) | 0x3f;
const IRQ_SETUP0_MASK: u32 = MATCH_ENABLE | YEAR_ENABLE | MONTH_ENABLE | DAY_ENABLE | SETUP0_MASK;
const IRQ_SETUP1_MASK: u32 =
    DOTW_ENABLE | HOUR_ENABLE | MINUTE_ENABLE | SECOND_ENABLE | SETUP1_MASK;

#[derive(Clone, Copy, Default)]
struct DateTime {
    year: u16,
    month: u8,
    day: u8,
    dotw: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl DateTime {
    fn from_setup(setup0: u32, setup1: u32) -> Self {
        let mut value = Self {
            year: u16::try_from((setup0 >> 12) & 0x0fff).expect("RTC year fits"),
            month: u8::try_from((setup0 >> 8) & 0x0f).expect("RTC month fits"),
            day: u8::try_from(setup0 & 0x1f).expect("RTC day fits"),
            dotw: u8::try_from((setup1 >> 24) & 7).expect("RTC weekday fits"),
            hour: u8::try_from((setup1 >> 16) & 0x1f).expect("RTC hour fits"),
            minute: u8::try_from((setup1 >> 8) & 0x3f).expect("RTC minute fits"),
            second: u8::try_from(setup1 & 0x3f).expect("RTC second fits"),
        };
        value.normalize(false);
        value
    }

    fn normalize(&mut self, force_not_leap: bool) {
        self.month = self.month.clamp(1, 12);
        self.day = self.day.clamp(
            1,
            Self::days_in_month(self.year, self.month, force_not_leap),
        );
        self.dotw %= 7;
        self.hour %= 24;
        self.minute %= 60;
        self.second %= 60;
    }

    fn days_in_month(year: u16, month: u8, force_not_leap: bool) -> u8 {
        match month {
            2 if year.is_multiple_of(4) && !force_not_leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    fn tick(&mut self, force_not_leap: bool) {
        self.second += 1;
        if self.second < 60 {
            return;
        }
        self.second = 0;
        self.minute += 1;
        if self.minute < 60 {
            return;
        }
        self.minute = 0;
        self.hour += 1;
        if self.hour < 24 {
            return;
        }
        self.hour = 0;
        self.dotw = (self.dotw + 1) % 7;
        self.day += 1;
        if self.day <= Self::days_in_month(self.year, self.month, force_not_leap) {
            return;
        }
        self.day = 1;
        self.month += 1;
        if self.month <= 12 {
            return;
        }
        self.month = 1;
        self.year = (self.year + 1) & 0x0fff;
    }

    fn setup0(self) -> u32 {
        (u32::from(self.year) << 12) | (u32::from(self.month) << 8) | u32::from(self.day)
    }

    fn setup1(self) -> u32 {
        (u32::from(self.dotw) << 24)
            | (u32::from(self.hour) << 16)
            | (u32::from(self.minute) << 8)
            | u32::from(self.second)
    }
}

#[derive(Clone)]
struct State {
    divider: u16,
    setup0: u32,
    setup1: u32,
    control: u32,
    irq_setup0: u32,
    irq_setup1: u32,
    datetime: DateTime,
    divider_ticks: u64,
    last_time: SimTime,
    raw: bool,
    enabled: bool,
    forced: bool,
    latched_rtc1: Option<u32>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            divider: 0,
            setup0: 0,
            setup1: 0,
            control: 0,
            irq_setup0: 0,
            irq_setup1: 0,
            datetime: DateTime::default(),
            divider_ticks: 0,
            last_time: SimTime::ZERO,
            raw: false,
            enabled: false,
            forced: false,
            latched_rtc1: None,
        }
    }
}

impl State {
    fn advance(&mut self, now: SimTime) {
        let elapsed = now.ticks().saturating_sub(self.last_time.ticks());
        self.last_time = now;
        if elapsed == 0 || self.control & CTRL_ENABLE == 0 {
            return;
        }
        let divider = u64::from(self.divider) + 1;
        let total = self.divider_ticks.saturating_add(elapsed);
        let mut seconds = total / divider;
        self.divider_ticks = total % divider;
        while seconds != 0 {
            self.datetime.tick(self.control & CTRL_FORCE_NOT_LEAP != 0);
            if self.matches_alarm() {
                self.raw = true;
            }
            seconds -= 1;
        }
    }

    fn matches_alarm(&self) -> bool {
        if self.irq_setup0 & MATCH_ENABLE == 0 {
            return false;
        }
        let setup0 = self.datetime.setup0();
        let setup1 = self.datetime.setup1();
        (self.irq_setup0 & YEAR_ENABLE == 0
            || (self.irq_setup0 >> 12) & 0x0fff == (setup0 >> 12) & 0x0fff)
            && (self.irq_setup0 & MONTH_ENABLE == 0
                || (self.irq_setup0 >> 8) & 0x0f == (setup0 >> 8) & 0x0f)
            && (self.irq_setup0 & DAY_ENABLE == 0 || self.irq_setup0 & 0x1f == setup0 & 0x1f)
            && (self.irq_setup1 & DOTW_ENABLE == 0
                || (self.irq_setup1 >> 24) & 7 == (setup1 >> 24) & 7)
            && (self.irq_setup1 & HOUR_ENABLE == 0
                || (self.irq_setup1 >> 16) & 0x1f == (setup1 >> 16) & 0x1f)
            && (self.irq_setup1 & MINUTE_ENABLE == 0
                || (self.irq_setup1 >> 8) & 0x3f == (setup1 >> 8) & 0x3f)
            && (self.irq_setup1 & SECOND_ENABLE == 0 || self.irq_setup1 & 0x3f == setup1 & 0x3f)
    }

    fn control_value(&self) -> u32 {
        (self.control & CTRL_FORCE_NOT_LEAP)
            | (self.control & CTRL_ENABLE)
            | if self.control & CTRL_ENABLE != 0 {
                CTRL_ACTIVE
            } else {
                0
            }
    }

    fn interrupt_status(&self) -> u32 {
        u32::from((self.raw && self.enabled) || self.forced)
    }

    fn apply_alias(register: &mut u32, alias: u64, value: u32) -> Result<(), DeviceError> {
        Rp2040Resets::update(register, alias, value)
    }
}

/// Scheduler-facing RP2040 RTC alarm state.
#[derive(Clone)]
pub struct Rp2040RtcHandle {
    state: Arc<Mutex<State>>,
}

impl Rp2040RtcHandle {
    /// Advances the calendar and returns masked alarm state.
    pub fn pending(&self, now: SimTime) -> bool {
        let mut state = self.state.lock().expect("RP2040 RTC lock poisoned");
        state.advance(now);
        state.interrupt_status() != 0
    }
}

/// Functional RP2040 real-time calendar and alarm peripheral.
pub struct Rp2040Rtc {
    name: String,
    state: Arc<Mutex<State>>,
}

impl Rp2040Rtc {
    /// Creates a stopped RTC without a scheduler handle.
    pub fn new(name: impl Into<String>) -> Self {
        let (device, _) = Self::new_with_handle(name);
        device
    }

    /// Creates the RTC and an alarm handle for a machine scheduler.
    pub fn new_with_handle(name: impl Into<String>) -> (Self, Rp2040RtcHandle) {
        let state = Arc::new(Mutex::new(State::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Rp2040RtcHandle { state },
        )
    }

    fn register(offset: u64) -> Result<Rp2040RtcRegister, DeviceError> {
        Rp2040RtcRegister::try_from(offset)
            .map_err(|()| DeviceError::new(format!("unmodeled RP2040 RTC register at {offset:#x}")))
    }
}

impl Device for Rp2040Rtc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2040 RTC requires aligned word accesses",
            ));
        }
        let register = Self::register(offset & 0x0fff)?;
        let mut state = self.state.lock().expect("RP2040 RTC lock poisoned");
        state.advance(at);
        let value = match register {
            Rp2040RtcRegister::ClockDivider => u32::from(state.divider),
            Rp2040RtcRegister::Setup0 => state.setup0,
            Rp2040RtcRegister::Setup1 => state.setup1,
            Rp2040RtcRegister::Control => state.control_value(),
            Rp2040RtcRegister::IrqSetup0 => {
                (state.irq_setup0 & IRQ_SETUP0_MASK)
                    | if state.matches_alarm() {
                        MATCH_ACTIVE
                    } else {
                        0
                    }
            }
            Rp2040RtcRegister::IrqSetup1 => state.irq_setup1 & IRQ_SETUP1_MASK,
            Rp2040RtcRegister::Rtc1 => state
                .latched_rtc1
                .take()
                .unwrap_or_else(|| state.datetime.setup0()),
            Rp2040RtcRegister::Rtc0 => {
                state.latched_rtc1 = Some(state.datetime.setup0());
                state.datetime.setup1()
            }
            Rp2040RtcRegister::Intr => u32::from(state.raw),
            Rp2040RtcRegister::Inte => u32::from(state.enabled),
            Rp2040RtcRegister::Intf => u32::from(state.forced),
            Rp2040RtcRegister::Ints => state.interrupt_status(),
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
                "RP2040 RTC requires aligned word accesses",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register = Self::register(offset & 0x0fff)?;
        let mut state = self.state.lock().expect("RP2040 RTC lock poisoned");
        state.advance(at);
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked RTC value fits");
        match register {
            Rp2040RtcRegister::ClockDivider => {
                let mut divider = u32::from(state.divider);
                State::apply_alias(&mut divider, alias, value & 0xffff)?;
                state.divider = u16::try_from(divider & 0xffff).expect("RTC divider fits");
            }
            Rp2040RtcRegister::Setup0 => {
                State::apply_alias(&mut state.setup0, alias, value & SETUP0_MASK)?;
                state.setup0 &= SETUP0_MASK;
            }
            Rp2040RtcRegister::Setup1 => {
                State::apply_alias(&mut state.setup1, alias, value & SETUP1_MASK)?;
                state.setup1 &= SETUP1_MASK;
            }
            Rp2040RtcRegister::Control => {
                let load = value & CTRL_LOAD != 0;
                let mut control = state.control & (CTRL_ENABLE | CTRL_FORCE_NOT_LEAP);
                State::apply_alias(
                    &mut control,
                    alias,
                    value & (CTRL_ENABLE | CTRL_FORCE_NOT_LEAP),
                )?;
                state.control = control;
                if load {
                    state.datetime = DateTime::from_setup(state.setup0, state.setup1);
                    let force_not_leap = state.control & CTRL_FORCE_NOT_LEAP != 0;
                    state.datetime.normalize(force_not_leap);
                    state.divider_ticks = 0;
                    state.raw = false;
                    state.latched_rtc1 = None;
                }
            }
            Rp2040RtcRegister::IrqSetup0 => {
                State::apply_alias(&mut state.irq_setup0, alias, value & IRQ_SETUP0_MASK)?;
                state.irq_setup0 &= IRQ_SETUP0_MASK;
            }
            Rp2040RtcRegister::IrqSetup1 => {
                State::apply_alias(&mut state.irq_setup1, alias, value & IRQ_SETUP1_MASK)?;
                state.irq_setup1 &= IRQ_SETUP1_MASK;
            }
            Rp2040RtcRegister::Rtc0 | Rp2040RtcRegister::Rtc1 => {
                return Err(DeviceError::new(
                    "RP2040 RTC current time registers are read-only",
                ));
            }
            Rp2040RtcRegister::Intr => {
                if value & INTERRUPT != 0 {
                    state.raw = false;
                }
            }
            Rp2040RtcRegister::Inte => {
                let mut enabled = u32::from(state.enabled);
                State::apply_alias(&mut enabled, alias, value & INTERRUPT)?;
                state.enabled = enabled & INTERRUPT != 0;
            }
            Rp2040RtcRegister::Intf => {
                let mut forced = u32::from(state.forced);
                State::apply_alias(&mut forced, alias, value & INTERRUPT)?;
                state.forced = forced & INTERRUPT != 0;
            }
            Rp2040RtcRegister::Ints => {
                return Err(DeviceError::new("RP2040 RTC INTS is read-only"));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RP2040 RTC lock poisoned") = State::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(device: &mut Rp2040Rtc, register: Rp2040RtcRegister, value: u32, at: u64) {
        device
            .write(
                register as u64,
                AccessWidth::Word,
                u64::from(value),
                SimTime::from_ticks(at),
            )
            .unwrap();
    }

    fn read(device: &mut Rp2040Rtc, register: Rp2040RtcRegister, at: u64) -> u32 {
        u32::try_from(
            device
                .read(register as u64, AccessWidth::Word, SimTime::from_ticks(at))
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn calendar_loads_advances_and_latches_ordered_reads() {
        let (mut rtc, handle) = Rp2040Rtc::new_with_handle("rtc");
        write(
            &mut rtc,
            Rp2040RtcRegister::Setup0,
            (2024 << 12) | (2 << 8) | 28,
            0,
        );
        write(
            &mut rtc,
            Rp2040RtcRegister::Setup1,
            (3 << 24) | (23 << 16) | (59 << 8) | 59,
            0,
        );
        write(
            &mut rtc,
            Rp2040RtcRegister::Control,
            CTRL_LOAD | CTRL_ENABLE,
            0,
        );
        assert_eq!(
            read(&mut rtc, Rp2040RtcRegister::Control, 0),
            CTRL_ACTIVE | CTRL_ENABLE
        );
        let rtc0 = read(&mut rtc, Rp2040RtcRegister::Rtc0, 1);
        assert_eq!(rtc0 & 0x3f, 0);
        assert_eq!(
            read(&mut rtc, Rp2040RtcRegister::Rtc1, 1),
            (2024 << 12) | (2 << 8) | 29
        );
        assert!(!handle.pending(SimTime::from_ticks(1)));
    }

    #[test]
    fn alarm_latches_raw_and_masked_interrupt_until_cleared() {
        let (mut rtc, handle) = Rp2040Rtc::new_with_handle("rtc");
        write(
            &mut rtc,
            Rp2040RtcRegister::Setup0,
            (2024 << 12) | (1 << 8) | 1,
            0,
        );
        write(&mut rtc, Rp2040RtcRegister::Setup1, 0, 0);
        write(
            &mut rtc,
            Rp2040RtcRegister::Control,
            CTRL_LOAD | CTRL_ENABLE,
            0,
        );
        write(
            &mut rtc,
            Rp2040RtcRegister::IrqSetup0,
            MATCH_ENABLE | DAY_ENABLE | (1 << 0),
            0,
        );
        write(&mut rtc, Rp2040RtcRegister::IrqSetup1, SECOND_ENABLE | 2, 0);
        write(&mut rtc, Rp2040RtcRegister::Inte, INTERRUPT, 0);
        assert!(!handle.pending(SimTime::from_ticks(1)));
        assert!(handle.pending(SimTime::from_ticks(2)));
        assert_eq!(read(&mut rtc, Rp2040RtcRegister::Intr, 2), INTERRUPT);
        assert_eq!(read(&mut rtc, Rp2040RtcRegister::Ints, 2), INTERRUPT);
        write(&mut rtc, Rp2040RtcRegister::Intr, INTERRUPT, 2);
        assert_eq!(read(&mut rtc, Rp2040RtcRegister::Ints, 2), 0);
        write(&mut rtc, Rp2040RtcRegister::Intf, INTERRUPT, 2);
        assert_eq!(read(&mut rtc, Rp2040RtcRegister::Ints, 2), INTERRUPT);
        write(&mut rtc, Rp2040RtcRegister::Intf, 0, 2);
        assert_eq!(read(&mut rtc, Rp2040RtcRegister::Ints, 2), 0);
    }
}
