use super::*;
use remu_signals::{SignalId, SignalValue};

const CR1: u64 = 0x00;
const CR2: u64 = 0x04;
const DIER: u64 = 0x0c;
const SR: u64 = 0x10;
const EGR: u64 = 0x14;
const CCMR1: u64 = 0x18;
const CCER: u64 = 0x20;
const CNT: u64 = 0x24;
const PSC: u64 = 0x28;
const ARR: u64 = 0x2c;
const CCR1: u64 = 0x34;
const CR1_CEN: u32 = 1;
const CR1_UDIS: u32 = 1 << 1;
const CR1_URS: u32 = 1 << 2;
const CR1_OPM: u32 = 1 << 3;
const CR1_ARPE: u32 = 1 << 7;
const CR1_MASK: u32 = CR1_CEN | CR1_UDIS | CR1_URS | CR1_OPM | CR1_ARPE;
const DIER_UIE: u32 = 1;
const DIER_CC1IE: u32 = 1 << 1;
const SR_UIF: u32 = 1;
const SR_CC1IF: u32 = 1 << 1;
const EGR_UG: u32 = 1;
const CCER_CC1E: u32 = 1;
const CCER_CC1P: u32 = 1 << 1;

#[derive(Default)]
struct Tim16State {
    cr1: u32,
    cr2: u32,
    dier: u32,
    sr: u32,
    ccmr1: u32,
    ccer: u32,
    cnt: u16,
    psc: u16,
    arr: u16,
    ccr1: u16,
    started: u64,
}

/// Host-facing STM32 TIM16 timing and channel-one handle.
#[derive(Clone)]
pub struct Stm32Tim16Handle {
    state: Arc<Mutex<Tim16State>>,
    hub: SignalHub,
    interrupt_signal: SignalId,
    output_signal: SignalId,
}

impl Stm32Tim16Handle {
    /// Advances TIM16 and returns an enabled update/compare interrupt level.
    pub fn poll(&self, now: SimTime) -> bool {
        let (pending, output) = {
            let mut state = self.state.lock().expect("STM32 TIM16 lock poisoned");
            if state.cr1 & CR1_CEN != 0 {
                let prescaler = u64::from(state.psc) + 1;
                let elapsed = now.ticks().saturating_sub(state.started) / prescaler;
                if elapsed != 0 {
                    state.started = state
                        .started
                        .saturating_add(elapsed.saturating_mul(prescaler));
                    let period = u64::from(state.arr) + 1;
                    let old = u64::from(state.cnt);
                    let total = old.saturating_add(elapsed);
                    let updates = total / period;
                    state.cnt = (total % period) as u16;
                    if updates != 0 && state.cr1 & CR1_UDIS == 0 {
                        state.sr |= SR_UIF;
                        if state.cr1 & CR1_OPM != 0 {
                            state.cr1 &= !CR1_CEN;
                        }
                    }
                    let compare = u64::from(state.ccr1);
                    if state.ccer & CCER_CC1E != 0
                        && compare <= u64::from(state.arr)
                        && (updates != 0 || (old < compare && old + elapsed >= compare))
                    {
                        state.sr |= SR_CC1IF;
                    }
                }
            }
            (
                (state.sr & SR_UIF != 0 && state.dier & DIER_UIE != 0)
                    || (state.sr & SR_CC1IF != 0 && state.dier & DIER_CC1IE != 0),
                Self::channel_output(&state),
            )
        };
        self.publish(pending, output, now);
        pending
    }

    /// Current TIM16 counter value.
    pub fn counter(&self) -> u16 {
        self.state.lock().expect("STM32 TIM16 lock poisoned").cnt
    }

    /// Current functional channel-one output level.
    pub fn channel_one_output(&self) -> bool {
        let state = self.state.lock().expect("STM32 TIM16 lock poisoned");
        Self::channel_output(&state)
    }

    fn channel_output(state: &Tim16State) -> bool {
        if state.ccer & CCER_CC1E == 0 {
            return false;
        }
        let mode = (state.ccmr1 >> 4) & 0x7;
        let active = match mode {
            6 => state.cnt < state.ccr1,
            7 => state.cnt >= state.ccr1,
            _ => false,
        };
        active ^ (state.ccer & CCER_CC1P != 0)
    }

    fn publish(&self, pending: bool, output: bool, at: SimTime) {
        self.hub
            .set(
                self.interrupt_signal,
                SignalValue::from_u64(u64::from(pending), 1)
                    .expect("TIM16 interrupt signal width is valid"),
                at,
            )
            .expect("TIM16 interrupt signal is declared");
        self.hub
            .set(
                self.output_signal,
                SignalValue::from_u64(u64::from(output), 1)
                    .expect("TIM16 output signal width is valid"),
                at,
            )
            .expect("TIM16 output signal is declared");
    }
}

