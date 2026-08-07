use super::*;

const CR: u64 = 0x00;
const CFR: u64 = 0x04;
const SR: u64 = 0x08;
const CR_WDGA: u32 = 1 << 7;
const CFR_W: u32 = 0x7f;
const CFR_WDGTB_SHIFT: u32 = 7;
const CFR_WDGTB_MASK: u32 = 0x3 << CFR_WDGTB_SHIFT;
const CFR_EWI: u32 = 1 << 9;
const SR_EWIF: u32 = 1;
const WWDG_TICK_DIVISOR: u64 = 16;

#[derive(Default)]
struct WwdgState {
    cr: u32,
    cfr: u32,
    sr: u32,
    counter: u8,
    enabled: bool,
    reset_requested: bool,
    started: u64,
}

/// Host-facing STM32 window-watchdog timing handle.
#[derive(Clone)]
pub struct Stm32WwdgHandle {
    state: Arc<Mutex<WwdgState>>,
    hub: SignalHub,
    interrupt_signal: SignalId,
    reset_signal: SignalId,
}

impl Stm32WwdgHandle {
    /// Advances the abstract watchdog and returns `(early_interrupt, reset)`.
    pub fn poll(&self, now: SimTime) -> (bool, bool) {
        let (early, reset) = {
            let mut state = self.state.lock().expect("STM32 WWDG lock poisoned");
            if state.enabled && !state.reset_requested {
                let prescaler = 1_u64 << ((state.cfr & CFR_WDGTB_MASK) >> CFR_WDGTB_SHIFT);
                let divisor = WWDG_TICK_DIVISOR.saturating_mul(prescaler);
                let elapsed = now.ticks().saturating_sub(state.started) / divisor;
                if elapsed != 0 {
                    state.started = state
                        .started
                        .saturating_add(elapsed.saturating_mul(divisor));
                    let old = u64::from(state.counter);
                    if old > 0x3f && elapsed >= old.saturating_sub(0x40) && state.cfr & CFR_EWI != 0
                    {
                        state.sr |= SR_EWIF;
                    }
                    let next = old.saturating_sub(elapsed);
                    if next <= 0x3f {
                        state.reset_requested = true;
                    } else {
                        state.counter = next as u8;
                        state.cr = u32::from(state.counter) | CR_WDGA;
                    }
                }
            }
            (
                state.sr & SR_EWIF != 0 && state.cfr & CFR_EWI != 0,
                state.reset_requested,
            )
        };
        self.publish(early, reset, now);
        (early, reset)
    }

    /// Current seven-bit watchdog counter.
    pub fn counter(&self) -> u8 {
        self.state.lock().expect("STM32 WWDG lock poisoned").counter
    }

    fn publish(&self, interrupt: bool, reset: bool, at: SimTime) {
        self.hub
            .set(
                self.interrupt_signal,
                SignalValue::from_u64(u64::from(interrupt), 1)
                    .expect("WWDG interrupt signal width is valid"),
                at,
            )
            .expect("WWDG interrupt signal is declared");
        self.hub
            .set(
                self.reset_signal,
                SignalValue::from_u64(u64::from(reset), 1)
                    .expect("WWDG reset signal width is valid"),
                at,
            )
            .expect("WWDG reset signal is declared");
    }
}

/// Functional STM32L432KC window watchdog slice.
///
/// The model implements the native CR/CFR/SR contract, seven-bit down-counting,
/// configurable abstract prescaling, refresh-window validation, early-wakeup
/// interrupt, reset request, and VCD-visible interrupt/reset signals. The
/// abstract tick divisor is intentionally deterministic rather than a physical
/// PCLK1 frequency model.
pub struct Stm32Wwdg {
    name: String,
    state: Arc<Mutex<WwdgState>>,
    handle: Stm32WwdgHandle,
}

