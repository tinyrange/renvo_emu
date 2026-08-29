use super::*;

const IMR1: u64 = 0x00;
const EMR1: u64 = 0x04;
const RTSR1: u64 = 0x08;
const FTSR1: u64 = 0x0c;
const SWIER1: u64 = 0x10;
const PR1: u64 = 0x14;
const IMR2: u64 = 0x20;
const PR2: u64 = 0x34;

#[derive(Default)]
struct ExtiState {
    imr1: u32,
    emr1: u32,
    rtsr1: u32,
    ftsr1: u32,
    pending1: u32,
    last_input: u32,
    registers: [u32; 8],
}

/// Host-facing STM32 EXTI edge and interrupt state.
#[derive(Clone)]
pub struct Stm32ExtiHandle {
    state: Arc<Mutex<ExtiState>>,
    hub: SignalHub,
    interrupt_signal: SignalId,
}

impl Stm32ExtiHandle {
    /// Samples GPIO lines and returns enabled pending EXTI1 lines.
    pub fn poll(&self, inputs: u32, at: SimTime) -> u32 {
        let pending = {
            let mut state = self.state.lock().expect("STM32 EXTI lock poisoned");
            let rising = (!state.last_input) & inputs & state.rtsr1;
            let falling = state.last_input & !inputs & state.ftsr1;
            state.pending1 |= rising | falling;
            state.last_input = inputs;
            state.pending1 & state.imr1
        };
        self.hub
            .set(
                self.interrupt_signal,
                SignalValue::from_u64(u64::from(pending != 0), 1)
                    .expect("EXTI interrupt signal width is valid"),
                at,
            )
            .expect("EXTI interrupt signal is declared");
        pending
    }

    /// Returns all pending EXTI1 lines, including masked lines.
    pub fn pending(&self) -> u32 {
        self.state
            .lock()
            .expect("STM32 EXTI lock poisoned")
            .pending1
    }
}

/// Functional STM32L432 EXTI edge router for GPIO lines 0 through 15.
///
/// The model implements the native IMR/RTSR/FTSR/SWIER/PR register contract,
/// deterministic rising and falling edge latching, and a trace-visible
/// aggregate interrupt request. SYSCFG port selection, event-only routing,
/// and internal wakeup sources remain outside this GPIO-edge slice.
pub struct Stm32Exti {
    name: String,
    state: Arc<Mutex<ExtiState>>,
    handle: Stm32ExtiHandle,
}

impl Stm32Exti {
    /// Creates the named EXTI block and its host sampling handle.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Stm32ExtiHandle), remu_signals::SignalError> {
        let name = name.into();
        let interrupt_signal = hub.declare(
            format!("{name}.irq"),
            SignalValue::from_u64(0, 1)?,
            Some("enabled EXTI GPIO edge request".to_owned()),
        )?;
        let state = Arc::new(Mutex::new(ExtiState::default()));
        let handle = Stm32ExtiHandle {
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

    fn read_register(&self, offset: u64) -> u32 {
        let state = self.state.lock().expect("STM32 EXTI lock poisoned");
        match offset {
            IMR1 => state.imr1,
            EMR1 => state.emr1,
            RTSR1 => state.rtsr1,
            FTSR1 => state.ftsr1,
            PR1 => state.pending1,
            IMR2..=PR2 if (offset - IMR2) % 4 == 0 => {
                state.registers[((offset - IMR2) / 4) as usize]
            }
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u32) {
        let mut state = self.state.lock().expect("STM32 EXTI lock poisoned");
        match offset {
            IMR1 => state.imr1 = value,
            EMR1 => state.emr1 = value,
            RTSR1 => state.rtsr1 = value,
            FTSR1 => state.ftsr1 = value,
            SWIER1 => state.pending1 |= value,
            PR1 => state.pending1 &= !value,
            IMR2..=PR2 if (offset - IMR2) % 4 == 0 => {
                state.registers[((offset - IMR2) / 4) as usize] = value;
            }
            _ => {}
        }
    }
}

impl Device for Stm32Exti {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 EXTI requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 EXTI access at {offset:#x}"
            )));
        }
        Ok(u64::from(self.read_register(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("STM32 EXTI requires word accesses"));
        }
        if offset & 3 != 0 || offset >= 0x400 {
            return Err(DeviceError::new(format!(
                "STM32 EXTI access at {offset:#x}"
            )));
        }
        self.write_register(offset, value as u32);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("STM32 EXTI lock poisoned") = ExtiState::default();
        self.handle
            .hub
            .set(
                self.handle.interrupt_signal,
                SignalValue::from_u64(0, 1).expect("EXTI signal width is valid"),
                SimTime::ZERO,
            )
            .expect("EXTI interrupt signal is declared");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rising_and_falling_edges_latch_and_clear_pending_lines() {
        let hub = SignalHub::new();
        let (mut exti, handle) = Stm32Exti::new("board.stm32l432kc.exti", hub.clone()).unwrap();
        exti.write(IMR1, AccessWidth::Word, 1 << 3, SimTime::ZERO)
            .unwrap();
        exti.write(RTSR1, AccessWidth::Word, 1 << 3, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.poll(0, SimTime::ZERO), 0);
        assert_eq!(handle.poll(1 << 3, SimTime::from_ticks(1)), 1 << 3);
        assert_eq!(exti.read(PR1, AccessWidth::Word, SimTime::ZERO), Ok(1 << 3));
        exti.write(PR1, AccessWidth::Word, 1 << 3, SimTime::ZERO)
            .unwrap();
        exti.write(FTSR1, AccessWidth::Word, 1 << 3, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.poll(0, SimTime::from_ticks(2)), 1 << 3);
        let signal = hub
            .with_registry(|registry| registry.find("board.stm32l432kc.exti.irq"))
            .unwrap();
        assert_eq!(
            hub.with_registry(|registry| registry.value(signal).unwrap().to_vcd_binary()),
            "1"
        );
    }

    #[test]
    fn software_trigger_is_masked_until_enabled() {
        let hub = SignalHub::new();
        let (mut exti, handle) = Stm32Exti::new("exti", hub).unwrap();
        exti.write(SWIER1, AccessWidth::Word, 1 << 7, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.poll(0, SimTime::ZERO), 0);
        exti.write(IMR1, AccessWidth::Word, 1 << 7, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.poll(0, SimTime::from_ticks(1)), 1 << 7);
    }
}
