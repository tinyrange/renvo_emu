use super::{GpioHandle, GpioState, SignalHub, refresh_gpio, vendor_gpio};
use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{Logic, SignalId};
use std::sync::{Arc, Mutex};

/// RA4M1 ELC event number for GPT0 counter overflow.
pub const RA4M1_EVENT_GPT0_OVERFLOW: u16 = 0x05d;
/// RA4M1 ELC event number for SCI9 transmit-data-empty.
pub const RA4M1_EVENT_SCI9_TXI: u16 = 0x0a9;
/// RA4M1 ELC event number for an RTC alarm match.
pub const RA4M1_EVENT_RTC_ALARM: u16 = 0x026;

fn input_bits(state: &Arc<Mutex<GpioState>>) -> u16 {
    state
        .lock()
        .expect("RA IOPORT lock poisoned")
        .nets
        .iter()
        .enumerate()
        .fold(0_u16, |value, (pin, net)| {
            value | (u16::from(net.resolved() == Logic::One) << pin)
        })
}

fn lane(value: u32, offset: u64, width: AccessWidth) -> Result<u64, DeviceError> {
    let shift = usize::try_from(offset & 3).expect("register lane fits usize") * 8;
    let bits = usize::from(width.bytes()) * 8;
    if shift + bits > 32 {
        return Err(DeviceError::new(
            "RA register access crosses a word boundary",
        ));
    }
    let mask = if bits == 32 {
        u32::MAX
    } else {
        (1_u32 << bits) - 1
    };
    Ok(u64::from((value >> shift) & mask))
}

fn merge_lane(
    current: u32,
    offset: u64,
    width: AccessWidth,
    value: u64,
) -> Result<u32, DeviceError> {
    let shift = usize::try_from(offset & 3).expect("register lane fits usize") * 8;
    let bits = usize::from(width.bytes()) * 8;
    if shift + bits > 32 {
        return Err(DeviceError::new(
            "RA register access crosses a word boundary",
        ));
    }
    let lane_mask = if bits == 32 {
        u32::MAX
    } else {
        (1_u32 << bits) - 1
    };
    let mask = lane_mask << shift;
    Ok((current & !mask) | (((value as u32) & lane_mask) << shift))
}

/// One RA4M1 16-pin IOPORT register bank.
pub struct RaIoPort {
    name: String,
    state: Arc<Mutex<GpioState>>,
    signals: Vec<SignalId>,
    hub: SignalHub,
    event_output: u16,
}

impl RaIoPort {
    /// Creates one package-visible port and its host pin handle.
    pub fn new(
        name: impl Into<String>,
        path: &str,
        hub: SignalHub,
    ) -> Result<(Self, GpioHandle), remu_signals::SignalError> {
        let (state, signals, handle) = vendor_gpio(16, path, &hub)?;
        Ok((
            Self {
                name: name.into(),
                state,
                signals,
                hub,
                event_output: 0,
            },
            handle,
        ))
    }

    /// Shared state used by the PFS view of the same package pins.
    fn state(&self) -> Arc<Mutex<GpioState>> {
        self.state.clone()
    }

    fn refresh(&self, at: SimTime) -> Result<(), DeviceError> {
        refresh_gpio(&self.state, &self.signals, &self.hub, 16, at)
    }
}

impl Device for RaIoPort {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let aligned = offset & !3;
        let value = match aligned {
            0x00 => {
                let state = self.state.lock().expect("RA IOPORT lock poisoned");
                (state.direction & 0xffff) | ((state.output & 0xffff) << 16)
            }
            0x04 => {
                let input = u32::from(input_bits(&self.state));
                input | (input << 16)
            }
            0x0c => u32::from(self.event_output) | (u32::from(self.event_output) << 16),
            _ => 0,
        };
        lane(value, offset, width)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let aligned = offset & !3;
        match aligned {
            0x00 => {
                let current = {
                    let state = self.state.lock().expect("RA IOPORT lock poisoned");
                    (state.direction & 0xffff) | ((state.output & 0xffff) << 16)
                };
                let updated = merge_lane(current, offset, width, value)?;
                let mut state = self.state.lock().expect("RA IOPORT lock poisoned");
                state.direction = updated & 0xffff;
                state.output = (updated >> 16) & 0xffff;
            }
            0x08 => {
                let command = merge_lane(0, offset, width, value)?;
                let set = command & 0xffff;
                let clear = command >> 16;
                let mut state = self.state.lock().expect("RA IOPORT lock poisoned");
                state.output = (state.output | set) & !clear;
            }
            0x0c => {
                let current = u32::from(self.event_output) | (u32::from(self.event_output) << 16);
                let updated = merge_lane(current, offset, width, value)?;
                self.event_output = ((updated & 0xffff) | (updated >> 16)) as u16;
            }
            _ => {}
        }
        self.refresh(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.event_output = 0;
        let mut state = self.state.lock().expect("RA IOPORT lock poisoned");
        state.direction = 0;
        state.output = 0;
    }
}

