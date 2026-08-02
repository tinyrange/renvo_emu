use super::*;

const CR1: u64 = 0x00;
const CR2: u64 = 0x04;
const SMCR: u64 = 0x08;
const DIER: u64 = 0x0c;
const SR: u64 = 0x10;
const EGR: u64 = 0x14;
const CCMR1: u64 = 0x18;
const CCMR2: u64 = 0x1c;
const CCER: u64 = 0x20;
const CNT: u64 = 0x24;
const PSC: u64 = 0x28;
const ARR: u64 = 0x2c;
const RCR: u64 = 0x30;
const CCR1: u64 = 0x34;
const CCR4: u64 = 0x40;
const BDTR: u64 = 0x44;
const DCR: u64 = 0x48;
const DMAR: u64 = 0x4c;

const CR1_CEN: u32 = 1 << 0;
const DIER_UIE: u32 = 1 << 0;
const SR_UIF: u32 = 1 << 0;
const EGR_UG: u32 = 1 << 0;
const BDTR_MOE: u32 = 1 << 15;

#[derive(Default)]
struct TimerState {
    cr1: u32,
    cr2: u32,
    smcr: u32,
    dier: u32,
    sr: u32,
    ccmr1: u32,
    ccmr2: u32,
    ccer: u32,
    cnt: u16,
    psc: u16,
    arr: u16,
    rcr: u16,
    ccr: [u16; 4],
    bdtr: u32,
    dcr: u32,
    dmar: u32,
    started: u64,
}

/// Handle for the functional TIM1 timer and PWM outputs.
#[derive(Clone)]
pub struct Stm32AdvancedTimerHandle {
    state: Arc<Mutex<TimerState>>,
    hub: SignalHub,
    channel_signals: [SignalId; 4],
    complementary_signals: [SignalId; 3],
    interrupt_signal: SignalId,
}

impl Stm32AdvancedTimerHandle {
    /// Advances TIM1 and returns whether an enabled update interrupt is pending.
    pub fn poll(&self, now: SimTime) -> bool {
        let (channels, complementary, interrupt) = {
            let mut state = self.state.lock().expect("TIM1 lock poisoned");
            let period = u64::from(state.arr).saturating_add(1);
            let prescaler = u64::from(state.psc).saturating_add(1);
            let elapsed = now.ticks().saturating_sub(state.started);
            let counter = if state.cr1 & CR1_CEN != 0 && period != 0 {
                let ticks = elapsed / prescaler;
                let remainder = ticks % period;
                if ticks >= period {
                    state.sr |= SR_UIF;
                    state.started = now
                        .ticks()
                        .saturating_sub(remainder.saturating_mul(prescaler));
                }
                remainder as u16
            } else {
                state.cnt
            };
            state.cnt = counter;
            let modes = [
                (state.ccmr1 >> 4) & 0x7,
                (state.ccmr1 >> 12) & 0x7,
                (state.ccmr2 >> 4) & 0x7,
                (state.ccmr2 >> 12) & 0x7,
            ];
            let main = core::array::from_fn(|channel| {
                let enabled = state.ccer & (1 << (channel * 4)) != 0;
                let pwm = modes[channel] == 6 || modes[channel] == 7;
                enabled && state.bdtr & BDTR_MOE != 0 && pwm && state.cnt < state.ccr[channel]
            });
            let complementary = core::array::from_fn(|channel| {
                let enabled = state.ccer & (1 << (channel * 4 + 2)) != 0;
                enabled
                    && state.bdtr & BDTR_MOE != 0
                    && (modes[channel] == 6 || modes[channel] == 7)
                    && !main[channel]
            });
            let interrupt = state.sr & SR_UIF != 0 && state.dier & DIER_UIE != 0;
            (main, complementary, interrupt)
        };
        self.publish(channels, complementary, interrupt, now);
        interrupt
    }

    /// Returns the current timer counter.
    pub fn counter(&self) -> u16 {
        self.state.lock().expect("TIM1 lock poisoned").cnt
    }

