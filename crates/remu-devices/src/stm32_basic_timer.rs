use super::*;
use remu_signals::{SignalId, SignalValue};

const CR1: u64 = 0x00;
const CR2: u64 = 0x04;
const DIER: u64 = 0x0c;
const SR: u64 = 0x10;
const EGR: u64 = 0x14;
const CNT: u64 = 0x24;
const PSC: u64 = 0x28;
const ARR: u64 = 0x2c;

const CR1_CEN: u32 = 1 << 0;
const CR1_UDIS: u32 = 1 << 1;
const CR1_URS: u32 = 1 << 2;
const CR1_OPM: u32 = 1 << 3;
const CR1_ARPE: u32 = 1 << 7;
const DIER_UIE: u32 = 1 << 0;
const SR_UIF: u32 = 1 << 0;
const EGR_UG: u32 = 1 << 0;
const CR1_MASK: u32 = CR1_CEN | CR1_UDIS | CR1_URS | CR1_OPM | CR1_ARPE;

#[derive(Default)]
struct BasicTimerState {
    cr1: u32,
    cr2: u32,
    dier: u32,
    sr: u32,
    cnt: u16,
    psc: u16,
    arr: u16,
    started: u64,
}

/// Host-facing STM32 basic-timer timing handle.
#[derive(Clone)]
pub struct Stm32BasicTimerHandle {
    state: Arc<Mutex<BasicTimerState>>,
    hub: SignalHub,
    interrupt_signal: SignalId,
}

impl Stm32BasicTimerHandle {
    /// Advances the timer and returns whether an enabled update interrupt is pending.
    pub fn poll(&self, now: SimTime) -> bool {
        let pending = {
            let mut state = self.state.lock().expect("STM32 basic timer lock poisoned");
            if state.cr1 & CR1_CEN != 0 {
                let prescaler = u64::from(state.psc) + 1;
                let elapsed_ticks = now.ticks().saturating_sub(state.started);
                let elapsed_counts = elapsed_ticks / prescaler;
                if elapsed_counts != 0 {
                    state.started = state
                        .started
                        .saturating_add(elapsed_counts.saturating_mul(prescaler));
                    let period = u64::from(state.arr) + 1;
                    let total = u64::from(state.cnt).saturating_add(elapsed_counts);
                    let updates = total / period;
                    state.cnt = (total % period) as u16;
                    if updates != 0 && state.cr1 & CR1_UDIS == 0 {
                        state.sr |= SR_UIF;
                        if state.cr1 & CR1_OPM != 0 {
                            state.cr1 &= !CR1_CEN;
                        }
                    }
                }
            }
            state.sr & SR_UIF != 0 && state.dier & DIER_UIE != 0
        };
        self.publish(pending, now);
        pending
    }

    /// Current timer counter value.
    pub fn counter(&self) -> u16 {
        self.state
            .lock()
            .expect("STM32 basic timer lock poisoned")
            .cnt
    }

    fn publish(&self, pending: bool, at: SimTime) {
        self.hub
            .set(
                self.interrupt_signal,
                SignalValue::from_u64(u64::from(pending), 1)
                    .expect("STM32 basic timer signal width is valid"),
                at,
            )
            .expect("STM32 basic timer signal is declared");
    }
}

/// Functional STM32L432 basic timer slice, used by TIM6 and TIM7.
pub struct Stm32BasicTimer {
    name: String,
    state: Arc<Mutex<BasicTimerState>>,
    handle: Stm32BasicTimerHandle,
}

impl Stm32BasicTimer {
    /// Creates a disabled basic timer with a maximal auto-reload value.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Stm32BasicTimerHandle), remu_signals::SignalError> {
        let name = name.into();
        let interrupt_signal = hub.declare(
            format!("{name}.irq"),
            SignalValue::from_u64(0, 1)?,
            Some("basic timer update interrupt request".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(BasicTimerState {
            arr: u16::MAX,
            ..BasicTimerState::default()
        }));
        let handle = Stm32BasicTimerHandle {
            state: state.clone(),
            hub,
            interrupt_signal,
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

    fn read_register(&self, offset: u64, at: SimTime) -> u32 {
        let state = self.state.lock().expect("STM32 basic timer lock poisoned");
        match offset {
            CR1 => state.cr1,
            CR2 => state.cr2,
            DIER => state.dier,
            SR => state.sr,
            CNT => {
                if state.cr1 & CR1_CEN == 0 {
                    u32::from(state.cnt)
                } else {
                    let elapsed_counts =
                        at.ticks().saturating_sub(state.started) / (u64::from(state.psc) + 1);
                    let period = u64::from(state.arr) + 1;
                    ((u64::from(state.cnt) + elapsed_counts) % period) as u32
                }
            }
            PSC => u32::from(state.psc),
            ARR => u32::from(state.arr),
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u32, at: SimTime) {
        let pending = {
            let mut state = self.state.lock().expect("STM32 basic timer lock poisoned");
            match offset {
                CR1 => {
                    state.cr1 = value & CR1_MASK;
                    state.started = at.ticks();
                }
                CR2 => state.cr2 = value,
                DIER => state.dier = value & DIER_UIE,
                SR => state.sr &= value & SR_UIF,
                EGR if value & EGR_UG != 0 => {
                    state.cnt = 0;
                    state.started = at.ticks();
                    if state.cr1 & CR1_UDIS == 0 && state.cr1 & CR1_URS == 0 {
                        state.sr |= SR_UIF;
                    }
                }
                CNT => {
                    state.cnt = value as u16;
                    state.started = at.ticks();
                }
                PSC => {
                    state.psc = value as u16;
                    state.started = at.ticks();
                }
                ARR => state.arr = value as u16,
                _ => {}
            }
            state.sr & SR_UIF != 0 && state.dier & DIER_UIE != 0
        };
        self.handle.publish(pending, at);
    }
}

impl Device for Stm32BasicTimer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 basic timer requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 basic timer access at {offset:#x}"
            )));
        }
        Ok(u64::from(self.read_register(offset, at)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 basic timer requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 basic timer access at {offset:#x}"
            )));
        }
        self.write_register(offset, value as u32, at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32 basic timer lock poisoned") = BasicTimerState {
            arr: u16::MAX,
            ..BasicTimerState::default()
        };
        self.handle.publish(false, SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_interrupt_latches_and_can_be_cleared() {
        let hub = SignalHub::new();
        let (mut timer, handle) = Stm32BasicTimer::new("tim6", hub).unwrap();
        timer
            .write(PSC, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        timer
            .write(ARR, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        timer
            .write(DIER, AccessWidth::Word, DIER_UIE.into(), SimTime::ZERO)
            .unwrap();
        timer
            .write(CR1, AccessWidth::Word, CR1_CEN.into(), SimTime::ZERO)
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(7)));
        assert!(handle.poll(SimTime::from_ticks(8)));
        assert_eq!(timer.read(SR, AccessWidth::Word, SimTime::ZERO), Ok(1));
        timer
            .write(SR, AccessWidth::Word, 0, SimTime::from_ticks(8))
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(8)));
    }

    #[test]
    fn update_event_resets_counter_and_reports_the_counter_value() {
        let hub = SignalHub::new();
        let (mut timer, handle) = Stm32BasicTimer::new("tim6", hub).unwrap();
        timer
            .write(ARR, AccessWidth::Word, 9, SimTime::ZERO)
            .unwrap();
        timer
            .write(EGR, AccessWidth::Word, EGR_UG.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.counter(), 0);
        assert_eq!(timer.read(CNT, AccessWidth::Word, SimTime::ZERO), Ok(0));
    }
}
