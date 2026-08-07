use super::*;
use std::collections::VecDeque;

/// Action emitted when an ESP watchdog stage expires.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EspWatchdogAction {
    /// Raise the watchdog interrupt and continue to the next stage.
    Interrupt,
    /// Reset the currently executing CPU core.
    ResetCpu,
    /// Reset the HP system while retaining the always-on domain.
    ResetSystem,
    /// Reset the HP system and always-on/LP domain.
    ResetRtc,
}

/// Host-facing state for the ESP32-C6 LP watchdog.
#[derive(Clone)]
pub struct EspLpWatchdogHandle {
    state: Rc<RefCell<EspLpWatchdogState>>,
}

impl EspLpWatchdogHandle {
    /// Returns whether a watchdog interrupt is pending at the supplied time.
    pub fn interrupt_pending(&self, now: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        state.advance(now);
        state.registers[EspLpWatchdogState::INT_RAW / 4] & (1 << 31) != 0
    }

    /// Consumes the next expired-stage action at the supplied time.
    pub fn take_action(&self, now: SimTime) -> Option<EspWatchdogAction> {
        let mut state = self.state.borrow_mut();
        state.advance(now);
        state.actions.pop_front()
    }
}

struct EspLpWatchdogState {
    registers: Vec<u32>,
    epoch: SimTime,
    stage: usize,
    actions: VecDeque<EspWatchdogAction>,
    write_enabled: bool,
}

impl EspLpWatchdogState {
    const CONFIG0: usize = 0x00;
    const STAGE0_HOLD: usize = 0x04;
    const FEED: usize = 0x14;
    const WPROTECT: usize = 0x18;
    const INT_RAW: usize = 0x24;
    const INT_STATUS: usize = 0x28;
    const INT_ENABLE: usize = 0x2c;
    const INT_CLEAR: usize = 0x30;
    const DATE: usize = 0x3fc;
    const ENABLE: u32 = 1 << 31;
    const LP_INT: u32 = 1 << 31;

    fn new() -> Self {
        let mut registers = vec![0; 0x400 / 4];
        // Reset values from Espressif's ESP32-C6 lp_wdt_reg.h.
        registers[Self::CONFIG0 / 4] = (1 << 9) | (1 << 12) | (1 << 16);
        registers[Self::STAGE0_HOLD / 4] = 200_000;
        registers[0x08 / 4] = 80_000;
        registers[0x0c / 4] = 4095;
        registers[0x10 / 4] = 4095;
        registers[0x1c / 4] = 300 << 20;
        registers[Self::DATE / 4] = 34_676_864;
        Self {
            registers,
            epoch: SimTime::ZERO,
            stage: 0,
            actions: VecDeque::new(),
            write_enabled: false,
        }
    }

    fn advance(&mut self, now: SimTime) {
        if self.registers[Self::CONFIG0 / 4] & Self::ENABLE == 0 {
            return;
        }
        while self.stage < 4 {
            let hold = u64::from(self.registers[(Self::STAGE0_HOLD / 4) + self.stage]).max(1);
            if now.ticks().saturating_sub(self.epoch.ticks()) < hold {
                break;
            }
            self.epoch = SimTime::from_ticks(self.epoch.ticks().saturating_add(hold));
            let shift = [28, 25, 22, 19][self.stage];
            let action = (self.registers[Self::CONFIG0 / 4] >> shift) & 0x7;
            self.stage += 1;
            match action {
                1 => {
                    self.registers[Self::INT_RAW / 4] |= Self::LP_INT;
                    self.actions.push_back(EspWatchdogAction::Interrupt);
                }
                2 => self.actions.push_back(EspWatchdogAction::ResetCpu),
                3 => self.actions.push_back(EspWatchdogAction::ResetSystem),
                4 => self.actions.push_back(EspWatchdogAction::ResetRtc),
                _ => {}
            }
            if action >= 2 {
                break;
            }
        }
    }

