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

fn bcd(value: u8) -> u32 {
    u32::from(value / 10) << 4 | u32::from(value % 10)
}

fn unbcd(value: u32) -> u8 {
    (((value >> 4) & 0xf) * 10 + (value & 0xf)) as u8
}

#[derive(Default)]
struct RtcState {
    base_seconds: u64,
    base_tick: u64,
    alarm_a: u32,
    alarm_b: u32,
    control: u32,
    status: u32,
    prescaler: u32,
    wakeup: u32,
    write_protect: u8,
}

impl RtcState {
    fn seconds(&self, at: SimTime) -> u64 {
        self.base_seconds
            .saturating_add(at.ticks().saturating_sub(self.base_tick))
    }

    fn time_register(&self, at: SimTime) -> u32 {
        let seconds = self.seconds(at) % 86_400;
        let hour = (seconds / 3_600) as u8;
        let minute = ((seconds / 60) % 60) as u8;
        let second = (seconds % 60) as u8;
        bcd(hour) << 16 | bcd(minute) << 8 | bcd(second)
    }

    fn alarm_matches(&self, alarm: u32, at: SimTime) -> bool {
        let seconds = self.seconds(at) % 86_400;
        let hour = ((alarm >> 16) & 0x3f) as u8;
        let minute = ((alarm >> 8) & 0x7f) as u8;
        let second = (alarm & 0x7f) as u8;
        let expected = u64::from(unbcd(u32::from(hour))) * 3_600
            + u64::from(unbcd(u32::from(minute))) * 60
            + u64::from(unbcd(u32::from(second)));
        seconds == expected
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
            status: INITF | RSF,
            prescaler: 0x007f_00ff,
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
            DR => 0x0024_0101, // Tuesday, 1 January 2024 in BCD form.
            ISR => state.status,
            PRER => state.prescaler,
            WUTR => state.wakeup,
            CR => state.control,
            ALRMAR => state.alarm_a,
            ALRMBR => state.alarm_b,
            WPR => u32::from(state.write_protect),
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
        state.update_alarms(at);
        let value = value as u32;
        match offset {
            TR => {
                let hour = unbcd((value >> 16) & 0x3f);
                let minute = unbcd((value >> 8) & 0x7f);
                let second = unbcd(value & 0x7f);
                state.base_seconds =
                    u64::from(hour) * 3_600 + u64::from(minute) * 60 + u64::from(second);
                state.base_tick = at.ticks();
            }
            DR => {}
            ISR => {
                if value & ALRAF != 0 {
                    state.status &= !ALRAF;
                }
                if value & ALRBF != 0 {
                    state.status &= !ALRBF;
                }
                if value & INITF != 0 {
                    state.status |= INITF;
                }
            }
            PRER => state.prescaler = value,
            WUTR => state.wakeup = value & 0x0fff,
            CR => state.control = value & (ALRAE | ALRBE | ALRAIE | ALRBIE),
            ALRMAR => state.alarm_a = value,
            ALRMBR => state.alarm_b = value,
            WPR => state.write_protect = value as u8,
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
            status: INITF | RSF,
            prescaler: 0x007f_00ff,
            ..RtcState::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_register_is_bcd_and_alarm_latches() {
        let (mut rtc, handle) = Stm32Rtc::new("rtc");
        rtc.write(
            TR,
            AccessWidth::Word,
            u64::from(bcd(12) << 16 | bcd(34) << 8 | bcd(56)),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            rtc.read(TR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32,
            bcd(12) << 16 | bcd(34) << 8 | bcd(56)
        );
        rtc.write(
            ALRMAR,
            AccessWidth::Word,
            u64::from(bcd(12) << 16 | bcd(35) << 8),
            SimTime::ZERO,
        )
        .unwrap();
        rtc.write(CR, AccessWidth::Word, ALRAE.into(), SimTime::ZERO)
            .unwrap();
        let _ = rtc.read(TR, AccessWidth::Word, SimTime::from_ticks(4));
        assert_eq!(handle.alarm_flags(), (true, false));
        rtc.write(ISR, AccessWidth::Word, ALRAF.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.alarm_flags(), (false, false));
    }

    #[test]
    fn reset_status_reports_synchronization_ready() {
        let (mut rtc, _) = Stm32Rtc::new("rtc");
        assert_eq!(
            rtc.read(ISR, AccessWidth::Word, SimTime::ZERO).unwrap() as u32 & (INITF | RSF),
            INITF | RSF
        );
    }
}
