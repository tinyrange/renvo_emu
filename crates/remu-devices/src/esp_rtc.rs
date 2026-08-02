use super::*;

const ESP_RTC_ULP_INT_BIT: u32 = 1 << 5;
const ESP_RTC_ULP_TIMER_ENABLE: u32 = 1 << 31;
const ESP_RTC_ULP_TIMER_PERIOD_MASK: u32 = 0x00ff_ffff;

struct EspRtcControlState {
    registers: Vec<u32>,
    ulp_started: bool,
    ulp_last_tick: u64,
    ulp_wakeups: u64,
    ulp_signal: Option<(SignalHub, SignalId)>,
}

impl EspRtcControlState {
    fn new(ulp_signal: Option<(SignalHub, SignalId)>) -> Self {
        Self {
            registers: vec![0; 0x1000 / 4],
            ulp_started: false,
            ulp_last_tick: 0,
            ulp_wakeups: 0,
            ulp_signal,
        }
    }

    fn signal(&self, value: bool, at: SimTime) {
        if let Some((hub, signal)) = &self.ulp_signal {
            hub.set(
                *signal,
                SignalValue::from_u64(u64::from(value), 1)
                    .expect("ULP interrupt signal is one bit wide"),
                at,
            )
            .expect("ULP interrupt signal remains declared");
        }
    }

    fn refresh_ulp(&mut self, now: SimTime) {
        let timer = self.registers[0xfc / 4];
        let period = (self.registers[0x134 / 4] >> 8 & ESP_RTC_ULP_TIMER_PERIOD_MASK).max(1);
        if timer & ESP_RTC_ULP_TIMER_ENABLE == 0 || !self.ulp_started {
            return;
        }
        let elapsed = now.ticks().saturating_sub(self.ulp_last_tick);
        let periods = elapsed / u64::from(period);
        if periods == 0 {
            return;
        }
        self.ulp_last_tick = self
            .ulp_last_tick
            .saturating_add(periods.saturating_mul(u64::from(period)));
        self.ulp_wakeups = self.ulp_wakeups.saturating_add(periods);
        self.registers[0x44 / 4] |= ESP_RTC_ULP_INT_BIT;
        self.registers[0x48 / 4] |= ESP_RTC_ULP_INT_BIT;
        self.signal(true, now);
    }

    fn clear_ulp_interrupt(&mut self, at: SimTime) {
        self.registers[0x44 / 4] &= !ESP_RTC_ULP_INT_BIT;
        self.registers[0x48 / 4] &= !ESP_RTC_ULP_INT_BIT;
        self.signal(false, at);
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.ulp_started = false;
        self.ulp_last_tick = 0;
        self.ulp_wakeups = 0;
        self.signal(false, SimTime::ZERO);
    }
}

/// Host-side view of the ESP32-S3 ULP/RTC wakeup path.
#[derive(Clone)]
pub struct EspRtcControlHandle {
    state: Rc<RefCell<EspRtcControlState>>,
}

impl EspRtcControlHandle {
    /// Returns true when the ULP interrupt is both raw and enabled.
    pub fn ulp_pending(&self, now: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        state.refresh_ulp(now);
        state.registers[0x44 / 4] & state.registers[0x40 / 4] & ESP_RTC_ULP_INT_BIT != 0
    }

    /// Returns the number of deterministic ULP timer wakeups observed.
    pub fn ulp_wakeups(&self) -> u64 {
        self.state.borrow().ulp_wakeups
    }
}

/// Functional ESP32-S3 RTC control and ULP timer block.
pub struct EspRtcControl {
    name: String,
    state: Rc<RefCell<EspRtcControlState>>,
}

impl EspRtcControl {
    /// Creates the RTC control page in its power-on state.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: Rc::new(RefCell::new(EspRtcControlState::new(None))),
        }
    }

    /// Creates the RTC page, host handle, and traceable ULP interrupt signal.
    pub fn new_with_signals(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, EspRtcControlHandle), SignalError> {
        let signal = hub.declare(
            "board.esp32s3.ulp.interrupt",
            SignalValue::from_u64(0, 1)?,
            Some("ESP32-S3 ULP timer interrupt".to_string()),
        )?;
        let state = Rc::new(RefCell::new(EspRtcControlState::new(Some((hub, signal)))));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspRtcControlHandle { state },
        ))
    }
}

