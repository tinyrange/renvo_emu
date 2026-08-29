use super::*;
use remu_signals::{SignalId, SignalValue};

const ISR: u64 = 0x00;
const ICR: u64 = 0x04;
const IER: u64 = 0x08;
const CFGR: u64 = 0x0c;
const CR: u64 = 0x10;
const CMP: u64 = 0x14;
const ARR: u64 = 0x18;
const CNT: u64 = 0x1c;

const FLAG_CMPM: u32 = 1;
const FLAG_ARRM: u32 = 1 << 1;
const FLAG_MASK: u32 = FLAG_CMPM | FLAG_ARRM;
const CR_ENABLE: u32 = 1;
const CR_SNGSTRT: u32 = 1 << 1;
const CR_CNTSTRT: u32 = 1 << 2;
const CR_COUNTRST: u32 = 1 << 3;
const CR_RESETAR: u32 = 1 << 4;
const CR_MASK: u32 = CR_ENABLE | CR_SNGSTRT | CR_CNTSTRT | CR_COUNTRST | CR_RESETAR;
const CFGR_PRESC_MASK: u32 = 0x7 << 7;
const CFGR_WAVE: u32 = 1 << 20;
const CFGR_WAVPOL: u32 = 1 << 21;
const CFGR_PRELOAD: u32 = 1 << 22;
const CFGR_COUNTMODE: u32 = 1 << 23;
const CFGR_MASK: u32 = CFGR_PRESC_MASK | CFGR_WAVE | CFGR_WAVPOL | CFGR_PRELOAD | CFGR_COUNTMODE;

#[derive(Default)]
struct LptimState {
    isr: u32,
    ier: u32,
    cfgr: u32,
    cr: u32,
    cmp: u16,
    arr: u16,
    cnt: u16,
    started: u64,
    running: bool,
    single_shot: bool,
}

/// Host-facing STM32 LPTIM1 counter and waveform handle.
#[derive(Clone)]
pub struct Stm32Lptim1Handle {
    state: Arc<Mutex<LptimState>>,
    hub: SignalHub,
    interrupt_signal: SignalId,
    output_signal: SignalId,
}

impl Stm32Lptim1Handle {
    /// Advances LPTIM1 and returns whether an enabled compare/arrive interrupt is pending.
    pub fn poll(&self, now: SimTime) -> bool {
        let (pending, output) = {
            let mut state = self.state.lock().expect("STM32 LPTIM1 lock poisoned");
            if state.running && state.cr & CR_ENABLE != 0 {
                let prescaler = 1_u64 << ((state.cfgr & CFGR_PRESC_MASK) >> 7);
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
                    let compare = u64::from(state.cmp);
                    let compare_hit = compare <= u64::from(state.arr)
                        && (updates != 0
                            || (old < compare && old.saturating_add(elapsed) >= compare));
                    if compare_hit {
                        state.isr |= FLAG_CMPM;
                    }
                    if updates != 0 {
                        state.isr |= FLAG_ARRM;
                        if state.single_shot {
                            state.running = false;
                            state.cr &= !(CR_SNGSTRT | CR_CNTSTRT);
                        }
                    }
                }
            }
            let pending = state.isr & state.ier & FLAG_MASK != 0;
            (pending, Self::waveform_level(&state))
        };
        self.publish(pending, output, now);
        pending
    }

    /// Returns the functional counter value.
    pub fn counter(&self) -> u16 {
        self.state.lock().expect("STM32 LPTIM1 lock poisoned").cnt
    }

    /// Returns the functional LPTIM1 output state.
    pub fn waveform_output(&self) -> bool {
        let state = self.state.lock().expect("STM32 LPTIM1 lock poisoned");
        Self::waveform_level(&state)
    }

    fn waveform_level(state: &LptimState) -> bool {
        if state.cr & CR_ENABLE == 0 {
            return false;
        }
        let active = state.running && state.cnt < state.cmp;
        active ^ (state.cfgr & CFGR_WAVPOL != 0)
    }

    fn publish(&self, pending: bool, output: bool, at: SimTime) {
        self.hub
            .set(
                self.interrupt_signal,
                SignalValue::from_u64(u64::from(pending), 1)
                    .expect("LPTIM1 interrupt signal width is valid"),
                at,
            )
            .expect("LPTIM1 interrupt signal is declared");
        self.hub
            .set(
                self.output_signal,
                SignalValue::from_u64(u64::from(output), 1)
                    .expect("LPTIM1 output signal width is valid"),
                at,
            )
            .expect("LPTIM1 output signal is declared");
    }
}

/// Functional STM32L432 LPTIM1 low-power counter slice.
pub struct Stm32Lptim1 {
    name: String,
    state: Arc<Mutex<LptimState>>,
    handle: Stm32Lptim1Handle,
}

