use super::*;

/// WCH watchdog peripheral family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WchWatchdogKind {
    /// Independent watchdog (`IWDG`) clocked from the low-speed source.
    Independent,
    /// Window watchdog (`WWDG`) clocked from the peripheral clock.
    Windowed,
}

/// A level reported by a WCH watchdog scheduler handle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WchWatchdogEvent {
    /// The window watchdog early-warning flag became set.
    pub early_warning: bool,
    /// The watchdog reached its reset condition.
    pub reset: bool,
}

/// Scheduler-facing handle for a WCH watchdog.
#[derive(Clone)]
pub struct WchWatchdogHandle {
    state: Rc<RefCell<WchWatchdogState>>,
}

impl WchWatchdogHandle {
    /// Advances the functional watchdog and reports newly visible events.
    ///
    /// Simulation ticks intentionally represent abstract peripheral time. They
    /// are deterministic and preserve feed/window/reset ordering, but do not
    /// claim the exact WCH oscillator or APB clock frequency.
    pub fn poll(&self, now: SimTime) -> WchWatchdogEvent {
        let mut state = self.state.borrow_mut();
        state.update(now);
        WchWatchdogEvent {
            early_warning: state.early_warning,
            reset: state.reset_requested,
        }
    }

    /// Consumes a pending watchdog reset request.
    pub fn take_reset(&self, now: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        state.update(now);
        std::mem::take(&mut state.reset_requested)
    }

    /// Returns whether the window watchdog early-warning flag is asserted.
    pub fn early_warning_pending(&self, now: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        state.update(now);
        state.early_warning
    }
}

struct WchWatchdogState {
    kind: WchWatchdogKind,
    enabled: bool,
    unlocked: bool,
    prescaler: u16,
    reload: u16,
    counter: u16,
    window: u16,
    early_warning_enabled: bool,
    early_warning: bool,
    reset_requested: bool,
    epoch: u64,
}

impl WchWatchdogState {
    fn reset(kind: WchWatchdogKind) -> Self {
        Self {
            kind,
            enabled: false,
            unlocked: false,
            prescaler: 0,
            reload: 0x0fff,
            counter: 0x0fff,
            window: 0x007f,
            early_warning_enabled: false,
            early_warning: false,
            reset_requested: false,
            epoch: 0,
        }
    }

    fn independent_divider(&self) -> u64 {
        match self.prescaler & 7 {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 8,
            4 => 16,
            5 => 32,
            6 => 64,
            _ => 128,
        }
    }

    fn window_divider(&self) -> u64 {
        1_u64 << u32::from((self.prescaler & 3) + 1)
    }

    fn update(&mut self, now: SimTime) {
        if !self.enabled || self.reset_requested {
            return;
        }
        let divider = match self.kind {
            WchWatchdogKind::Independent => self.independent_divider(),
            WchWatchdogKind::Windowed => self.window_divider(),
        };
        let elapsed = now.ticks().saturating_sub(self.epoch);
        let steps = elapsed / divider;
        if steps == 0 {
            return;
        }
        self.epoch = self.epoch.saturating_add(steps.saturating_mul(divider));
        match self.kind {
            WchWatchdogKind::Independent => {
                let old = self.counter;
                self.counter = old.saturating_sub(
                    u16::try_from(steps.min(u64::from(u16::MAX)))
                        .expect("clamped IWDG step count fits u16"),
                );
                if self.counter == 0 {
                    self.reset_requested = true;
                }
            }
            WchWatchdogKind::Windowed => {
                let old = self.counter;
                self.counter = old.saturating_sub(
                    u16::try_from(steps.min(u64::from(u16::MAX)))
                        .expect("clamped WWDG step count fits u16"),
                );
                if self.early_warning_enabled && old > 0x40 && self.counter <= 0x40 {
                    self.early_warning = true;
                }
                if self.counter < 0x40 {
                    self.reset_requested = true;
                }
            }
        }
    }

    fn restart(&mut self, now: SimTime) {
        self.epoch = now.ticks();
    }
}

/// Functional WCH CH32V00x independent or window watchdog register block.
///
/// The model covers the key, prescaler, reload, counter/window, early-warning,
/// and status semantics used by vendor startup and HAL code. Countdown time is
/// deliberately abstract; machine integration can consume the returned reset
/// and early-warning events without depending on host wall-clock time.
pub struct WchWatchdog {
    name: String,
    kind: WchWatchdogKind,
    state: Rc<RefCell<WchWatchdogState>>,
}

impl WchWatchdog {
    /// Creates an independent watchdog and scheduler handle.
    pub fn new_iwdg(name: impl Into<String>) -> (Self, WchWatchdogHandle) {
        Self::new(name, WchWatchdogKind::Independent)
    }

    /// Creates a window watchdog and scheduler handle.
    pub fn new_wwdg(name: impl Into<String>) -> (Self, WchWatchdogHandle) {
        Self::new(name, WchWatchdogKind::Windowed)
    }

    fn new(name: impl Into<String>, kind: WchWatchdogKind) -> (Self, WchWatchdogHandle) {
        let state = Rc::new(RefCell::new(WchWatchdogState::reset(kind)));
        (
            Self {
                name: name.into(),
                kind,
                state: state.clone(),
            },
            WchWatchdogHandle { state },
        )
    }

    fn require_register_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if !matches!(width, AccessWidth::HalfWord | AccessWidth::Word) || offset & 3 != 0 {
            return Err(DeviceError::new(
                "WCH watchdog requires aligned halfword or word access",
            ));
        }
        Ok(())
    }
}

