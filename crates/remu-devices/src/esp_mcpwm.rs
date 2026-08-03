use super::*;

const TIMER_COUNT: usize = 3;
const OPERATOR_COUNT: usize = 3;
const OPERATOR_STRIDE: usize = 0x38;
const CLK_CFG: usize = 0x00;
const TIMER_CFG0: usize = 0x04;
const TIMER_STATUS: usize = 0x10;
const OPERATOR_TIMERSEL: usize = 0x38;
const GEN0_TSTMP_A: usize = 0x40;
const GEN0_TSTMP_B: usize = 0x44;
const GEN0_FORCE: usize = 0x4c;
const INT_ENA: usize = 0x110;
const INT_RAW: usize = 0x114;
const INT_ST: usize = 0x118;
const INT_CLR: usize = 0x11c;
const VERSION: usize = 0x12c;
const INT_MASK: u32 = (1 << 30) - 1;
const GENERATOR_FORCE_RESET: u32 = 1 << 5;

/// Native-address MCPWM register block with deterministic PWM observations.
pub struct EspMcpwm {
    name: String,
    state: Arc<Mutex<EspMcpwmState>>,
}

struct EspMcpwmState {
    registers: Vec<u32>,
    timer_base: [u64; TIMER_COUNT],
    timer_value: [u32; TIMER_COUNT],
    timer_at: [SimTime; TIMER_COUNT],
    outputs: [bool; OPERATOR_COUNT * 2],
    hub: SignalHub,
    signals: [SignalId; OPERATOR_COUNT * 2],
}

impl EspMcpwm {
    /// Creates MCPWM0, including three timers and six generator signals.
    pub fn new(name: impl Into<String>, hub: SignalHub) -> Result<Self, SignalError> {
        let mut signals = Vec::with_capacity(OPERATOR_COUNT * 2);
        for operator in 0..OPERATOR_COUNT {
            for channel in ['a', 'b'] {
                signals.push(hub.declare(
                    format!("board.esp32c6.mcpwm.gen{operator}{channel}"),
                    SignalValue::from_u64(0, 1)?,
                    Some(format!("ESP32-C6 MCPWM operator {operator}{channel}")),
                )?);
            }
        }
        let signals: [SignalId; OPERATOR_COUNT * 2] =
            signals.try_into().expect("six MCPWM signals");
        let state = EspMcpwmState {
            registers: vec![0; 0x1000 / 4],
            timer_base: [0; TIMER_COUNT],
            timer_value: [0; TIMER_COUNT],
            timer_at: [SimTime::ZERO; TIMER_COUNT],
            outputs: [false; OPERATOR_COUNT * 2],
            hub,
            signals,
        };
        let mut device = Self {
            name: name.into(),
            state: Arc::new(Mutex::new(state)),
        };
        device.reset(ResetKind::PowerOn);
        Ok(device)
    }
}

impl EspMcpwmState {
    fn timer_offset(timer: usize, offset: usize) -> usize {
        TIMER_CFG0 + timer * 0x10 + offset
    }

    fn timer_period(&self, timer: usize) -> u32 {
        (self.registers[Self::timer_offset(timer, 0) / 4] >> 8) & 0xffff
    }

    fn timer_running(&self, timer: usize) -> bool {
        let cfg = self.registers[Self::timer_offset(timer, 4) / 4];
        // TIMER_START is a command field: 0/1 stop at empty/full, while
        // 2/3/4 start (with the latter two stopping at a boundary).
        matches!(cfg & 7, 2..=4) && ((cfg >> 3) & 3) != 0
    }

    fn timer_mode(&self, timer: usize) -> u32 {
        (self.registers[Self::timer_offset(timer, 4) / 4] >> 3) & 3
    }

    fn advance(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let clock_divider = u64::from((self.registers[CLK_CFG / 4] & 0xff) + 1);
        for timer in 0..TIMER_COUNT {
            let elapsed = at.ticks().saturating_sub(self.timer_at[timer].ticks());
            if self.timer_running(timer) {
                let timer_divider =
                    u64::from((self.registers[Self::timer_offset(timer, 0) / 4] & 0xff) + 1);
                let ticks = elapsed / clock_divider.saturating_mul(timer_divider).max(1);
                let period = self.timer_period(timer).max(1);
                let span = u64::from(period) + 1;
                let value = match self.timer_mode(timer) {
                    1 => (self.timer_base[timer].wrapping_add(ticks) % span) as u32,
                    2 => (self.timer_base[timer].wrapping_sub(ticks % span) % span) as u32,
                    3 => {
                        let full = u64::from(period) * 2;
                        let phase = self.timer_base[timer].wrapping_add(ticks) % full.max(1);
                        if phase <= u64::from(period) {
                            phase as u32
                        } else {
                            (full - phase) as u32
                        }
                    }
                    _ => self.timer_base[timer] as u32,
                };
                self.timer_value[timer] = value;
                self.timer_base[timer] = u64::from(value);
            }
            self.timer_at[timer] = at;
            self.registers[(TIMER_STATUS + timer * 0x10) / 4] = self.timer_value[timer];
        }
        for operator in 0..OPERATOR_COUNT {
            let timer = ((self.registers[OPERATOR_TIMERSEL / 4] >> (operator * 2)) & 3) as usize;
            if timer >= TIMER_COUNT {
                continue;
            }
            let force = self.registers[(GEN0_FORCE + operator * OPERATOR_STRIDE) / 4];
            let compares = [
                self.registers[(GEN0_TSTMP_A + operator * OPERATOR_STRIDE) / 4] & 0xffff,
                self.registers[(GEN0_TSTMP_B + operator * OPERATOR_STRIDE) / 4] & 0xffff,
            ];
            for (channel, compare) in compares.into_iter().enumerate() {
                let force_mode = (force >> (6 + channel * 2)) & 3;
                let output = match force_mode {
                    1 => false,
                    2 => true,
                    _ => self.timer_running(timer) && self.timer_value[timer] < compare,
                };
                let signal = operator * 2 + channel;
                if self.outputs[signal] != output {
                    self.outputs[signal] = output;
                    self.hub
                        .set(
                            self.signals[signal],
                            SignalValue::from_u64(u64::from(output), 1)
                                .expect("MCPWM output is one bit"),
                            at,
                        )
                        .map_err(|error| DeviceError::new(error.to_string()))?;
                }
            }
        }
        Ok(())
    }
}