/// Functional STM32L432 TIM16 counter/update and channel-one PWM slice.
pub struct Stm32Tim16 {
    name: String,
    state: Arc<Mutex<Tim16State>>,
    handle: Stm32Tim16Handle,
}

impl Stm32Tim16 {
    /// Creates a disabled TIM16 with a maximal auto-reload value.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Stm32Tim16Handle), remu_signals::SignalError> {
        let name = name.into();
        let interrupt_signal = hub.declare(
            format!("{name}.irq"),
            SignalValue::from_u64(0, 1)?,
            Some("TIM16 update/compare interrupt request".to_owned()),
        )?;
        let output_signal = hub.declare(
            format!("{name}.ch1"),
            SignalValue::from_u64(0, 1)?,
            Some("TIM16 channel-one functional output".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(Tim16State {
            arr: u16::MAX,
            ..Tim16State::default()
        }));
        let handle = Stm32Tim16Handle {
            state: state.clone(),
            hub,
            interrupt_signal,
            output_signal,
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
        let state = self.state.lock().expect("STM32 TIM16 lock poisoned");
        match offset {
            CR1 => state.cr1,
            CR2 => state.cr2,
            DIER => state.dier,
            SR => state.sr,
            CCMR1 => state.ccmr1,
            CCER => state.ccer,
            CNT => {
                let elapsed = at.ticks().saturating_sub(state.started) / (u64::from(state.psc) + 1);
                let period = u64::from(state.arr) + 1;
                ((u64::from(state.cnt) + elapsed) % period) as u32
            }
            PSC => u32::from(state.psc),
            ARR => u32::from(state.arr),
            CCR1 => u32::from(state.ccr1),
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u32, at: SimTime) {
        let (pending, output) = {
            let mut state = self.state.lock().expect("STM32 TIM16 lock poisoned");
            match offset {
                CR1 => {
                    state.cr1 = value & CR1_MASK;
                    state.started = at.ticks();
                }
                CR2 => state.cr2 = value,
                DIER => state.dier = value & (DIER_UIE | DIER_CC1IE),
                SR => state.sr &= value & (SR_UIF | SR_CC1IF),
                EGR if value & EGR_UG != 0 => {
                    state.cnt = 0;
                    state.started = at.ticks();
                    if state.cr1 & CR1_UDIS == 0 && state.cr1 & CR1_URS == 0 {
                        state.sr |= SR_UIF;
                    }
                }
                CCMR1 => state.ccmr1 = value,
                CCER => state.ccer = value & (CCER_CC1E | CCER_CC1P),
                CNT => {
                    state.cnt = value as u16;
                    state.started = at.ticks();
                }
                PSC => {
                    state.psc = value as u16;
                    state.started = at.ticks();
                }
                ARR => state.arr = value as u16,
                CCR1 => state.ccr1 = value as u16,
                _ => {}
            }
            (
                (state.sr & SR_UIF != 0 && state.dier & DIER_UIE != 0)
                    || (state.sr & SR_CC1IF != 0 && state.dier & DIER_CC1IE != 0),
                Stm32Tim16Handle::channel_output(&state),
            )
        };
        self.handle.publish(pending, output, at);
    }
}

impl Device for Stm32Tim16 {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 TIM16 requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 TIM16 access at {offset:#x}"
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
            return Err(DeviceError::new("STM32 TIM16 requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 TIM16 access at {offset:#x}"
            )));
        }
        self.write_register(offset, value as u32, at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32 TIM16 lock poisoned") = Tim16State {
            arr: u16::MAX,
            ..Tim16State::default()
        };
        self.handle.publish(false, false, SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tim16_update_compare_and_pwm_output_are_observable() {
        let hub = SignalHub::new();
        let (mut timer, handle) = Stm32Tim16::new("tim16", hub).unwrap();
        timer
            .write(PSC, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        timer
            .write(ARR, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        timer
            .write(CCR1, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        timer
            .write(CCMR1, AccessWidth::Word, 6 << 4, SimTime::ZERO)
            .unwrap();
        timer
            .write(CCER, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        timer
            .write(DIER, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        timer
            .write(CR1, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert!(handle.channel_one_output());
        assert!(handle.poll(SimTime::from_ticks(4)));
        assert!(!handle.channel_one_output());
        assert_eq!(timer.read(SR, AccessWidth::Word, SimTime::ZERO), Ok(2));
        assert!(handle.poll(SimTime::from_ticks(8)));
        timer
            .write(SR, AccessWidth::Word, 0, SimTime::from_ticks(8))
            .unwrap();
        assert_eq!(timer.read(SR, AccessWidth::Word, SimTime::ZERO), Ok(0));
    }
}
