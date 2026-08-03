//! STM32L4 RTC calendar and alarm subset.

use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

const TR: u64 = 0x00;
const DR: u64 = 0x04;
const ISR: u64 = 0x0c;
const PRER: u64 = 0x10;
const WUTR: u64 = 0x14;
const CR: u64 = 0x18;
const ALRMAR: u64 = 0x1c;
const ALRMBR: u64 = 0x20;
const WPR: u64 = 0x24;

const ALRAF: u32 = 1 << 8;
const ALRBF: u32 = 1 << 9;
const INITF: u32 = 1 << 6;
const RSF: u32 = 1 << 5;
const ALRAE: u32 = 1 << 8;
const ALRBE: u32 = 1 << 9;
const ALRAIE: u32 = 1 << 12;
const ALRBIE: u32 = 1 << 13;
const INIT: u32 = 1 << 7;
const INITS: u32 = 1 << 4;
const FMT: u32 = 1 << 6;
const MSK1: u32 = 1 << 7;
const MSK2: u32 = 1 << 15;
const MSK3: u32 = 1 << 23;
const WDSEL: u32 = 1 << 30;
const MSK4: u32 = 1 << 31;
const WPR_FIRST_KEY: u32 = 0xca;
const WPR_SECOND_KEY: u32 = 0x53;
const WPR_LOCK_KEY: u32 = 0xff;
const BASE_YEAR: i32 = 2000;
const BASE_DATE_DAYS: i64 = 8_766; // 2024-01-01 relative to 2000-01-01.

fn bcd(value: u8) -> u32 {
    u32::from(value / 10) << 4 | u32::from(value % 10)
}

fn unbcd(value: u32) -> u8 {
    (((value >> 4) & 0xf) * 10 + (value & 0xf)) as u8
}

fn leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn days_in_year(year: i32) -> i64 {
    if leap_year(year) { 366 } else { 365 }
}