/// RA4M1 pin-function-select array sharing the IOPORT pin latches.
pub struct RaPfs {
    name: String,
    states: Vec<Arc<Mutex<GpioState>>>,
    registers: Vec<u32>,
}

impl RaPfs {
    /// Creates the 15-port by 16-pin PFS window.
    pub fn new(name: impl Into<String>, ports: &[RaIoPort]) -> Self {
        Self {
            name: name.into(),
            states: ports.iter().map(RaIoPort::state).collect(),
            registers: vec![0; 15 * 16],
        }
    }

    fn pin_index(offset: u64) -> Option<(usize, usize)> {
        let index = usize::try_from(offset / 4).ok()?;
        let port = index / 16;
        let pin = index % 16;
        (port < 15).then_some((port, pin))
    }
}

impl Device for RaPfs {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let (port, pin) = Self::pin_index(offset)
            .ok_or_else(|| DeviceError::new(format!("unmodeled RA PFS read at {offset:#x}")))?;
        let mut value = self.registers[port * 16 + pin];
        let input = self
            .states
            .get(port)
            .is_some_and(|state| input_bits(state) & (1 << pin) != 0);
        value = (value & !(1 << 1)) | (u32::from(input) << 1);
        lane(value, offset, width)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let (port, pin) = Self::pin_index(offset)
            .ok_or_else(|| DeviceError::new(format!("unmodeled RA PFS write at {offset:#x}")))?;
        let index = port * 16 + pin;
        self.registers[index] =
            merge_lane(self.registers[index], offset, width, value)? & !(1 << 1);
        if let Some(state) = self.states.get(port) {
            let register = self.registers[index];
            let mut state = state.lock().expect("RA PFS lock poisoned");
            let bit = 1_u32 << pin;
            if register & 1 != 0 {
                state.output |= bit;
            } else {
                state.output &= !bit;
            }
            if register & (1 << 2) != 0 {
                state.direction |= bit;
            } else {
                state.direction &= !bit;
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
    }
}

#[derive(Default)]
struct GptState {
    running: bool,
    overflow_interrupt: bool,
    pending: bool,
    started: u64,
    counter: u32,
    period: u32,
    divider: u8,
}

/// Host-facing RA4M1 GPT0 state.
#[derive(Clone)]
pub struct RaGptHandle(Arc<Mutex<GptState>>);

impl RaGptHandle {
    /// Advances GPT0 and reports an overflow event pulse/level.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("RA GPT lock poisoned");
        let divider = 1_u64 << state.divider.min(7);
        let period = u64::from(state.period)
            .saturating_add(1)
            .saturating_mul(divider);
        if state.running && period != 0 && now.ticks().saturating_sub(state.started) >= period {
            state.pending = true;
            state.started = now.ticks();
        }
        state.pending && state.overflow_interrupt
    }
}

/// Functional RA4M1 GPT0 counter/overflow slice.
pub struct RaGpt {
    name: String,
    state: Arc<Mutex<GptState>>,
    registers: [u32; 64],
}

impl RaGpt {
    /// Creates GPT0 and its event handle.
    pub fn new(name: impl Into<String>) -> (Self, RaGptHandle) {
        let state = Arc::new(Mutex::new(GptState {
            period: u32::MAX,
            ..GptState::default()
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 64],
            },
            RaGptHandle(state),
        )
    }
}

impl Device for RaGpt {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RA GPT requires word accesses"));
        }
        let state = self.state.lock().expect("RA GPT lock poisoned");
        let value = match offset {
            0x2c => u32::from(state.running) | (u32::from(state.divider) << 24),
            0x3c => u32::from(state.pending) << 6,
            0x48 => state.counter.wrapping_add(
                (at.ticks().saturating_sub(state.started) >> state.divider.min(7)) as u32,
            ),
            0x64 => state.period,
            _ => self.registers[usize::try_from(offset / 4).unwrap_or(0).min(63)],
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RA GPT requires word accesses"));
        }
        let value = value as u32;
        let mut state = self.state.lock().expect("RA GPT lock poisoned");
        match offset {
            0x2c => {
                state.running = value & 1 != 0;
                state.divider = ((value >> 24) & 7) as u8;
                state.started = at.ticks();
            }
            0x38 => state.overflow_interrupt = value & (3 << 6) != 0,
            0x3c => {
                if value & (1 << 6) == 0 {
                    state.pending = false;
                }
            }
            0x48 => {
                state.counter = value;
                state.started = at.ticks();
            }
            0x64 => state.period = value,
            _ => self.registers[usize::try_from(offset / 4).unwrap_or(0).min(63)] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA GPT lock poisoned") = GptState {
            period: u32::MAX,
            ..GptState::default()
        };
        self.registers.fill(0);
    }
}