    fn interrupt_status(&self) -> u32 {
        self.registers[Self::INT_RAW / 4] & self.registers[Self::INT_ENABLE / 4]
    }
}

/// Functional ESP32-C6 low-power watchdog/reset slice.
///
/// Native configuration, stage-0 hold, feed, and raw/status/enable/clear
/// registers are retained. The abstract timeline advances stage 0 without
/// claiming RTC-clock accuracy. Stage-0 interrupt actions become visible
/// through raw/status bits; CPU/system reset actions are consumed by the
/// machine as a deterministic watchdog reset. LP-domain power sequencing and
/// later watchdog stages remain outside this slice.
pub struct EspLpWatchdog {
    name: String,
    state: Rc<RefCell<EspLpWatchdogState>>,
}

impl EspLpWatchdog {
    /// Creates a reset LP watchdog and host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, EspLpWatchdogHandle) {
        let state = Rc::new(RefCell::new(EspLpWatchdogState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspLpWatchdogHandle { state },
        )
    }
}

impl Device for EspLpWatchdog {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP LP watchdog requires aligned word access",
            ));
        }
        let offset = usize::try_from(offset).expect("LP watchdog offset fits");
        let mut state = self.state.borrow_mut();
        state.advance(at);
        let value =
            match offset {
                EspLpWatchdogState::INT_STATUS => state.interrupt_status(),
                EspLpWatchdogState::INT_CLEAR
                | EspLpWatchdogState::FEED
                | EspLpWatchdogState::WPROTECT => 0,
                _ => state.registers.get(offset / 4).copied().ok_or_else(|| {
                    DeviceError::new(format!("{} read at {offset:#x}", self.name))
                })?,
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
            return Err(DeviceError::new(
                "ESP LP watchdog requires aligned word access",
            ));
        }
        let offset = usize::try_from(offset).expect("LP watchdog offset fits");
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits");
        let mut state = self.state.borrow_mut();
        if offset == EspLpWatchdogState::WPROTECT {
            state.write_enabled = value == 0x50d8_3aa1;
            return Ok(());
        }
        if !state.write_enabled {
            return Ok(());
        }
        match offset {
            EspLpWatchdogState::FEED => {
                state.epoch = at;
                state.stage = 0;
                state.actions.clear();
            }
            EspLpWatchdogState::INT_CLEAR => {
                state.registers[EspLpWatchdogState::INT_RAW / 4] &= !value;
            }
            EspLpWatchdogState::INT_STATUS => {}
            _ => {
                let register = state.registers.get_mut(offset / 4).ok_or_else(|| {
                    DeviceError::new(format!("{} write at {offset:#x}", self.name))
                })?;
                *register = value;
                if offset == EspLpWatchdogState::CONFIG0 && value & EspLpWatchdogState::ENABLE != 0
                {
                    state.epoch = at;
                    state.stage = 0;
                }
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = EspLpWatchdogState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_four_lp_watchdog_stages_advance_in_order() {
        let (mut watchdog, handle) = EspLpWatchdog::new("wdt");
        watchdog
            .write(0x18, AccessWidth::Word, 0x50d8_3aa1, SimTime::ZERO)
            .unwrap();
        for offset in [0x04, 0x08, 0x0c, 0x10] {
            watchdog
                .write(offset, AccessWidth::Word, 2, SimTime::ZERO)
                .unwrap();
        }
        let config = (1 << 31) | (1 << 28) | (2 << 25) | (3 << 22) | (4 << 19);
        watchdog
            .write(0, AccessWidth::Word, config, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            handle.take_action(SimTime::from_ticks(2)),
            Some(EspWatchdogAction::Interrupt)
        );
        assert_eq!(
            handle.take_action(SimTime::from_ticks(4)),
            Some(EspWatchdogAction::ResetCpu)
        );
        watchdog
            .write(0x14, AccessWidth::Word, 1, SimTime::from_ticks(4))
            .unwrap();
        assert_eq!(handle.take_action(SimTime::from_ticks(5)), None);
    }
}