impl Device for EspRtcControl {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP RTC control requires aligned word access",
            ));
        }
        let mut state = self.state.borrow_mut();
        state.refresh_ulp(at);
        state
            .registers
            .get(usize::try_from(offset / 4).expect("RTC offset fits"))
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP RTC control requires aligned word access",
            ));
        }
        let mut state = self.state.borrow_mut();
        state.refresh_ulp(at);
        let index = usize::try_from(offset / 4).expect("RTC offset fits");
        if index >= state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = value as u32;
        match offset {
            // RTC_CNTL_TIME_UPDATE latches the functional abstract time.
            0x0c if value & (1 << 31) != 0 => {
                state.registers[index] = value;
                let counter = at.ticks();
                state.registers[0x10 / 4] = counter as u32;
                state.registers[0x14 / 4] = (counter >> 32) as u32;
            }
            // RTC_CNTL_INT_CLR has write-one-to-clear semantics.
            0x4c => {
                state.registers[index] = 0;
                if value & ESP_RTC_ULP_INT_BIT != 0 {
                    state.clear_ulp_interrupt(at);
                }
            }
            // RTC_CNTL_ULP_CP_CTRL starts or resets the functional ULP timer.
            0x100 => {
                state.registers[index] = value;
                if value & ((1 << 31) | (1 << 30)) != 0 {
                    state.ulp_started = true;
                    state.ulp_last_tick = at.ticks();
                }
                if value & (1 << 29) != 0 {
                    state.ulp_started = false;
                    state.clear_ulp_interrupt(at);
                }
            }
            // The ULP timer and period are native R/W fields.
            0xfc | 0x134 => state.registers[index] = value,
            // Raw/status registers are read-only on the native block.
            0x44 | 0x48 => {}
            _ => state.registers[index] = value,
        }
        // SENS_SAR_MEAS1_CTRL2 shares the RTC peripheral page at 0x800.
        // A software-triggered functional conversion completes immediately.
        // Keep the selected pad/control fields, clear START, assert DONE, and
        // return a deterministic zero sample in the low 16 bits.
        if matches!(offset, 0x80c | 0x830) && value & (1 << 17) != 0 {
            state.registers[index] = (value & !((1 << 17) | 0xffff)) | (1 << 16);
        }
        state.refresh_ulp(at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulp_timer_sets_and_clears_the_native_rtc_interrupt() {
        let hub = SignalHub::new();
        let (mut device, handle) = EspRtcControl::new_with_signals("rtc", hub).unwrap();
        device
            .write(0x134, AccessWidth::Word, 4_u64 << 8, SimTime::ZERO)
            .unwrap();
        device
            .write(
                0x40,
                AccessWidth::Word,
                u64::from(ESP_RTC_ULP_INT_BIT),
                SimTime::ZERO,
            )
            .unwrap();
        device
            .write(0x100, AccessWidth::Word, 1_u64 << 31, SimTime::ZERO)
            .unwrap();
        device
            .write(
                0xfc,
                AccessWidth::Word,
                u64::from(ESP_RTC_ULP_TIMER_ENABLE),
                SimTime::ZERO,
            )
            .unwrap();

        assert!(!handle.ulp_pending(SimTime::from_ticks(3)));
        assert!(handle.ulp_pending(SimTime::from_ticks(4)));
        assert_eq!(handle.ulp_wakeups(), 1);
        assert_eq!(
            device.read(0x44, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 5
        );
        assert_eq!(
            device.read(0x48, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 5
        );

        device
            .write(
                0x4c,
                AccessWidth::Word,
                u64::from(ESP_RTC_ULP_INT_BIT),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.ulp_pending(SimTime::from_ticks(4)));
    }
}
