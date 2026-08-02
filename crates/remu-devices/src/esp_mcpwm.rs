use super::*;

const TIMERS: usize = 3;
const OPERATORS: usize = 3;
const OUTPUTS: usize = OPERATORS * 2;
const TIMER_BASE: u64 = 0x04;
const TIMER_STRIDE: u64 = 0x10;
const OP_BASE: u64 = 0x3c;
const OP_STRIDE: u64 = 0x38;
const INT_ENA: u64 = 0x110;
const INT_RAW: u64 = 0x114;
const INT_ST: u64 = 0x118;
const INT_CLR: u64 = 0x11c;
const UPDATE_CFG: u64 = 0x10c;
const CLK_CFG: u64 = 0x00;
const CLK: u64 = 0x120;
const VERSION: u64 = 0x124;

#[derive(Clone, Copy)]
struct McpwmTimer {
    cfg0: u32,
    cfg1: u32,
    sync: u32,
    value: u16,
    direction_down: bool,
    last_time: SimTime,
}

impl McpwmTimer {
    const fn reset() -> Self {
        Self {
            cfg0: 255 << 8,
            cfg1: 0,
            sync: 0,
            value: 0,
            direction_down: false,
            last_time: SimTime::ZERO,
        }
    }

    fn period(self) -> u32 {
        ((self.cfg0 >> 8) & 0xffff).max(1)
    }

    fn divider(self, clock_cfg: u32) -> u64 {
        u64::from((clock_cfg & 0xff).saturating_add(1))
            * u64::from((self.cfg0 & 0xff).saturating_add(1))
    }

    fn running(self) -> bool {
        self.cfg1 & 7 != 0 && (self.cfg1 >> 3) & 3 != 0
    }
}

#[derive(Clone, Copy)]
struct McpwmOperator {
    timestamp_a: u16,
    timestamp_b: u16,
    cfg0: u32,
    force: u32,
    generator_a: u32,
    generator_b: u32,
}

impl McpwmOperator {
    const fn reset() -> Self {
        Self {
            timestamp_a: 0,
            timestamp_b: 0,
            cfg0: 0,
            force: 0,
            generator_a: 0,
            generator_b: 0,
        }
    }
}

struct McpwmState {
    clock_cfg: u32,
    timers: [McpwmTimer; TIMERS],
    operators: [McpwmOperator; OPERATORS],
    operator_timer_sel: u32,
    update_cfg: u32,
    clock: u32,
    int_ena: u32,
    int_raw: u32,
    outputs: [bool; OUTPUTS],
}

impl McpwmState {
    const fn reset() -> Self {
        Self {
            clock_cfg: 0,
            timers: [McpwmTimer::reset(); TIMERS],
            operators: [McpwmOperator::reset(); OPERATORS],
            operator_timer_sel: 0,
            update_cfg: 0x1fc,
            clock: 0,
            int_ena: 0,
            int_raw: 0,
            outputs: [false; OUTPUTS],
        }
    }

    fn timer_mode(timer: McpwmTimer) -> u32 {
        (timer.cfg1 >> 3) & 3
    }

    fn advance(&mut self, now: SimTime) {
        for (index, timer) in self.timers.iter_mut().enumerate() {
            let elapsed = now.ticks().saturating_sub(timer.last_time.ticks());
            if elapsed == 0 || !timer.running() {
                timer.last_time = now;
                continue;
            }
            let steps = elapsed / timer.divider(self.clock_cfg);
            if steps == 0 {
                timer.last_time = now;
                continue;
            }
            let period = u64::from(timer.period());
            let old = u64::from(timer.value);
            let mode = Self::timer_mode(*timer);
            let (value, direction_down, wrapped) = match mode {
                1 => {
                    let total = old + steps;
                    (total % period, false, total >= period)
                }
                2 => {
                    let steps = steps % period;
                    let value = (old + period - steps) % period;
                    (value, true, steps > old)
                }
                3 => {
                    let cycle = period.saturating_mul(2).saturating_sub(2).max(1);
                    let phase = if timer.direction_down {
                        period
                            .saturating_add(period.saturating_sub(old))
                            .saturating_sub(2)
                    } else {
                        old
                    };
                    let next = (phase + steps) % cycle;
                    let down = next >= period;
                    let value = if down {
                        cycle.saturating_sub(next)
                    } else {
                        next
                    };
                    (value, down, phase + steps >= cycle)
                }
                _ => (old, timer.direction_down, false),
            };
            timer.value = u16::try_from(value.min(u64::from(u16::MAX))).expect("MCPWM value fits");
            timer.direction_down = direction_down;
            timer.last_time = now;
            if wrapped {
                self.int_raw |= 1 << index;
            }
        }
    }