impl Stm32Wwdg {
    /// Creates a disabled watchdog with the reset counter value.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Stm32WwdgHandle), remu_signals::SignalError> {
        let name = name.into();
        let interrupt_signal = hub.declare(
            format!("{name}.irq"),
            SignalValue::from_u64(0, 1)?,
            Some("WWDG early-wakeup interrupt request".to_owned()),
        )?;
        let reset_signal = hub.declare(
            format!("{name}.reset"),
            SignalValue::from_u64(0, 1)?,
            Some("WWDG reset request".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(WwdgState {
            counter: 0x7f,
            cr: 0x7f,
            ..WwdgState::default()
        }));
        let handle = Stm32WwdgHandle {
            state: state.clone(),
            hub,
            interrupt_signal,
            reset_signal,
        };
        Ok((
            Self {
                name,
                state,
                handle: handle.clone(),
            },
            handle,
        ))
    }

    fn read_register(&self, offset: u64) -> u32 {
        let state = self.state.lock().expect("STM32 WWDG lock poisoned");
        match offset {
            CR => u32::from(state.counter) | u32::from(state.enabled) << 7,
            CFR => state.cfr,
            SR => state.sr,
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u32, at: SimTime) {
        let mut state = self.state.lock().expect("STM32 WWDG lock poisoned");
        match offset {
            CR => {
                let requested = (value & 0x7f) as u8;
                if state.enabled {
                    let window = (state.cfr & CFR_W) as u8;
                    if state.counter > window {
                        state.reset_requested = true;
                    } else {
                        state.counter = requested.max(0x40);
                        state.started = at.ticks();
                    }
                } else if value & CR_WDGA != 0 {
                    state.enabled = true;
                    state.counter = requested.max(0x40);
                    state.started = at.ticks();
                }
                state.cr = u32::from(state.counter) | u32::from(state.enabled) << 7;
            }
            CFR => state.cfr = value & (CFR_W | CFR_WDGTB_MASK | CFR_EWI),
            SR if value & SR_EWIF == 0 => state.sr &= !SR_EWIF,
            _ => {}
        }
    }
}

impl Device for Stm32Wwdg {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 WWDG requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 WWDG access at {offset:#x}"
            )));
        }
        Ok(u64::from(self.read_register(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 WWDG requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 WWDG access at {offset:#x}"
            )));
        }
        self.write_register(offset, value as u32, at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32 WWDG lock poisoned") = WwdgState {
            counter: 0x7f,
            cr: 0x7f,
            ..WwdgState::default()
        };
        self.handle.poll(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn early_wakeup_precedes_window_watchdog_reset() {
        let hub = SignalHub::new();
        let (mut wwdg, handle) = Stm32Wwdg::new("board.stm32l432kc.wwdg", hub.clone()).unwrap();
        wwdg.write(
            CFR,
            AccessWidth::Word,
            u64::from(CFR_EWI | 0x60),
            SimTime::ZERO,
        )
        .unwrap();
        wwdg.write(
            CR,
            AccessWidth::Word,
            u64::from(CR_WDGA | 0x45),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            handle.poll(SimTime::from_ticks(WWDG_TICK_DIVISOR * 5)),
            (true, false)
        );
        assert_eq!(wwdg.read(SR, AccessWidth::Word, SimTime::ZERO), Ok(1));
        assert_eq!(
            handle.poll(SimTime::from_ticks(WWDG_TICK_DIVISOR * 6)),
            (true, true)
        );
        let reset = hub
            .with_registry(|registry| registry.find("board.stm32l432kc.wwdg.reset"))
            .unwrap();
        assert_eq!(
            hub.with_registry(|registry| registry.value(reset).unwrap().to_vcd_binary()),
            "1"
        );
    }

    #[test]
    fn refresh_is_only_accepted_inside_the_configured_window() {
        let hub = SignalHub::new();
        let (mut wwdg, handle) = Stm32Wwdg::new("wwdg", hub).unwrap();
        wwdg.write(CFR, AccessWidth::Word, 0x60, SimTime::ZERO)
            .unwrap();
        wwdg.write(
            CR,
            AccessWidth::Word,
            u64::from(CR_WDGA | 0x50),
            SimTime::ZERO,
        )
        .unwrap();
        handle.poll(SimTime::from_ticks(WWDG_TICK_DIVISOR * 4));
        wwdg.write(
            CR,
            AccessWidth::Word,
            u64::from(CR_WDGA | 0x50),
            SimTime::from_ticks(WWDG_TICK_DIVISOR * 4),
        )
        .unwrap();
        assert_eq!(handle.counter(), 0x50);
    }
}