    fn publish(&self, channels: [bool; 4], complementary: [bool; 3], interrupt: bool, at: SimTime) {
        for (signal, value) in self.channel_signals.into_iter().zip(channels) {
            self.hub
                .set(
                    signal,
                    SignalValue::from_u64(u64::from(value), 1)
                        .expect("TIM1 channel signal width is valid"),
                    at,
                )
                .expect("TIM1 channel signal is declared");
        }
        for (signal, value) in self.complementary_signals.into_iter().zip(complementary) {
            self.hub
                .set(
                    signal,
                    SignalValue::from_u64(u64::from(value), 1)
                        .expect("TIM1 complementary signal width is valid"),
                    at,
                )
                .expect("TIM1 complementary signal is declared");
        }
        self.hub
            .set(
                self.interrupt_signal,
                SignalValue::from_u64(u64::from(interrupt), 1)
                    .expect("TIM1 interrupt signal width is valid"),
                at,
            )
            .expect("TIM1 interrupt signal is declared");
    }
}

/// Functional STM32L432KC TIM1 advanced-control timer slice.
///
/// The model covers the native time-base, update interrupt, four PWM compare
/// channels, three complementary outputs, main-output enable, and update
/// generation. It is intentionally not a cycle-accurate gate/dead-time,
/// capture, break, DMA, or external-pin alternate-function model.
pub struct Stm32AdvancedTimer {
    name: String,
    state: Arc<Mutex<TimerState>>,
    handle: Stm32AdvancedTimerHandle,
}