impl Stm32Lptim1 {
    /// Creates a disabled LPTIM1 with the documented maximal ARR reset value.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Stm32Lptim1Handle), remu_signals::SignalError> {
        let name = name.into();
        let interrupt_signal = hub.declare(
            format!("{name}.irq"),
            SignalValue::from_u64(0, 1)?,
            Some("LPTIM1 compare/autoreload interrupt request".to_owned()),
        )?;
        let output_signal = hub.declare(
            format!("{name}.out"),
            SignalValue::from_u64(0, 1)?,
            Some("LPTIM1 functional waveform output".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(LptimState {
            arr: u16::MAX,
            cmp: u16::MAX,
            ..LptimState::default()
        }));
        let handle = Stm32Lptim1Handle {
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
        let state = self.state.lock().expect("STM32 LPTIM1 lock poisoned");
        match offset {
            ISR => state.isr,
            IER => state.ier,
            CFGR => state.cfgr,
            CR => state.cr & CR_MASK,
            CMP => u32::from(state.cmp),
            ARR => u32::from(state.arr),
            CNT => {
                if !state.running || state.cr & CR_ENABLE == 0 {
                    return u32::from(state.cnt);
                }
                let prescaler = 1_u64 << ((state.cfgr & CFGR_PRESC_MASK) >> 7);
                let elapsed = at.ticks().saturating_sub(state.started) / prescaler;
                let period = u64::from(state.arr) + 1;
                ((u64::from(state.cnt) + elapsed) % period) as u32
            }
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u32, at: SimTime) {
        let (pending, output) = {
            let mut state = self.state.lock().expect("STM32 LPTIM1 lock poisoned");
            match offset {
                ICR => state.isr &= !(value & FLAG_MASK),
                IER => state.ier = value & FLAG_MASK,
                CFGR => state.cfgr = value & CFGR_MASK,
                CR => {
                    let was_enabled = state.cr & CR_ENABLE != 0;
                    state.cr = value & CR_MASK;
                    if state.cr & CR_COUNTRST != 0 {
                        state.cnt = 0;
                        state.cr &= !CR_COUNTRST;
                    }
                    if state.cr & CR_ENABLE == 0 {
                        state.running = false;
                    } else if !was_enabled || value & (CR_SNGSTRT | CR_CNTSTRT) != 0 {
                        state.started = at.ticks();
                        state.running = value & (CR_SNGSTRT | CR_CNTSTRT) != 0;
                        state.single_shot = value & CR_SNGSTRT != 0;
                    }
                }
                CMP => state.cmp = value as u16,
                ARR => state.arr = value as u16,
                CNT => {
                    state.cnt = value as u16;
                    state.started = at.ticks();
                }
                _ => {}
            }
            let pending = state.isr & state.ier & FLAG_MASK != 0;
            (pending, Stm32Lptim1Handle::waveform_level(&state))
        };
        self.handle.publish(pending, output, at);
    }
}

impl Device for Stm32Lptim1 {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 LPTIM1 requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 LPTIM1 access at {offset:#x}"
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
            return Err(DeviceError::new("STM32 LPTIM1 requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 LPTIM1 access at {offset:#x}"
            )));
        }
        self.write_register(offset, value as u32, at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32 LPTIM1 lock poisoned") = LptimState {
            arr: u16::MAX,
            cmp: u16::MAX,
            ..LptimState::default()
        };
        self.handle.publish(false, false, SimTime::ZERO);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lptim1_compare_autoreload_and_waveform_are_observable() {
        let hub = SignalHub::new();
        let (mut timer, handle) = Stm32Lptim1::new("lptim1", hub).unwrap();
        timer
            .write(ARR, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        timer
            .write(CMP, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        timer
            .write(CFGR, AccessWidth::Word, 1 << 7, SimTime::ZERO)
            .unwrap();
        timer
            .write(IER, AccessWidth::Word, FLAG_MASK as u64, SimTime::ZERO)
            .unwrap();
        timer
            .write(
                CR,
                AccessWidth::Word,
                u64::from(CR_ENABLE | CR_CNTSTRT),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(handle.waveform_output());
        assert!(handle.poll(SimTime::from_ticks(4)));
        assert_eq!(
            timer.read(CNT, AccessWidth::Word, SimTime::from_ticks(4)),
            Ok(2)
        );
        assert!(!handle.waveform_output());
        assert!(handle.poll(SimTime::from_ticks(8)));
        assert_eq!(
            timer.read(ISR, AccessWidth::Word, SimTime::ZERO),
            Ok(FLAG_MASK as u64)
        );
        timer
            .write(
                ICR,
                AccessWidth::Word,
                FLAG_MASK as u64,
                SimTime::from_ticks(8),
            )
            .unwrap();
        assert_eq!(timer.read(ISR, AccessWidth::Word, SimTime::ZERO), Ok(0));
    }

    #[test]
    fn lptim1_single_shot_stops_after_autoreload() {
        let hub = SignalHub::new();
        let (mut timer, handle) = Stm32Lptim1::new("lptim1", hub).unwrap();
        timer
            .write(ARR, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        timer
            .write(
                CR,
                AccessWidth::Word,
                u64::from(CR_ENABLE | CR_SNGSTRT),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(4)));
        assert_eq!(handle.counter(), 0);
        assert!(!handle.poll(SimTime::from_ticks(8)));
        assert_eq!(handle.counter(), 0);
    }
}