#[derive(Default)]
struct SciState {
    scr: u8,
    bytes: Vec<u8>,
}

/// Host-facing RA4M1 SCI9 output and event state.
#[derive(Clone)]
pub struct RaSciHandle(Arc<Mutex<SciState>>);

impl RaSciHandle {
    /// Bytes written to SCI9.TDR.
    pub fn bytes(&self) -> Vec<u8> {
        self.0.lock().expect("RA SCI lock poisoned").bytes.clone()
    }
    /// Whether the TXI event is enabled and asserted.
    pub fn txi_pending(&self) -> bool {
        self.0.lock().expect("RA SCI lock poisoned").scr & 0x80 != 0
    }
}

/// Functional RA4M1 SCI9 asynchronous transmit slice.
pub struct RaSci {
    name: String,
    state: Arc<Mutex<SciState>>,
    registers: [u8; 32],
}

impl RaSci {
    /// Creates SCI9 and its machine handle.
    pub fn new(name: impl Into<String>) -> (Self, RaSciHandle) {
        let state = Arc::new(Mutex::new(SciState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                registers: [0; 32],
            },
            RaSciHandle(state),
        )
    }
}

impl Device for RaSci {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("RA SCI requires byte accesses"));
        }
        let value = match offset {
            0x02 => self.state.lock().expect("RA SCI lock poisoned").scr,
            0x04 => 0x84,
            _ => *self.registers.get(offset as usize).unwrap_or(&0),
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Byte {
            return Err(DeviceError::new("RA SCI requires byte accesses"));
        }
        match offset {
            0x02 => self.state.lock().expect("RA SCI lock poisoned").scr = value as u8,
            0x03 => self
                .state
                .lock()
                .expect("RA SCI lock poisoned")
                .bytes
                .push(value as u8),
            _ => {
                if let Some(register) = self.registers.get_mut(offset as usize) {
                    *register = value as u8;
                }
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA SCI lock poisoned") = SciState::default();
        self.registers.fill(0);
    }
}

const RTC_R64CNT: u64 = 0x00;
const RTC_SECCNT: u64 = 0x02;
const RTC_MINCNT: u64 = 0x04;
const RTC_HRCNT: u64 = 0x06;
const RTC_WKCNT: u64 = 0x08;
const RTC_DAYCNT: u64 = 0x0a;
const RTC_MONCNT: u64 = 0x0c;
const RTC_YRCNT: u64 = 0x0e;
const RTC_SECAL: u64 = 0x10;
const RTC_MINAL: u64 = 0x12;
const RTC_HRAL: u64 = 0x14;
const RTC_WKAL: u64 = 0x16;
const RTC_DAYAL: u64 = 0x18;
const RTC_MONAL: u64 = 0x1a;
const RTC_YRAL: u64 = 0x1c;
const RTC_YRAL_HI: u64 = 0x1d;
const RTC_YRAREN: u64 = 0x1e;
const RTC_RCR1: u64 = 0x22;
const RTC_RCR2: u64 = 0x24;
const RTC_RCR4: u64 = 0x28;
const RTC_RCR1_AIE: u8 = 1 << 0;
const RTC_RCR1_CIE: u8 = 1 << 1;
const RTC_RCR1_PIE: u8 = 1 << 2;
const RTC_RCR1_RTCOS: u8 = 1 << 3;
const RTC_RCR1_PES_MASK: u8 = 0xf0;
const RTC_RCR2_START: u8 = 1 << 0;
const RTC_RCR2_RESET: u8 = 1 << 1;
const RTC_RCR2_ADJ30: u8 = 1 << 2;
const RTC_RCR2_RTCOE: u8 = 1 << 3;
const RTC_RCR2_AADJE: u8 = 1 << 4;
const RTC_RCR2_AADJP: u8 = 1 << 5;
const RTC_RCR2_HR24: u8 = 1 << 6;
const RTC_RCR2_CNTMD: u8 = 1 << 7;
const RTC_ALARM_ENABLE: u8 = 1 << 7;
const RTC_YEAR_ALARM_ENABLE: u8 = 1;

#[derive(Clone, Copy)]
struct RtcCalendar {
    year: u16,
    month: u8,
    day: u8,
    weekday: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl Default for RtcCalendar {
    fn default() -> Self {
        Self {
            year: 2000,
            month: 1,
            day: 1,
            weekday: 6,
            hour: 0,
            minute: 0,
            second: 0,
        }
    }
}

fn to_bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

fn from_bcd(value: u8) -> u8 {
    (value >> 4).saturating_mul(10).saturating_add(value & 0x0f)
}

// Days from 2000-01-01. The RTC's calendar registers are BCD encoded, but
// the simulator keeps an integer epoch so reads remain deterministic when an
// instruction advances more than one calendar field.
fn days_from_calendar(calendar: RtcCalendar) -> u64 {
    let year = i64::from(calendar.year);
    let month = i64::from(calendar.month.max(1));
    let day = i64::from(calendar.day.max(1));
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let epoch = 10_957_i64; // civil days for 2000-01-01 in the Unix-day scale
    u64::try_from(days.saturating_sub(epoch)).unwrap_or(0)
}

fn calendar_from_seconds(seconds: u64) -> RtcCalendar {
    // The emulator's calendar starts at 2000-01-01. A small bounded search is
    // preferable to another date dependency in this device-only crate.
    let mut remaining_days = seconds / 86_400;
    let mut year = 2000_u16;
    loop {
        let leap = u16::from(year % 4 == 0 && (year % 100 != 0 || year % 400 == 0));
        let year_days = 365 + leap;
        if remaining_days < u64::from(year_days) {
            break;
        }
        remaining_days -= u64::from(year_days);
        year = year.saturating_add(1);
    }
    let month_days = [
        31_u16,
        28 + u16::from(year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)),
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1_u8;
    for days in month_days {
        if remaining_days < u64::from(days) {
            break;
        }
        remaining_days -= u64::from(days);
        month = month.saturating_add(1);
    }
    let day_index = seconds % 86_400;
    let weekday = ((6 + seconds / 86_400) % 7) as u8;
    RtcCalendar {
        year,
        month,
        day: u8::try_from(remaining_days + 1).unwrap_or(31),
        weekday,
        hour: u8::try_from(day_index / 3_600).unwrap_or(23),
        minute: u8::try_from((day_index / 60) % 60).unwrap_or(59),
        second: u8::try_from(day_index % 60).unwrap_or(59),
    }
}

#[derive(Default)]
struct RtcState {
    running: bool,
    base_tick: u64,
    base_seconds: u64,
    rcr1: u8,
    rcr2: u8,
    rcr4: u8,
    alarms: [u8; 6],
    year_alarm: u16,
    year_alarm_enable: bool,
    alarm_pending: bool,
    alarm_request_sent: bool,
    last_alarm_second: Option<u64>,
}

impl RtcState {
    fn seconds(&self, now: SimTime) -> u64 {
        self.base_seconds.saturating_add(
            self.running
                .then_some(now.ticks().saturating_sub(self.base_tick))
                .unwrap_or(0),
        )
    }

    fn refresh(&mut self, now: SimTime) {
        if !self.running {
            return;
        }
        let calendar = calendar_from_seconds(self.seconds(now));
        let values = [
            calendar.second,
            calendar.minute,
            calendar.hour,
            calendar.weekday,
            calendar.day,
            calendar.month,
        ];
        let seconds = self.seconds(now);
        let fields_match =
            self.alarms
                .iter()
                .zip(values)
                .enumerate()
                .all(|(index, (&alarm, value))| {
                    alarm & RTC_ALARM_ENABLE == 0
                        || from_bcd(alarm & alarm_value_mask(index)) == value
                });
        let year_matches = !self.year_alarm_enable
            || from_bcd(self.year_alarm as u8) == (calendar.year % 100) as u8;
        let any_alarm_enabled = self
            .alarms
            .iter()
            .any(|alarm| alarm & RTC_ALARM_ENABLE != 0)
            || self.year_alarm_enable;
        if fields_match
            && year_matches
            && any_alarm_enabled
            && self.last_alarm_second != Some(seconds)
        {
            self.alarm_pending = true;
            self.alarm_request_sent = false;
            self.last_alarm_second = Some(seconds);
        }
    }

    fn clear_alarm_latch(&mut self) {
        self.alarm_pending = false;
        self.alarm_request_sent = false;
        self.last_alarm_second = None;
    }
}

fn alarm_value_mask(index: usize) -> u8 {
    match index {
        // ENB is bit 7; the lower seven bits are BCD seconds/minutes.
        0 | 1 => 0x7f,
        // ENB is bit 7 and PM is ignored by the functional 24-hour model.
        2 => 0x3f,
        // ENB plus the three-bit weekday value.
        3 => 0x07,
        // ENB plus date bits (bit 6 is reserved).
        4 => 0x3f,
        // ENB plus month bits (bits 6 and 5 are reserved).
        5 => 0x1f,
        _ => 0x7f,
    }
}

/// Host-facing RA4M1 RTC alarm and calendar state.
#[derive(Clone)]
pub struct RaRtcHandle(Arc<Mutex<RtcState>>);

impl RaRtcHandle {
    /// Polls the calendar and reports an enabled alarm interrupt request.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("RA RTC lock poisoned");
        state.refresh(now);
        if state.alarm_pending && state.rcr1 & RTC_RCR1_AIE != 0 && !state.alarm_request_sent {
            // The real alarm request is latched in the ICU IELSR IR bit. Emit
            // one request per counter match; the machine/firmware clears the
            // corresponding ICU pending bit before another match can occur.
            state.alarm_request_sent = true;
            true
        } else {
            false
        }
    }

    /// Returns whether the alarm flag is latched.
    pub fn alarm_pending(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("RA RTC lock poisoned");
        state.refresh(now);
        state.alarm_pending
    }
}

/// Functional RA4M1 RTC calendar, binary counter, and alarm slice.
pub struct RaRtc {
    name: String,
    state: Arc<Mutex<RtcState>>,
}

impl RaRtc {
    /// Creates a stopped RTC initialized to 2000-01-01 00:00:00.
    pub fn new(name: impl Into<String>) -> (Self, RaRtcHandle) {
        let state = Arc::new(Mutex::new(RtcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaRtcHandle(state),
        )
    }

    fn read_byte(&self, offset: u64, at: SimTime) -> u8 {
        let mut state = self.state.lock().expect("RA RTC lock poisoned");
        state.refresh(at);
        let seconds = state.seconds(at);
        let calendar = calendar_from_seconds(seconds);
        match offset {
            // The functional scheduler advances in whole RTC seconds, so no
            // sub-second phase is represented yet. Bit 7 is reserved and
            // reads as zero on the hardware.
            RTC_R64CNT => 0,
            RTC_SECCNT => to_bcd(calendar.second),
            RTC_MINCNT => to_bcd(calendar.minute),
            RTC_HRCNT => to_bcd(calendar.hour),
            RTC_WKCNT => to_bcd(calendar.weekday),
            RTC_DAYCNT => to_bcd(calendar.day),
            RTC_MONCNT => to_bcd(calendar.month),
            RTC_YRCNT => to_bcd((calendar.year % 100) as u8),
            RTC_SECAL | RTC_MINAL | RTC_HRAL | RTC_WKAL | RTC_DAYAL | RTC_MONAL => {
                let index = usize::try_from((offset - RTC_SECAL) / 2).unwrap_or(0);
                state.alarms[index]
            }
            RTC_YRAL => state.year_alarm as u8,
            RTC_YRAL_HI => 0,
            RTC_YRAREN => u8::from(state.year_alarm_enable),
            RTC_RCR1 => state.rcr1,
            RTC_RCR2 => state.rcr2,
            RTC_RCR4 => state.rcr4,
            _ => 0,
        }
    }

    fn write_byte(&mut self, offset: u64, value: u8, at: SimTime) {
        let mut state = self.state.lock().expect("RA RTC lock poisoned");
        state.refresh(at);
        let current = calendar_from_seconds(state.seconds(at));
        let mut updated = current;
        match offset {
            RTC_SECCNT => updated.second = from_bcd(value).min(59),
            RTC_MINCNT => updated.minute = from_bcd(value).min(59),
            RTC_HRCNT => updated.hour = from_bcd(value).min(23),
            RTC_WKCNT => updated.weekday = from_bcd(value).min(6),
            RTC_DAYCNT => updated.day = from_bcd(value).clamp(1, 31),
            RTC_MONCNT => updated.month = from_bcd(value).clamp(1, 12),
            // RYRCNT is a 16-bit register, but only the low byte is the
            // BCD year (00..99); the high byte is reserved and ignored.
            RTC_YRCNT => updated.year = (updated.year / 100) * 100 + u16::from(from_bcd(value)),
            RTC_SECAL | RTC_MINAL | RTC_HRAL | RTC_WKAL | RTC_DAYAL | RTC_MONAL => {
                let index = usize::try_from((offset - RTC_SECAL) / 2).unwrap_or(0);
                state.alarms[index] = match index {
                    0 | 1 => value & 0x7f | (value & RTC_ALARM_ENABLE),
                    2 => value & 0x7f | (value & RTC_ALARM_ENABLE),
                    3 => value & 0x87,
                    4 => value & 0xbf,
                    5 => value & 0x9f,
                    _ => value,
                };
                state.clear_alarm_latch();
                return;
            }
            RTC_YRAL => {
                state.year_alarm = u16::from(value & 0x7f);
                state.clear_alarm_latch();
                return;
            }
            RTC_YRAL_HI => return,
            RTC_YRAREN => {
                state.year_alarm_enable = value & RTC_YEAR_ALARM_ENABLE != 0;
                state.clear_alarm_latch();
                return;
            }
            RTC_RCR1 => {
                state.rcr1 = value
                    & (RTC_RCR1_AIE
                        | RTC_RCR1_CIE
                        | RTC_RCR1_PIE
                        | RTC_RCR1_RTCOS
                        | RTC_RCR1_PES_MASK);
                return;
            }
            RTC_RCR2 => {
                let previous_seconds = state.seconds(at);
                if value & RTC_RCR2_RESET != 0 {
                    // An RTC software reset initializes the prescaler and
                    // alarm/adjustment registers. The calendar counters are
                    // battery-backed and are not reset by this operation.
                    state.alarms = [0; 6];
                    state.year_alarm = 0;
                    state.year_alarm_enable = false;
                    state.clear_alarm_latch();
                    state.rcr1 &= !(RTC_RCR1_CIE | RTC_RCR1_RTCOS);
                    state.rcr2 &=
                        !(RTC_RCR2_ADJ30 | RTC_RCR2_AADJE | RTC_RCR2_AADJP | RTC_RCR2_RTCOE);
                    state.base_seconds = previous_seconds;
                    state.base_tick = at.ticks();
                }
                if value & RTC_RCR2_ADJ30 != 0 {
                    let second = current.second;
                    let adjustment = if second < 30 {
                        -i64::from(second)
                    } else {
                        i64::from(60 - second)
                    };
                    state.base_seconds = if adjustment.is_negative() {
                        previous_seconds.saturating_sub(adjustment.unsigned_abs())
                    } else {
                        previous_seconds.saturating_add(adjustment as u64)
                    };
                    state.base_tick = at.ticks();
                }
                // RESET and ADJ30 are command bits and clear when the
                // operation completes. START is the only run/stop control;
                // RTCOE (bit 3) is merely the RTCOUT pin enable.
                state.rcr2 = value
                    & (RTC_RCR2_START
                        | RTC_RCR2_RTCOE
                        | RTC_RCR2_AADJE
                        | RTC_RCR2_AADJP
                        | RTC_RCR2_HR24
                        | RTC_RCR2_CNTMD);
                state.running = value & RTC_RCR2_START != 0;
                state.base_seconds = state.seconds(at);
                state.base_tick = at.ticks();
                return;
            }
            RTC_RCR4 => {
                state.rcr4 = value & 1;
                return;
            }
            _ => return,
        }
        state.clear_alarm_latch();
        state.base_seconds = days_from_calendar(updated)
            .saturating_mul(86_400)
            .saturating_add(u64::from(updated.hour) * 3_600)
            .saturating_add(u64::from(updated.minute) * 60)
            .saturating_add(u64::from(updated.second));
        state.base_tick = at.ticks();
    }
}

impl Device for RaRtc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if !matches!(
            width,
            AccessWidth::Byte | AccessWidth::HalfWord | AccessWidth::Word
        ) {
            return Err(DeviceError::new(
                "RA RTC does not support double-word accesses",
            ));
        }
        let mut value = 0_u64;
        for byte in 0..u64::from(width.bytes()) {
            value |= u64::from(self.read_byte(offset + byte, at)) << (byte * 8);
        }
        Ok(value)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if !matches!(
            width,
            AccessWidth::Byte | AccessWidth::HalfWord | AccessWidth::Word
        ) {
            return Err(DeviceError::new(
                "RA RTC does not support double-word accesses",
            ));
        }
        for byte in 0..u64::from(width.bytes()) {
            self.write_byte(offset + byte, (value >> (byte * 8)) as u8, at);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        // The RA4M1 RTC is in the battery-backed domain and its counters are
        // explicitly not initialized by MCU, watchdog, external, or software
        // system resets. Construction provides the initial power-on state;
        // RCR2.RESET models the separate RTC software-reset command.
    }
}

struct IcuState {
    ielsr: [u32; 96],
}

impl Default for IcuState {
    fn default() -> Self {
        Self { ielsr: [0; 96] }
    }
}

/// Host-facing explicit ICU event-to-NVIC router.
#[derive(Clone)]
pub struct RaIcuHandle(Arc<Mutex<IcuState>>);

impl RaIcuHandle {
    /// Latches one ELC event and returns every NVIC line configured for it.
    pub fn route_event(&self, event: u16) -> Vec<u16> {
        let mut state = self.0.lock().expect("RA ICU lock poisoned");
        let mut lines = Vec::new();
        for (line, register) in state.ielsr.iter_mut().enumerate() {
            if *register & 0x1ff == u32::from(event) && *register & (1 << 16) == 0 {
                *register |= 1 << 16;
                lines.push(u16::try_from(line).expect("RA ICU line fits u16"));
            }
        }
        lines
    }
}

/// RA4M1 ICU IELSR register window.
pub struct RaIcu {
    name: String,
    state: Arc<Mutex<IcuState>>,
}

impl RaIcu {
    /// Creates the ICU and shared routing handle.
    pub fn new(name: impl Into<String>) -> (Self, RaIcuHandle) {
        let state = Arc::new(Mutex::new(IcuState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaIcuHandle(state),
        )
    }
}

impl Device for RaIcu {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !(0x300..0x480).contains(&offset) || offset & 3 != 0 {
            return Ok(0);
        }
        let index = usize::try_from((offset - 0x300) / 4).expect("IELSR index fits usize");
        Ok(u64::from(
            self.state.lock().expect("RA ICU lock poisoned").ielsr[index],
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !(0x300..0x480).contains(&offset) || offset & 3 != 0 {
            return Ok(());
        }
        let index = usize::try_from((offset - 0x300) / 4).expect("IELSR index fits usize");
        let mut state = self.state.lock().expect("RA ICU lock poisoned");
        let old = state.ielsr[index];
        let mut updated = value as u32 & 0x0101_01ff;
        if value & (1 << 16) != 0 {
            updated = (updated & !(1 << 16)) | (old & (1 << 16));
        }
        state.ielsr[index] = updated;
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state
            .lock()
            .expect("RA ICU lock poisoned")
            .ielsr
            .fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioport_atomic_output_and_pfs_direction_are_visible() {
        let hub = SignalHub::new();
        let (mut port, handle) = RaIoPort::new("port1", "board.ra.port1", hub).unwrap();
        port.write(0, AccessWidth::Word, 1 << 11, SimTime::ZERO)
            .unwrap();
        port.write(8, AccessWidth::Word, 1 << 11, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.output(), 1 << 11);
        assert_eq!(handle.direction(), 1 << 11);
    }

    #[test]
    fn gpt_and_sci_events_route_through_ielsr() {
        let (mut icu, handle) = RaIcu::new("icu");
        icu.write(
            0x300 + 7 * 4,
            AccessWidth::Word,
            u64::from(RA4M1_EVENT_GPT0_OVERFLOW),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.route_event(RA4M1_EVENT_GPT0_OVERFLOW), vec![7]);

        let (mut gpt, gpt_handle) = RaGpt::new("gpt0");
        gpt.write(0x64, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        gpt.write(0x38, AccessWidth::Word, 1 << 6, SimTime::ZERO)
            .unwrap();
        gpt.write(0x2c, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(gpt_handle.poll(SimTime::from_ticks(4)));

        let (mut sci, sci_handle) = RaSci::new("sci9");
        sci.write(3, AccessWidth::Byte, b'R'.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(sci_handle.bytes(), b"R");
    }

    #[test]
    fn rtc_calendar_ticks_and_alarm_are_bcd_encoded() {
        let (mut rtc, handle) = RaRtc::new("rtc");
        rtc.write(
            RTC_RCR2,
            AccessWidth::Byte,
            u64::from(RTC_RCR2_START),
            SimTime::ZERO,
        )
        .unwrap();
        rtc.write(
            RTC_SECAL,
            AccessWidth::Byte,
            u64::from(RTC_ALARM_ENABLE | 0x02),
            SimTime::ZERO,
        )
        .unwrap();
        rtc.write(
            RTC_RCR1,
            AccessWidth::Byte,
            u64::from(RTC_RCR1_AIE),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            rtc.read(RTC_SECCNT, AccessWidth::Byte, SimTime::from_ticks(2))
                .unwrap(),
            0x02
        );
        assert!(handle.poll(SimTime::from_ticks(2)));
        assert_eq!(
            rtc.read(RTC_RCR1, AccessWidth::Byte, SimTime::from_ticks(2))
                .unwrap(),
            u64::from(RTC_RCR1_AIE)
        );
        // Alarm requests are latched by the ICU event, not exposed as a
        // status bit in RCR1. A second poll for the same matching second is
        // therefore not a duplicate request.
        assert!(!handle.poll(SimTime::from_ticks(2)));
        rtc.write(
            RTC_SECAL,
            AccessWidth::Byte,
            u64::from(RTC_ALARM_ENABLE | 0x03),
            SimTime::from_ticks(2),
        )
        .unwrap();
        assert!(!handle.alarm_pending(SimTime::from_ticks(2)));
    }

    #[test]
    fn rtc_calendar_register_writes_set_time_and_year() {
        let (mut rtc, _) = RaRtc::new("rtc");
        rtc.write(RTC_SECCNT, AccessWidth::Byte, 0x56, SimTime::ZERO)
            .unwrap();
        rtc.write(RTC_MINCNT, AccessWidth::Byte, 0x34, SimTime::ZERO)
            .unwrap();
        rtc.write(RTC_HRCNT, AccessWidth::Byte, 0x12, SimTime::ZERO)
            .unwrap();
        rtc.write(RTC_YRCNT, AccessWidth::HalfWord, 0x2024, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rtc.read(RTC_SECCNT, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x56
        );
        assert_eq!(
            rtc.read(RTC_MINCNT, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x34
        );
        assert_eq!(
            rtc.read(RTC_HRCNT, AccessWidth::Byte, SimTime::ZERO)
                .unwrap(),
            0x12
        );
        assert_eq!(
            rtc.read(RTC_YRCNT, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0x0024
        );
    }

    #[test]
    fn rtc_uses_native_start_and_year_alarm_registers() {
        let (mut rtc, handle) = RaRtc::new("rtc");
        // RTCOE is an output-enable bit, not a clock-enable gate.
        rtc.write(
            RTC_RCR2,
            AccessWidth::Byte,
            u64::from(RTC_RCR2_RTCOE),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            rtc.read(RTC_SECCNT, AccessWidth::Byte, SimTime::from_ticks(3))
                .unwrap(),
            0
        );
        rtc.write(
            RTC_RCR2,
            AccessWidth::Byte,
            u64::from(RTC_RCR2_START),
            SimTime::ZERO,
        )
        .unwrap();
        rtc.write(RTC_YRAL, AccessWidth::HalfWord, 0x0000, SimTime::ZERO)
            .unwrap();
        rtc.write(RTC_YRAREN, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        rtc.write(
            RTC_RCR1,
            AccessWidth::Byte,
            u64::from(RTC_RCR1_AIE),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.poll(SimTime::from_ticks(1)));
        assert_eq!(
            rtc.read(RTC_YRAL, AccessWidth::HalfWord, SimTime::from_ticks(1))
                .unwrap(),
            0
        );
    }

    #[test]
    fn rtc_software_reset_does_not_erase_calendar_but_clears_alarms() {
        let (mut rtc, handle) = RaRtc::new("rtc");
        rtc.write(RTC_SECCNT, AccessWidth::Byte, 0x42, SimTime::ZERO)
            .unwrap();
        rtc.write(
            RTC_SECAL,
            AccessWidth::Byte,
            RTC_ALARM_ENABLE.into(),
            SimTime::ZERO,
        )
        .unwrap();
        rtc.write(
            RTC_RCR2,
            AccessWidth::Byte,
            u64::from(RTC_RCR2_START),
            SimTime::ZERO,
        )
        .unwrap();
        rtc.write(
            RTC_RCR2,
            AccessWidth::Byte,
            u64::from(RTC_RCR2_START | RTC_RCR2_RESET),
            SimTime::from_ticks(1),
        )
        .unwrap();
        assert_eq!(
            rtc.read(RTC_SECCNT, AccessWidth::Byte, SimTime::from_ticks(1))
                .unwrap(),
            0x43
        );
        assert!(!handle.alarm_pending(SimTime::from_ticks(1)));
        assert_eq!(
            rtc.read(RTC_RCR2, AccessWidth::Byte, SimTime::from_ticks(1))
                .unwrap()
                & u64::from(RTC_RCR2_RESET),
            0
        );
    }
}