    fn output_level(&self, operator: usize, output: usize) -> bool {
        let timer_index = usize::try_from((self.operator_timer_sel >> (operator * 2)) & 3)
            .expect("MCPWM timer selector fits");
        let timer = self.timers[timer_index.min(TIMERS - 1)];
        let op = self.operators[operator];
        let force_shift = if output == 0 { 6 } else { 8 };
        match (op.force >> force_shift) & 3 {
            1 => false,
            2 => true,
            _ if !timer.running() => false,
            _ => {
                let compare = if output == 0 {
                    op.timestamp_a
                } else {
                    op.timestamp_b
                };
                u32::from(timer.value) < u32::from(compare)
            }
        }
    }
}

/// Scheduler-facing handle for one ESP32-S3 MCPWM instance.
#[derive(Clone)]
pub struct Esp32S3McpwmHandle {
    state: Rc<RefCell<McpwmState>>,
    hub: SignalHub,
    output_signals: Vec<SignalId>,
}

impl Esp32S3McpwmHandle {
    /// Advances all three functional PWM timers and publishes output changes.
    pub fn poll(&self, now: SimTime) -> Result<bool, SignalError> {
        let mut state = self.state.borrow_mut();
        state.advance(now);
        let mut changed = false;
        for operator in 0..OPERATORS {
            for output in 0..2 {
                let index = operator * 2 + output;
                let level = state.output_level(operator, output);
                if state.outputs[index] == level {
                    continue;
                }
                state.outputs[index] = level;
                self.hub.set(
                    self.output_signals[index],
                    SignalValue::from_u64(u64::from(level), 1)?,
                    now,
                )?;
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Returns the current output level for a generator output.
    pub fn output_level(&self, output: usize) -> bool {
        self.state.borrow().outputs[output]
    }

    /// Returns a timer's current counter value.
    pub fn timer_value(&self, timer: usize) -> u16 {
        self.state.borrow().timers[timer].value
    }
}

/// Functional ESP32-S3 MCPWM instance.
///
/// This model covers the native three-timer/three-operator register layout,
/// abstract-tick up/down counters, compare timestamps, software force levels,
/// timer-overflow interrupt latches, and six VCD-visible generator outputs.
/// Dead-time, carrier modulation, fault handling, capture, GPIO-matrix routing,
/// and exact clock fidelity remain outside this functional slice.
pub struct Esp32S3Mcpwm {
    name: String,
    state: Rc<RefCell<McpwmState>>,
    hub: SignalHub,
    output_signals: Vec<SignalId>,
}

impl Esp32S3Mcpwm {
    /// Creates an MCPWM instance and its scheduler handle.
    pub fn new(
        name: impl Into<String>,
        signal_path: &str,
        hub: SignalHub,
    ) -> Result<(Self, Esp32S3McpwmHandle), SignalError> {
        let mut output_signals = Vec::with_capacity(OUTPUTS);
        for operator in 0..OPERATORS {
            for output in ['a', 'b'] {
                output_signals.push(hub.declare(
                    format!("{signal_path}.op{operator}.{output}"),
                    SignalValue::from_u64(0, 1)?,
                    Some("Functional ESP32-S3 MCPWM generator output".to_owned()),
                )?);
            }
        }
        let state = Rc::new(RefCell::new(McpwmState::reset()));
        let device = Self {
            name: name.into(),
            state: state.clone(),
            hub: hub.clone(),
            output_signals: output_signals.clone(),
        };
        let handle = Esp32S3McpwmHandle {
            state,
            hub,
            output_signals,
        };
        Ok((device, handle))
    }

    fn handle(&self) -> Esp32S3McpwmHandle {
        Esp32S3McpwmHandle {
            state: self.state.clone(),
            hub: self.hub.clone(),
            output_signals: self.output_signals.clone(),
        }
    }

    fn read_register(&self, offset: u64) -> Result<u32, DeviceError> {
        let state = self.state.borrow();
        if offset == CLK_CFG {
            return Ok(state.clock_cfg);
        }
        if let Some((timer, register)) = timer_register(offset) {
            let timer_state = state.timers[timer];
            return Ok(match register {
                0 => timer_state.cfg0,
                4 => timer_state.cfg1,
                8 => timer_state.sync,
                12 => u32::from(timer_state.value) | (u32::from(timer_state.direction_down) << 16),
                _ => unreachable!(),
            });
        }
        if offset == 0x38 {
            return Ok(state.operator_timer_sel);
        }
        if let Some((operator, register)) = operator_register(offset) {
            let op = state.operators[operator];
            return Ok(match register {
                0x04 => u32::from(op.timestamp_a),
                0x08 => u32::from(op.timestamp_b),
                0x10 => op.cfg0,
                0x14 => op.force,
                0x18 => op.generator_a,
                0x1c => op.generator_b,
                _ => 0,
            });
        }
        match offset {
            UPDATE_CFG => Ok(state.update_cfg),
            INT_ENA => Ok(state.int_ena),
            INT_RAW => Ok(state.int_raw),
            INT_ST => Ok(state.int_raw & state.int_ena),
            CLK => Ok(state.clock),
            VERSION => Ok(0x0100_0000),
            _ => Err(DeviceError::new(format!(
                "unmodeled ESP32-S3 MCPWM read at offset {offset:#x}"
            ))),
        }
    }
}

fn timer_register(offset: u64) -> Option<(usize, u64)> {
    let adjusted = offset.checked_sub(TIMER_BASE)?;
    let timer = usize::try_from(adjusted / TIMER_STRIDE).ok()?;
    let register = adjusted % TIMER_STRIDE;
    (timer < TIMERS && register.is_multiple_of(4)).then_some((timer, register))
}

fn operator_register(offset: u64) -> Option<(usize, u64)> {
    let adjusted = offset.checked_sub(OP_BASE)?;
    let operator = usize::try_from(adjusted / OP_STRIDE).ok()?;
    let register = adjusted % OP_STRIDE;
    (operator < OPERATORS && register.is_multiple_of(4)).then_some((operator, register))
}

impl Device for Esp32S3Mcpwm {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 MCPWM requires aligned word access",
            ));
        }
        self.handle()
            .poll(at)
            .map_err(|error| DeviceError::new(error.to_string()))?;
        Ok(u64::from(self.read_register(offset)?))
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
                "ESP32-S3 MCPWM requires aligned word access",
            ));
        }
        self.handle()
            .poll(at)
            .map_err(|error| DeviceError::new(error.to_string()))?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("MCPWM value fits u32");
        let mut state = self.state.borrow_mut();
        if offset == CLK_CFG {
            state.clock_cfg = value & 0xff;
        } else if let Some((timer, register)) = timer_register(offset) {
            let timer_state = &mut state.timers[timer];
            match register {
                0 => timer_state.cfg0 = value & 0x03ff_ffff,
                4 => timer_state.cfg1 = value & 0x1f,
                8 => {
                    timer_state.sync = value & 0x001f_ffff;
                    if value & 2 != 0 {
                        timer_state.value =
                            u16::try_from((value >> 4) & 0xffff).expect("MCPWM phase fits u16");
                        timer_state.last_time = at;
                    }
                }
                12 => return Err(DeviceError::new("MCPWM timer STATUS is read-only")),
                _ => unreachable!(),
            }
        } else if offset == 0x38 {
            state.operator_timer_sel = value & 0x3f;
        } else if let Some((operator, register)) = operator_register(offset) {
            let op = &mut state.operators[operator];
            match register {
                0x04 => op.timestamp_a = u16::try_from(value & 0xffff).expect("timestamp fits"),
                0x08 => op.timestamp_b = u16::try_from(value & 0xffff).expect("timestamp fits"),
                0x10 => op.cfg0 = value,
                0x14 => op.force = value,
                0x18 => op.generator_a = value,
                0x1c => op.generator_b = value,
                _ => {}
            }
        } else {
            match offset {
                UPDATE_CFG => state.update_cfg = value & 0xfff,
                INT_ENA => state.int_ena = value & 0x7,
                INT_CLR => state.int_raw &= !value,
                CLK => state.clock = value & 1,
                INT_RAW | INT_ST | VERSION => {
                    return Err(DeviceError::new("MCPWM register is read-only"));
                }
                _ => {
                    return Err(DeviceError::new(format!(
                        "unmodeled ESP32-S3 MCPWM write at offset {offset:#x}"
                    )));
                }
            }
        }
        drop(state);
        self.handle()
            .poll(at)
            .map(|_| ())
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = McpwmState::reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_compare_and_force_outputs_are_deterministic() {
        let hub = SignalHub::new();
        let (mut mcpwm, handle) =
            Esp32S3Mcpwm::new("mcpwm", "board.esp32s3.mcpwm", hub.clone()).unwrap();
        mcpwm
            .write(TIMER_BASE, AccessWidth::Word, 4 << 8, SimTime::ZERO)
            .unwrap();
        mcpwm
            .write(
                TIMER_BASE + 4,
                AccessWidth::Word,
                2 | (1 << 3),
                SimTime::ZERO,
            )
            .unwrap();
        mcpwm
            .write(OP_BASE + 4, AccessWidth::Word, 2, SimTime::ZERO)
            .unwrap();
        handle.poll(SimTime::from_ticks(1)).unwrap();
        assert!(handle.output_level(0));
        assert!(handle.poll(SimTime::from_ticks(3)).unwrap());
        assert!(!handle.output_level(0));
        mcpwm
            .write(
                OP_BASE + 0x14,
                AccessWidth::Word,
                2 << 6,
                SimTime::from_ticks(3),
            )
            .unwrap();
        assert!(handle.output_level(0));
        let changes = hub.drain_changes();
        assert!(
            changes
                .iter()
                .any(|change| change.value.bit(0) == Some(Logic::One))
        );
    }

    #[test]
    fn timer_wrap_sets_interrupt_and_clear_register_lowers_it() {
        let hub = SignalHub::new();
        let (mut mcpwm, handle) = Esp32S3Mcpwm::new("mcpwm", "board.esp32s3.mcpwm", hub).unwrap();
        mcpwm
            .write(TIMER_BASE, AccessWidth::Word, 2 << 8, SimTime::ZERO)
            .unwrap();
        mcpwm
            .write(
                TIMER_BASE + 4,
                AccessWidth::Word,
                2 | (1 << 3),
                SimTime::ZERO,
            )
            .unwrap();
        mcpwm
            .write(INT_ENA, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        handle.poll(SimTime::from_ticks(3)).unwrap();
        assert_eq!(
            mcpwm
                .read(INT_ST, AccessWidth::Word, SimTime::from_ticks(3))
                .unwrap(),
            1
        );
        mcpwm
            .write(INT_CLR, AccessWidth::Word, 1, SimTime::from_ticks(3))
            .unwrap();
        assert_eq!(
            mcpwm
                .read(INT_RAW, AccessWidth::Word, SimTime::from_ticks(3))
                .unwrap(),
            0
        );
    }
}