impl Device for WchWatchdog {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        Self::require_register_access(offset, width)?;
        let mut state = self.state.borrow_mut();
        state.update(at);
        let value = match self.kind {
            WchWatchdogKind::Independent => match offset {
                0x00 => 0,
                0x04 => state.prescaler & 7,
                0x08 => state.reload & 0x0fff,
                0x0c => 0,
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled WCH IWDG read at offset {offset:#x}"
                    )));
                }
            },
            WchWatchdogKind::Windowed => match offset {
                0x00 => (u16::from(state.enabled) << 7) | (state.counter & 0x7f),
                0x04 => {
                    state.window & 0x7f
                        | ((state.prescaler & 3) << 7)
                        | (u16::from(state.early_warning_enabled) << 9)
                }
                0x08 => u16::from(state.early_warning),
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled WCH WWDG read at offset {offset:#x}"
                    )));
                }
            },
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
        Self::require_register_access(offset, width)?;
        let value = u16::try_from(value & u64::from(u16::MAX))
            .expect("masked WCH watchdog register value fits u16");
        let mut state = self.state.borrow_mut();
        state.update(at);
        match self.kind {
            WchWatchdogKind::Independent => match offset {
                0x00 => match value {
                    0x5555 => state.unlocked = true,
                    0xcccc => {
                        state.enabled = true;
                        state.counter = state.reload;
                        state.restart(at);
                    }
                    0xaaaa => {
                        state.counter = state.reload;
                        state.restart(at);
                    }
                    _ => {}
                },
                0x04 if state.unlocked => state.prescaler = value & 7,
                0x08 if state.unlocked => {
                    state.reload = value & 0x0fff;
                    state.counter = state.reload;
                    state.restart(at);
                }
                0x04 | 0x08 => {}
                0x0c => {}
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled WCH IWDG write at offset {offset:#x}"
                    )));
                }
            },
            WchWatchdogKind::Windowed => match offset {
                0x00 => {
                    let next_counter = value & 0x7f;
                    if state.enabled && state.window != 0 && next_counter > state.window {
                        state.reset_requested = true;
                    } else {
                        state.enabled = value & 0x80 != 0;
                        state.counter = next_counter;
                        state.restart(at);
                    }
                }
                0x04 => {
                    state.window = value & 0x7f;
                    state.prescaler = (value >> 7) & 3;
                    state.early_warning_enabled = value & (1 << 9) != 0;
                }
                0x08 => {
                    if value & 1 != 0 {
                        state.early_warning = false;
                    }
                }
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled WCH WWDG write at offset {offset:#x}"
                    )));
                }
            },
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = WchWatchdogState::reset(self.kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_key_sequence_unlocks_configures_and_feeds() {
        let (mut device, handle) = WchWatchdog::new_iwdg("iwdg");
        device
            .write(0x00, AccessWidth::HalfWord, 0x5555, SimTime::ZERO)
            .unwrap();
        device
            .write(0x04, AccessWidth::HalfWord, 0, SimTime::ZERO)
            .unwrap();
        device
            .write(0x08, AccessWidth::HalfWord, 3, SimTime::ZERO)
            .unwrap();
        device
            .write(0x00, AccessWidth::HalfWord, 0xcccc, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            device
                .read(0x08, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            3
        );
        assert!(!handle.poll(SimTime::from_ticks(2)).reset);
        device
            .write(0x00, AccessWidth::HalfWord, 0xaaaa, SimTime::from_ticks(2))
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(4)).reset);
        assert!(handle.poll(SimTime::from_ticks(5)).reset);
        assert!(handle.take_reset(SimTime::from_ticks(5)));
    }

    #[test]
    fn window_watchdog_reports_early_warning_and_rejects_early_feed() {
        let (mut device, handle) = WchWatchdog::new_wwdg("wwdg");
        device
            .write(0x04, AccessWidth::Word, 0x260, SimTime::ZERO)
            .unwrap();
        device
            .write(0x00, AccessWidth::Word, 0xd0, SimTime::ZERO)
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(32)).early_warning);
        assert_eq!(
            device
                .read(0x08, AccessWidth::Word, SimTime::from_ticks(32))
                .unwrap(),
            1
        );
        device
            .write(0x08, AccessWidth::Word, 1, SimTime::from_ticks(32))
            .unwrap();
        assert_eq!(
            device
                .read(0x08, AccessWidth::Word, SimTime::from_ticks(32))
                .unwrap(),
            0
        );

        device
            .write(0x04, AccessWidth::Word, 0x240, SimTime::from_ticks(32))
            .unwrap();
        device
            .write(0x00, AccessWidth::Word, 0xc0, SimTime::from_ticks(32))
            .unwrap();
        device
            .write(0x00, AccessWidth::Word, 0xff, SimTime::from_ticks(32))
            .unwrap();
        assert!(handle.take_reset(SimTime::from_ticks(32)));
    }

    #[test]
    fn reset_returns_both_blocks_to_documented_reset_values() {
        let (mut device, _handle) = WchWatchdog::new_iwdg("iwdg");
        device
            .write(0x00, AccessWidth::Word, 0x5555, SimTime::ZERO)
            .unwrap();
        device
            .write(0x08, AccessWidth::Word, 7, SimTime::ZERO)
            .unwrap();
        device.reset(ResetKind::PowerOn);
        assert_eq!(
            device.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0
        );
        assert_eq!(
            device.read(0x08, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x0fff
        );
    }
}