impl Stm32AdvancedTimer {
    /// Creates TIM1 and its trace-visible control handle.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Stm32AdvancedTimerHandle), remu_signals::SignalError> {
        let name = name.into();
        let channel_signals = [
            hub.declare(
                format!("{name}.ch1"),
                SignalValue::from_u64(0, 1)?,
                Some("TIM1 channel 1 PWM output".to_owned()),
            )?,
            hub.declare(
                format!("{name}.ch2"),
                SignalValue::from_u64(0, 1)?,
                Some("TIM1 channel 2 PWM output".to_owned()),
            )?,
            hub.declare(
                format!("{name}.ch3"),
                SignalValue::from_u64(0, 1)?,
                Some("TIM1 channel 3 PWM output".to_owned()),
            )?,
            hub.declare(
                format!("{name}.ch4"),
                SignalValue::from_u64(0, 1)?,
                Some("TIM1 channel 4 PWM output".to_owned()),
            )?,
        ];
        let complementary_signals = [
            hub.declare(
                format!("{name}.ch1n"),
                SignalValue::from_u64(0, 1)?,
                Some("TIM1 channel 1 complementary PWM output".to_owned()),
            )?,
            hub.declare(
                format!("{name}.ch2n"),
                SignalValue::from_u64(0, 1)?,
                Some("TIM1 channel 2 complementary PWM output".to_owned()),
            )?,
            hub.declare(
                format!("{name}.ch3n"),
                SignalValue::from_u64(0, 1)?,
                Some("TIM1 channel 3 complementary PWM output".to_owned()),
            )?,
        ];
        let interrupt_signal = hub.declare(
            format!("{name}.irq"),
            SignalValue::from_u64(0, 1)?,
            Some("TIM1 update interrupt request".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(TimerState {
            arr: u16::MAX,
            ..TimerState::default()
        }));
        let handle = Stm32AdvancedTimerHandle {
            state: state.clone(),
            hub,
            channel_signals,
            complementary_signals,
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

    fn read_state(&self, offset: u64, at: SimTime) -> u32 {
        let state = self.state.lock().expect("TIM1 lock poisoned");
        match offset {
            CR1 => state.cr1,
            CR2 => state.cr2,
            SMCR => state.smcr,
            DIER => state.dier,
            SR => state.sr,
            CCMR1 => state.ccmr1,
            CCMR2 => state.ccmr2,
            CCER => state.ccer,
            CNT => {
                let period = u64::from(state.arr).saturating_add(1);
                let prescaler = u64::from(state.psc).saturating_add(1);
                if state.cr1 & CR1_CEN != 0 && period != 0 {
                    ((at.ticks().saturating_sub(state.started) / prescaler) % period) as u32
                } else {
                    u32::from(state.cnt)
                }
            }
            PSC => u32::from(state.psc),
            ARR => u32::from(state.arr),
            RCR => u32::from(state.rcr),
            CCR1..=CCR4 => u32::from(state.ccr[((offset - CCR1) / 4) as usize]),
            BDTR => state.bdtr,
            DCR => state.dcr,
            DMAR => state.dmar,
            _ => 0,
        }
    }

    fn write_state(&mut self, offset: u64, value: u32, at: SimTime) {
        let mut state = self.state.lock().expect("TIM1 lock poisoned");
        match offset {
            CR1 => {
                state.cr1 = value;
                state.started = at.ticks();
            }
            CR2 => state.cr2 = value,
            SMCR => state.smcr = value,
            DIER => state.dier = value,
            SR => state.sr &= value,
            EGR if value & EGR_UG != 0 => {
                state.sr |= SR_UIF;
                state.started = at.ticks();
                state.cnt = 0;
            }
            CCMR1 => state.ccmr1 = value,
            CCMR2 => state.ccmr2 = value,
            CCER => state.ccer = value,
            CNT => state.cnt = value as u16,
            PSC => state.psc = value as u16,
            ARR => state.arr = value as u16,
            RCR => state.rcr = value as u16,
            CCR1..=CCR4 => state.ccr[((offset - CCR1) / 4) as usize] = value as u16,
            BDTR => state.bdtr = value,
            DCR => state.dcr = value,
            DMAR => state.dmar = value,
            _ => {}
        }
    }
}

impl Device for Stm32AdvancedTimer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 TIM1 requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 TIM1 access at {offset:#x}"
            )));
        }
        Ok(u64::from(self.read_state(offset, at)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 TIM1 requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 TIM1 access at {offset:#x}"
            )));
        }
        self.write_state(offset, value as u32, at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("TIM1 lock poisoned") = TimerState {
            arr: u16::MAX,
            ..TimerState::default()
        };
        self.handle.poll(SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pwm_channels_and_update_interrupt_are_deterministic() {
        let hub = SignalHub::new();
        let (mut timer, handle) =
            Stm32AdvancedTimer::new("board.stm32l432kc.tim1", hub.clone()).unwrap();
        timer
            .write(ARR, AccessWidth::Word, 9, SimTime::ZERO)
            .unwrap();
        timer
            .write(CCR1, AccessWidth::Word, 4, SimTime::ZERO)
            .unwrap();
        timer
            .write(CCMR1, AccessWidth::Word, 6 << 4, SimTime::ZERO)
            .unwrap();
        timer
            .write(CCER, AccessWidth::Word, 1 | (1 << 2), SimTime::ZERO)
            .unwrap();
        timer
            .write(BDTR, AccessWidth::Word, u64::from(BDTR_MOE), SimTime::ZERO)
            .unwrap();
        timer
            .write(DIER, AccessWidth::Word, u64::from(DIER_UIE), SimTime::ZERO)
            .unwrap();
        timer
            .write(CR1, AccessWidth::Word, u64::from(CR1_CEN), SimTime::ZERO)
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(10)));
        assert_eq!(handle.counter(), 0);
        assert_eq!(timer.read(SR, AccessWidth::Word, SimTime::ZERO), Ok(1));
        timer
            .write(SR, AccessWidth::Word, 0, SimTime::ZERO)
            .unwrap();
        assert_eq!(timer.read(SR, AccessWidth::Word, SimTime::ZERO), Ok(0));
        let main = hub
            .with_registry(|registry| registry.find("board.stm32l432kc.tim1.ch1"))
            .unwrap();
        let complementary = hub
            .with_registry(|registry| registry.find("board.stm32l432kc.tim1.ch1n"))
            .unwrap();
        assert_eq!(
            hub.with_registry(|registry| registry.value(main).unwrap().to_vcd_binary()),
            "1"
        );
        assert_eq!(
            hub.with_registry(|registry| registry.value(complementary).unwrap().to_vcd_binary()),
            "0"
        );
    }

    #[test]
    fn register_access_rejects_non_word_and_unaligned_offsets() {
        let hub = SignalHub::new();
        let (mut timer, _) = Stm32AdvancedTimer::new("tim1", hub).unwrap();
        assert!(
            timer
                .read(CR1 + 2, AccessWidth::HalfWord, SimTime::ZERO)
                .is_err()
        );
        assert!(
            timer
                .write(0x401, AccessWidth::Word, 0, SimTime::ZERO)
                .is_err()
        );
    }
}