impl Device for EspMcpwm {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("ESP MCPWM requires aligned word access"));
        }
        let offset = usize::try_from(offset).expect("MCPWM offset fits");
        let mut state = self.state.lock().expect("ESP MCPWM lock poisoned");
        state.advance(at)?;
        let value = if offset == VERSION {
            35_656_256
        } else {
            *state
                .registers
                .get(offset / 4)
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?
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
            return Err(DeviceError::new("ESP MCPWM requires aligned word access"));
        }
        let offset = usize::try_from(offset).expect("MCPWM offset fits");
        let mut state = self.state.lock().expect("ESP MCPWM lock poisoned");
        if offset >= state.registers.len() * 4 {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        state.advance(at)?;
        match offset {
            INT_RAW | INT_ST | VERSION => {}
            INT_ENA => state.registers[INT_ENA / 4] = value as u32 & INT_MASK,
            INT_CLR => {
                state.registers[INT_RAW / 4] &= !(value as u32 & INT_MASK);
                state.registers[INT_ST / 4] =
                    state.registers[INT_RAW / 4] & state.registers[INT_ENA / 4];
            }
            _ => state.registers[offset / 4] = value as u32,
        }
        state.advance(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ESP MCPWM lock poisoned");
        state.registers.fill(0);
        state.timer_base = [0; TIMER_COUNT];
        state.timer_value = [0; TIMER_COUNT];
        state.timer_at = [SimTime::ZERO; TIMER_COUNT];
        state.outputs = [false; OPERATOR_COUNT * 2];
        for timer in 0..TIMER_COUNT {
            state.registers[EspMcpwmState::timer_offset(timer, 0) / 4] = 255 << 8;
            state.registers[(GEN0_FORCE + timer * OPERATOR_STRIDE) / 4] = GENERATOR_FORCE_RESET;
        }
        state.registers[VERSION / 4] = 35_656_256;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_and_generator_produce_deterministic_pwm_signal() {
        let hub = SignalHub::new();
        let mut mcpwm = EspMcpwm::new("mcpwm", hub.clone()).unwrap();
        mcpwm
            .write(TIMER_CFG0 as u64, AccessWidth::Word, 9 << 8, SimTime::ZERO)
            .unwrap();
        mcpwm
            .write(
                (TIMER_CFG0 + 4) as u64,
                AccessWidth::Word,
                2 | (1 << 3),
                SimTime::ZERO,
            )
            .unwrap();
        mcpwm
            .write(GEN0_TSTMP_A as u64, AccessWidth::Word, 5, SimTime::ZERO)
            .unwrap();
        mcpwm
            .read(
                TIMER_STATUS as u64,
                AccessWidth::Word,
                SimTime::from_ticks(2),
            )
            .unwrap();
        mcpwm
            .read(
                TIMER_STATUS as u64,
                AccessWidth::Word,
                SimTime::from_ticks(6),
            )
            .unwrap();
        let changes = hub.drain_changes();
        assert!(
            changes
                .iter()
                .any(|change| change.value.bit(0) == Some(Logic::One))
        );
        assert!(
            changes
                .iter()
                .any(|change| change.value.bit(0) == Some(Logic::Zero))
        );
    }

    #[test]
    fn software_force_and_native_version_are_visible() {
        let hub = SignalHub::new();
        let mut mcpwm = EspMcpwm::new("mcpwm", hub).unwrap();
        mcpwm
            .write(GEN0_FORCE as u64, AccessWidth::Word, 2 << 6, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            mcpwm
                .read(VERSION as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            35_656_256
        );
    }

    #[test]
    fn stop_commands_do_not_advance_and_generator_force_has_native_reset_value() {
        let hub = SignalHub::new();
        let mut mcpwm = EspMcpwm::new("mcpwm", hub).unwrap();
        assert_eq!(
            mcpwm
                .read(GEN0_FORCE as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(GENERATOR_FORCE_RESET)
        );
        mcpwm
            .write(TIMER_CFG0 as u64, AccessWidth::Word, 9 << 8, SimTime::ZERO)
            .unwrap();
        mcpwm
            .write(
                (TIMER_CFG0 + 4) as u64,
                AccessWidth::Word,
                1 << 3,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            mcpwm
                .read(
                    TIMER_STATUS as u64,
                    AccessWidth::Word,
                    SimTime::from_ticks(5)
                )
                .unwrap(),
            0
        );
    }
}