fn days_in_month(year: i32, month: u8) -> i64 {
    match month {
        2 if leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn date_to_days(year: i32, month: u8, day: u8) -> i64 {
    let mut days = 0;
    for current in BASE_YEAR..year {
        days += days_in_year(current);
    }
    for current in (year..BASE_YEAR).rev() {
        days -= days_in_year(current);
    }
    for current in 1..month {
        days += days_in_month(year, current);
    }
    days + i64::from(day.saturating_sub(1))
}

fn days_to_date(mut days: i64) -> (i32, u8, u8, u8) {
    let mut year = BASE_YEAR;
    while days < 0 {
        year -= 1;
        days += days_in_year(year);
    }
    while days >= days_in_year(year) {
        days -= days_in_year(year);
        year += 1;
    }
    let mut month = 1;
    while days >= days_in_month(year, month) {
        days -= days_in_month(year, month);
        month += 1;
    }
    let day = days as u8 + 1;
    // 2000-01-01 was Saturday; RTC weekday uses Monday=1 through Sunday=7.
    let weekday = ((date_to_days(year, month, day) + 5).rem_euclid(7) + 1) as u8;
    (year, month, day, weekday)
}

#[derive(Default)]
struct RtcState {
    base_seconds: u64,
    base_tick: u64,
    base_date_days: i64,
    alarm_a: u32,
    alarm_b: u32,
    control: u32,
    status: u32,
    prescaler: u32,
    wakeup: u32,
    write_protect: u8,
    protected: bool,
    init_mode: bool,
}

impl RtcState {
    fn seconds(&self, at: SimTime) -> u64 {
        self.base_seconds
            .saturating_add(at.ticks().saturating_sub(self.base_tick))
    }

    fn calendar(&self, at: SimTime) -> (i64, u64) {
        let total = self.seconds(at);
        (
            self.base_date_days + (total / 86_400) as i64,
            total % 86_400,
        )
    }

    fn time_register(&self, at: SimTime) -> u32 {
        let (_, seconds) = self.calendar(at);
        let hour = (seconds / 3_600) as u8;
        let minute = ((seconds / 60) % 60) as u8;
        let second = (seconds % 60) as u8;
        bcd(hour) << 16 | bcd(minute) << 8 | bcd(second)
    }

    fn date_register(&self, at: SimTime) -> u32 {
        let (days, _) = self.calendar(at);
        let (year, month, day, weekday) = days_to_date(days);
        bcd(year.rem_euclid(100) as u8) << 16
            | u32::from(weekday) << 13
            | bcd(month) << 8
            | bcd(day)
    }

    fn alarm_matches(&self, alarm: u32, at: SimTime) -> bool {
        let (days, seconds) = self.calendar(at);
        let (_, _, day, weekday) = days_to_date(days);
        let hour = unbcd((alarm >> 16) & 0x3f);
        let minute = unbcd((alarm >> 8) & 0x7f);
        let second = unbcd(alarm & 0x7f);
        let hour_matches = alarm & MSK3 != 0 || seconds / 3_600 == u64::from(hour);
        let minute_matches = alarm & MSK2 != 0 || (seconds / 60) % 60 == u64::from(minute);
        let second_matches = alarm & MSK1 != 0 || seconds % 60 == u64::from(second);
        let date_matches = if alarm & MSK4 != 0 {
            true
        } else if alarm & WDSEL != 0 {
            u8::try_from((alarm >> 24) & 7).unwrap_or_default() == weekday
        } else {
            unbcd((alarm >> 24) & 0x3f) == day
        };
        hour_matches && minute_matches && second_matches && date_matches
    }

    fn update_alarms(&mut self, at: SimTime) {
        if self.control & ALRAE != 0 && self.alarm_matches(self.alarm_a, at) {
            self.status |= ALRAF;
        }
        if self.control & ALRBE != 0 && self.alarm_matches(self.alarm_b, at) {
            self.status |= ALRBF;
        }
    }
}

/// Host-facing STM32 RTC state.
#[derive(Clone)]
pub struct Stm32RtcHandle(Arc<Mutex<RtcState>>);

impl Stm32RtcHandle {
    /// Sets the simulated seconds since midnight at the next read/poll.
    pub fn set_seconds(&self, seconds: u64) {
        let mut state = self.0.lock().expect("STM32 RTC lock poisoned");
        state.base_seconds = seconds;
        state.base_tick = 0;
        state.base_date_days = BASE_DATE_DAYS;
        state.status &= !(ALRAF | ALRBF);
    }

    /// Returns whether alarm A or B is latched.
    pub fn alarm_flags(&self) -> (bool, bool) {
        let state = self.0.lock().expect("STM32 RTC lock poisoned");
        (state.status & ALRAF != 0, state.status & ALRBF != 0)
    }
}

/// Functional STM32L432 RTC calendar/alarm register block.
pub struct Stm32Rtc {
    name: String,
    state: Arc<Mutex<RtcState>>,
}

impl Stm32Rtc {
    /// Creates a reset-state RTC.
    pub fn new(name: impl Into<String>) -> (Self, Stm32RtcHandle) {
        let state = Arc::new(Mutex::new(RtcState {
            base_date_days: BASE_DATE_DAYS,
            status: RSF,
            prescaler: 0x007f_00ff,
            protected: true,
            ..RtcState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32RtcHandle(state),
        )
    }
}

impl Device for Stm32Rtc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("STM32 RTC requires aligned word access"));
        }
        let mut state = self.state.lock().expect("STM32 RTC lock poisoned");
        state.update_alarms(at);
        let value = match offset {
            TR => state.time_register(at),
            DR => state.date_register(at),
            ISR => state.status,
            PRER => state.prescaler,
            WUTR => state.wakeup,
            CR => state.control,
            ALRMAR => state.alarm_a,
            ALRMBR => state.alarm_b,
            WPR => 0,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32 RTC read at {offset:#x}"
                )));
            }
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("STM32 RTC requires aligned word access"));
        }
        let mut state = self.state.lock().expect("STM32 RTC lock poisoned");
        let value = value as u32;
        if offset == WPR {
            match value & 0xff {
                WPR_FIRST_KEY => state.write_protect = 1,
                WPR_SECOND_KEY if state.write_protect == 1 => {
                    state.write_protect = 2;
                    state.protected = false;
                }
                WPR_LOCK_KEY => {
                    state.write_protect = 0;
                    state.protected = true;
                }
                _ => {
                    state.write_protect = 0;
                    state.protected = true;
                }
            }
            return Ok(());
        }
        if state.protected {
            return Ok(());
        }
        state.update_alarms(at);
        match offset {
            TR => {
                if !state.init_mode {
                    return Ok(());
                }
                let hour = unbcd((value >> 16) & 0x3f);
                let minute = unbcd((value >> 8) & 0x7f);
                let second = unbcd(value & 0x7f);
                state.base_seconds =
                    u64::from(hour) * 3_600 + u64::from(minute) * 60 + u64::from(second);
                state.base_tick = at.ticks();
            }
            DR => {
                if state.init_mode {
                    let year = i32::from(unbcd((value >> 16) & 0xff)) + BASE_YEAR;
                    let month = unbcd((value >> 8) & 0x1f);
                    let day = unbcd(value & 0x3f);
                    state.base_date_days = date_to_days(year, month, day);
                }
            }
            ISR => {
                if value & INIT != 0 {
                    state.init_mode = true;
                    state.status |= INIT | INITF;
                } else if state.init_mode {
                    state.init_mode = false;
                    state.status &= !(INIT | INITF);
                    state.status |= RSF | INITS;
                }
                if value & ALRAF == 0 {
                    state.status &= !ALRAF;
                }
                if value & ALRBF == 0 {
                    state.status &= !ALRBF;
                }
                if value & RSF == 0 {
                    state.status &= !RSF;
                }
            }
            PRER => {
                if state.init_mode {
                    state.prescaler = value & 0x007f_7fff;
                }
            }
            WUTR => state.wakeup = value & 0xffff,
            CR => state.control = value & (FMT | ALRAE | ALRBE | ALRAIE | ALRBIE),
            ALRMAR => {
                if state.control & ALRAE == 0 {
                    state.alarm_a = value;
                }
            }
            ALRMBR => {
                if state.control & ALRBE == 0 {
                    state.alarm_b = value;
                }
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled STM32 RTC write at {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32 RTC lock poisoned") = RtcState {
            base_date_days: BASE_DATE_DAYS,
            status: RSF,
            prescaler: 0x007f_00ff,
            protected: true,
            ..RtcState::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unlock(rtc: &mut Stm32Rtc) {
        rtc.write(WPR, AccessWidth::Word, WPR_FIRST_KEY.into(), SimTime::ZERO)
            .unwrap();
        rtc.write(WPR, AccessWidth::Word, WPR_SECOND_KEY.into(), SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn calendar_register_is_bcd_and_alarm_latches() {
        let (mut rtc, handle) = Stm32Rtc::new("rtc");
        unlock(&mut rtc);
        rtc.write(ISR, AccessWidth::Word, INIT.into(), SimTime::ZERO)
            .unwrap();
        rtc.write(
            TR,
            AccessWidth::Word,
            u64::from(bcd(12) << 16 | bcd(34) << 8 | bcd(56)),
            SimTime::ZERO,
        )
        .unwrap();
        rtc.write(ISR, AccessWidth::Word, RSF.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rtc.read(TR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32,
            bcd(12) << 16 | bcd(34) << 8 | bcd(56)
        );
        rtc.write(
            ALRMAR,
            AccessWidth::Word,
            u64::from(MSK4 | bcd(12) << 16 | bcd(35) << 8),
            SimTime::ZERO,
        )
        .unwrap();
        rtc.write(CR, AccessWidth::Word, ALRAE.into(), SimTime::ZERO)
            .unwrap();
        let _ = rtc.read(TR, AccessWidth::Word, SimTime::from_ticks(4));
        assert_eq!(handle.alarm_flags(), (true, false));
        rtc.write(ISR, AccessWidth::Word, RSF.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.alarm_flags(), (false, false));
    }

    #[test]
    fn reset_status_reports_synchronization_ready() {
        let (mut rtc, _) = Stm32Rtc::new("rtc");
        assert_eq!(
            rtc.read(ISR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32 & (INITF | RSF),
            RSF
        );
    }

    #[test]
    fn date_register_tracks_programmed_date_and_rollover() {
        let (mut rtc, _) = Stm32Rtc::new("rtc");
        unlock(&mut rtc);
        rtc.write(ISR, AccessWidth::Word, INIT.into(), SimTime::ZERO)
            .unwrap();
        rtc.write(
            DR,
            AccessWidth::Word,
            u64::from(bcd(24) << 16 | 3 << 13 | bcd(2) << 8 | bcd(28)),
            SimTime::ZERO,
        )
        .unwrap();
        rtc.write(ISR, AccessWidth::Word, RSF.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rtc.read(DR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32,
            bcd(24) << 16 | 3 << 13 | bcd(2) << 8 | bcd(28)
        );
        assert_eq!(
            rtc.read(DR, AccessWidth::Word, SimTime::from_ticks(86_400))
                .unwrap() as u32,
            bcd(24) << 16 | 4 << 13 | bcd(2) << 8 | bcd(29)
        );
    }

    #[test]
    fn write_protection_requires_the_native_key_sequence() {
        let (mut rtc, _) = Stm32Rtc::new("rtc");
        rtc.write(TR, AccessWidth::Word, u64::from(bcd(1) << 8), SimTime::ZERO)
            .unwrap();
        assert_eq!(rtc.read(TR, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
        unlock(&mut rtc);
        rtc.write(ISR, AccessWidth::Word, INIT.into(), SimTime::ZERO)
            .unwrap();
        rtc.write(TR, AccessWidth::Word, u64::from(bcd(1) << 8), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rtc.read(TR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32,
            bcd(0) << 16 | bcd(1) << 8
        );
    }
}
