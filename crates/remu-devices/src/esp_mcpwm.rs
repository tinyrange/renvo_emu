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
const TIMER_SYNC_INPUT_CFG: u64 = 0x34;
const OP_TIMER_SEL: u64 = 0x38;
const CLOCK_CFG_MASK: u32 = 0x0000_00ff;
const TIMER_CFG0_MASK: u32 = 0x03ff_ffff;
const TIMER_CFG1_MASK: u32 = 0x0000_001f;
const TIMER_SYNC_MASK: u32 = 0x001f_ffff;
const TIMER_STATUS_MASK: u32 = 0x0001_ffff;
const TIMER_SYNC_INPUT_MASK: u32 = 0x0000_0fff;
const OP_TIMER_SEL_MASK: u32 = 0x0000_003f;
const GEN_STMP_CFG_MASK: u32 = 0x0000_03ff;
const GEN_TIMESTAMP_MASK: u32 = 0x0000_ffff;
const GEN_CFG0_MASK: u32 = 0x0000_03ff;
const GEN_FORCE_MASK: u32 = 0x0000_ffff;
const GEN_ACTION_MASK: u32 = 0x00ff_ffff;
const UPDATE_CFG_MASK: u32 = 0x0000_00ff;
const UPDATE_CFG_RESET: u32 = 0x0000_0055;
const INTERRUPT_MASK: u32 = 0x3fff_ffff;
const CLOCK_MASK: u32 = 0x0000_0001;
const VERSION_MASK: u32 = 0x0fff_ffff;
const VERSION_RESET: u32 = 0x0210_7230;

/// Named ESP32-S3 MCPWM register IDs covered by the functional model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32s3McpwmRegister {
    /// MCPWM clock prescaler.
    ClockCfg,
    /// Timer period/prescaler/update configuration.
    TimerCfg0(usize),
    /// Timer start/stop and counting-mode configuration.
    TimerCfg1(usize),
    /// Timer synchronisation and phase configuration.
    TimerSync(usize),
    /// Timer counter and direction status.
    TimerStatus(usize),
    /// Shared timer synchronisation input selection.
    TimerSyncInputCfg,
    /// Operator-to-timer selectors.
    OperatorTimerSel,
    /// Generator timestamp shadow-transfer configuration.
    GeneratorStmpCfg(usize),
    /// Generator A timestamp shadow register.
    GeneratorTimestampA(usize),
    /// Generator B timestamp shadow register.
    GeneratorTimestampB(usize),
    /// Generator event configuration.
    GeneratorCfg0(usize),
    /// Generator software-force configuration.
    GeneratorForce(usize),
    /// Generator A event actions.
    GeneratorA(usize),
    /// Generator B event actions.
    GeneratorB(usize),
    /// Global and per-operator active-register update enables.
    UpdateCfg,
    /// Interrupt enables.
    IntEna,
    /// Raw interrupt status.
    IntRaw,
    /// Masked interrupt status.
    IntSt,
    /// Write-one-to-clear interrupt status.
    IntClr,
    /// Register-file clock gate.
    Clock,
    /// MCPWM version/date register.
    Version,
}

impl Esp32s3McpwmRegister {
    /// Returns the native byte offset of this register ID.
    pub const fn offset(self) -> u64 {
        match self {
            Self::ClockCfg => CLK_CFG,
            Self::TimerCfg0(timer) => TIMER_BASE + (timer as u64) * TIMER_STRIDE,
            Self::TimerCfg1(timer) => TIMER_BASE + (timer as u64) * TIMER_STRIDE + 0x04,
            Self::TimerSync(timer) => TIMER_BASE + (timer as u64) * TIMER_STRIDE + 0x08,
            Self::TimerStatus(timer) => 0x10 + (timer as u64) * TIMER_STRIDE,
            Self::TimerSyncInputCfg => TIMER_SYNC_INPUT_CFG,
            Self::OperatorTimerSel => OP_TIMER_SEL,
            Self::GeneratorStmpCfg(operator) => OP_BASE + (operator as u64) * OP_STRIDE,
            Self::GeneratorTimestampA(operator) => OP_BASE + (operator as u64) * OP_STRIDE + 0x04,
            Self::GeneratorTimestampB(operator) => OP_BASE + (operator as u64) * OP_STRIDE + 0x08,
            Self::GeneratorCfg0(operator) => OP_BASE + (operator as u64) * OP_STRIDE + 0x0c,
            Self::GeneratorForce(operator) => OP_BASE + (operator as u64) * OP_STRIDE + 0x10,
            Self::GeneratorA(operator) => OP_BASE + (operator as u64) * OP_STRIDE + 0x14,
            Self::GeneratorB(operator) => OP_BASE + (operator as u64) * OP_STRIDE + 0x18,
            Self::UpdateCfg => UPDATE_CFG,
            Self::IntEna => INT_ENA,
            Self::IntRaw => INT_RAW,
            Self::IntSt => INT_ST,
            Self::IntClr => INT_CLR,
            Self::Clock => CLK,
            Self::Version => VERSION,
        }
    }

    /// Converts a modeled native offset into a named register ID.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        if offset == CLK_CFG {
            return Some(Self::ClockCfg);
        }
        if offset >= TIMER_BASE && offset <= 0x30 {
            let timer = (offset - TIMER_BASE) / TIMER_STRIDE;
            let register = (offset - TIMER_BASE) % TIMER_STRIDE;
            if timer < TIMERS as u64 {
                return Some(match register {
                    0x00 => Self::TimerCfg0(timer as usize),
                    0x04 => Self::TimerCfg1(timer as usize),
                    0x08 => Self::TimerSync(timer as usize),
                    0x0c => Self::TimerStatus(timer as usize),
                    _ => return None,
                });
            }
        }
        Some(match offset {
            TIMER_SYNC_INPUT_CFG => Self::TimerSyncInputCfg,
            OP_TIMER_SEL => Self::OperatorTimerSel,
            UPDATE_CFG => Self::UpdateCfg,
            INT_ENA => Self::IntEna,
            INT_RAW => Self::IntRaw,
            INT_ST => Self::IntSt,
            INT_CLR => Self::IntClr,
            CLK => Self::Clock,
            VERSION => Self::Version,
            _ if offset >= OP_BASE && offset < UPDATE_CFG => {
                let operator = (offset - OP_BASE) / OP_STRIDE;
                let register = (offset - OP_BASE) % OP_STRIDE;
                if operator >= OPERATORS as u64 {
                    return None;
                }
                match register {
                    0x00 => Self::GeneratorStmpCfg(operator as usize),
                    0x04 => Self::GeneratorTimestampA(operator as usize),
                    0x08 => Self::GeneratorTimestampB(operator as usize),
                    0x0c => Self::GeneratorCfg0(operator as usize),
                    0x10 => Self::GeneratorForce(operator as usize),
                    0x14 => Self::GeneratorA(operator as usize),
                    0x18 => Self::GeneratorB(operator as usize),
                    _ => return None,
                }
            }
            _ => return None,
        })
    }
}

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
    stmp_cfg: u32,
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
            stmp_cfg: 0,
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
    timer_sync_input_cfg: u32,
    timers: [McpwmTimer; TIMERS],
    operators: [McpwmOperator; OPERATORS],
    operator_timer_sel: u32,
    update_cfg: u32,
    clock: u32,
    int_ena: u32,
    int_raw: u32,
    outputs: [bool; OUTPUTS],
    version: u32,
}

impl McpwmState {
    const fn reset() -> Self {
        Self {
            clock_cfg: 0,
            timer_sync_input_cfg: 0,
            timers: [McpwmTimer::reset(); TIMERS],
            operators: [McpwmOperator::reset(); OPERATORS],
            operator_timer_sel: 0,
            update_cfg: UPDATE_CFG_RESET,
            clock: 0,
            int_ena: 0,
            int_raw: 0,
            outputs: [false; OUTPUTS],
            version: VERSION_RESET,
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
                let interrupt = match mode {
                    1 => 6 + index,
                    2 | 3 => 3 + index,
                    _ => unreachable!(),
                };
                self.int_raw |= 1 << interrupt;
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
        match Esp32s3McpwmRegister::from_offset(offset) {
            Some(Esp32s3McpwmRegister::ClockCfg) => Ok(state.clock_cfg & CLOCK_CFG_MASK),
            Some(Esp32s3McpwmRegister::TimerCfg0(timer)) => {
                Ok(state.timers[timer].cfg0 & TIMER_CFG0_MASK)
            }
            Some(Esp32s3McpwmRegister::TimerCfg1(timer)) => {
                Ok(state.timers[timer].cfg1 & TIMER_CFG1_MASK)
            }
            Some(Esp32s3McpwmRegister::TimerSync(timer)) => {
                Ok(state.timers[timer].sync & TIMER_SYNC_MASK)
            }
            Some(Esp32s3McpwmRegister::TimerStatus(timer)) => {
                let timer_state = state.timers[timer];
                Ok(
                    (u32::from(timer_state.value) | (u32::from(timer_state.direction_down) << 16))
                        & TIMER_STATUS_MASK,
                )
            }
            Some(Esp32s3McpwmRegister::TimerSyncInputCfg) => {
                Ok(state.timer_sync_input_cfg & TIMER_SYNC_INPUT_MASK)
            }
            Some(Esp32s3McpwmRegister::OperatorTimerSel) => {
                Ok(state.operator_timer_sel & OP_TIMER_SEL_MASK)
            }
            Some(Esp32s3McpwmRegister::GeneratorStmpCfg(operator)) => {
                Ok(state.operators[operator].stmp_cfg & GEN_STMP_CFG_MASK)
            }
            Some(Esp32s3McpwmRegister::GeneratorTimestampA(operator)) => {
                Ok(u32::from(state.operators[operator].timestamp_a))
            }
            Some(Esp32s3McpwmRegister::GeneratorTimestampB(operator)) => {
                Ok(u32::from(state.operators[operator].timestamp_b))
            }
            Some(Esp32s3McpwmRegister::GeneratorCfg0(operator)) => {
                Ok(state.operators[operator].cfg0 & GEN_CFG0_MASK)
            }
            Some(Esp32s3McpwmRegister::GeneratorForce(operator)) => {
                Ok(state.operators[operator].force & GEN_FORCE_MASK)
            }
            Some(Esp32s3McpwmRegister::GeneratorA(operator)) => {
                Ok(state.operators[operator].generator_a & GEN_ACTION_MASK)
            }
            Some(Esp32s3McpwmRegister::GeneratorB(operator)) => {
                Ok(state.operators[operator].generator_b & GEN_ACTION_MASK)
            }
            Some(Esp32s3McpwmRegister::UpdateCfg) => Ok(state.update_cfg & UPDATE_CFG_MASK),
            Some(Esp32s3McpwmRegister::IntEna) => Ok(state.int_ena & INTERRUPT_MASK),
            Some(Esp32s3McpwmRegister::IntRaw) => Ok(state.int_raw & INTERRUPT_MASK),
            Some(Esp32s3McpwmRegister::IntSt) => {
                Ok((state.int_raw & state.int_ena) & INTERRUPT_MASK)
            }
            Some(Esp32s3McpwmRegister::IntClr) => {
                Err(DeviceError::new("ESP32-S3 MCPWM INT_CLR is write-only"))
            }
            Some(Esp32s3McpwmRegister::Clock) => Ok(state.clock & CLOCK_MASK),
            Some(Esp32s3McpwmRegister::Version) => Ok(state.version & VERSION_MASK),
            None => Err(DeviceError::new(format!(
                "unmodeled ESP32-S3 MCPWM read at offset {offset:#x}"
            ))),
        }
    }
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
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 MCPWM value exceeds u32"))?;
        let mut state = self.state.borrow_mut();
        match Esp32s3McpwmRegister::from_offset(offset) {
            Some(Esp32s3McpwmRegister::ClockCfg) => state.clock_cfg = value & CLOCK_CFG_MASK,
            Some(Esp32s3McpwmRegister::TimerCfg0(timer)) => {
                state.timers[timer].cfg0 = value & TIMER_CFG0_MASK
            }
            Some(Esp32s3McpwmRegister::TimerCfg1(timer)) => {
                state.timers[timer].cfg1 = value & TIMER_CFG1_MASK
            }
            Some(Esp32s3McpwmRegister::TimerSync(timer)) => {
                let timer_state = &mut state.timers[timer];
                timer_state.sync = value & TIMER_SYNC_MASK;
                if value & (1 << 1) != 0 {
                    timer_state.value =
                        u16::try_from((value >> 4) & 0xffff).expect("MCPWM phase fits u16");
                    timer_state.direction_down = value & (1 << 20) != 0;
                    timer_state.last_time = at;
                }
            }
            Some(Esp32s3McpwmRegister::TimerStatus(timer)) => {
                return Err(DeviceError::new(format!(
                    "ESP32-S3 MCPWM timer {timer} STATUS is read-only"
                )));
            }
            Some(Esp32s3McpwmRegister::TimerSyncInputCfg) => {
                state.timer_sync_input_cfg = value & TIMER_SYNC_INPUT_MASK
            }
            Some(Esp32s3McpwmRegister::OperatorTimerSel) => {
                state.operator_timer_sel = value & OP_TIMER_SEL_MASK
            }
            Some(Esp32s3McpwmRegister::GeneratorStmpCfg(operator)) => {
                state.operators[operator].stmp_cfg = value & GEN_STMP_CFG_MASK
            }
            Some(Esp32s3McpwmRegister::GeneratorTimestampA(operator)) => {
                state.operators[operator].timestamp_a =
                    u16::try_from(value & GEN_TIMESTAMP_MASK).expect("timestamp fits")
            }
            Some(Esp32s3McpwmRegister::GeneratorTimestampB(operator)) => {
                state.operators[operator].timestamp_b =
                    u16::try_from(value & GEN_TIMESTAMP_MASK).expect("timestamp fits")
            }
            Some(Esp32s3McpwmRegister::GeneratorCfg0(operator)) => {
                state.operators[operator].cfg0 = value & GEN_CFG0_MASK
            }
            Some(Esp32s3McpwmRegister::GeneratorForce(operator)) => {
                state.operators[operator].force = value & GEN_FORCE_MASK
            }
            Some(Esp32s3McpwmRegister::GeneratorA(operator)) => {
                state.operators[operator].generator_a = value & GEN_ACTION_MASK
            }
            Some(Esp32s3McpwmRegister::GeneratorB(operator)) => {
                state.operators[operator].generator_b = value & GEN_ACTION_MASK
            }
            Some(Esp32s3McpwmRegister::UpdateCfg) => state.update_cfg = value & UPDATE_CFG_MASK,
            Some(Esp32s3McpwmRegister::IntEna) => state.int_ena = value & INTERRUPT_MASK,
            Some(Esp32s3McpwmRegister::IntRaw) => state.int_raw &= !(value & INTERRUPT_MASK),
            Some(Esp32s3McpwmRegister::IntClr) => state.int_raw &= !(value & INTERRUPT_MASK),
            Some(Esp32s3McpwmRegister::Clock) => state.clock = value & CLOCK_MASK,
            Some(Esp32s3McpwmRegister::IntSt) => {
                return Err(DeviceError::new("ESP32-S3 MCPWM INT_ST is read-only"));
            }
            Some(Esp32s3McpwmRegister::Version) => state.version = value & VERSION_MASK,
            None => {
                return Err(DeviceError::new(format!(
                    "unmodeled ESP32-S3 MCPWM write at offset {offset:#x}"
                )));
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
        for signal in &self.output_signals {
            self.hub
                .set(
                    *signal,
                    SignalValue::from_u64(0, 1).expect("MCPWM output signal is one bit"),
                    SimTime::ZERO,
                )
                .expect("MCPWM output signals remain declared");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_ids_match_native_timer_and_generator_windows() {
        let expected = [
            (Esp32s3McpwmRegister::ClockCfg, 0x00),
            (Esp32s3McpwmRegister::TimerCfg0(0), 0x04),
            (Esp32s3McpwmRegister::TimerCfg1(2), 0x28),
            (Esp32s3McpwmRegister::TimerSync(1), 0x1c),
            (Esp32s3McpwmRegister::TimerStatus(0), 0x10),
            (Esp32s3McpwmRegister::TimerStatus(2), 0x30),
            (Esp32s3McpwmRegister::TimerSyncInputCfg, 0x34),
            (Esp32s3McpwmRegister::OperatorTimerSel, 0x38),
            (Esp32s3McpwmRegister::GeneratorStmpCfg(0), 0x3c),
            (Esp32s3McpwmRegister::GeneratorTimestampA(2), 0xb0),
            (Esp32s3McpwmRegister::GeneratorCfg0(1), 0x80),
            (Esp32s3McpwmRegister::GeneratorForce(1), 0x84),
            (Esp32s3McpwmRegister::GeneratorA(1), 0x88),
            (Esp32s3McpwmRegister::GeneratorB(1), 0x8c),
            (Esp32s3McpwmRegister::UpdateCfg, 0x10c),
            (Esp32s3McpwmRegister::IntEna, 0x110),
            (Esp32s3McpwmRegister::IntRaw, 0x114),
            (Esp32s3McpwmRegister::IntSt, 0x118),
            (Esp32s3McpwmRegister::IntClr, 0x11c),
            (Esp32s3McpwmRegister::Clock, 0x120),
            (Esp32s3McpwmRegister::Version, 0x124),
        ];
        for (register, offset) in expected {
            assert_eq!(register.offset(), offset);
            assert_eq!(Esp32s3McpwmRegister::from_offset(offset), Some(register));
        }
        assert_eq!(Esp32s3McpwmRegister::from_offset(OP_BASE + 0x1c), None);
        assert_eq!(Esp32s3McpwmRegister::from_offset(0x128), None);
    }

    #[test]
    fn register_masks_reset_values_and_access_modes_follow_vendor_layout() {
        let hub = SignalHub::new();
        let (mut mcpwm, _) = Esp32S3Mcpwm::new("mcpwm", "board.esp32s3.mcpwm", hub).unwrap();
        assert_eq!(
            mcpwm.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap(),
            255 << 8
        );
        assert_eq!(
            mcpwm
                .read(UPDATE_CFG, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(UPDATE_CFG_RESET)
        );
        assert_eq!(
            mcpwm
                .read(VERSION, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(VERSION_RESET)
        );

        for (offset, expected) in [
            (CLK_CFG, CLOCK_CFG_MASK),
            (0x04, TIMER_CFG0_MASK),
            (0x08, TIMER_CFG1_MASK),
            (0x0c, TIMER_SYNC_MASK),
            (TIMER_SYNC_INPUT_CFG, TIMER_SYNC_INPUT_MASK),
            (OP_TIMER_SEL, OP_TIMER_SEL_MASK),
            (OP_BASE, GEN_STMP_CFG_MASK),
            (OP_BASE + 0x04, GEN_TIMESTAMP_MASK),
            (OP_BASE + 0x0c, GEN_CFG0_MASK),
            (OP_BASE + 0x10, GEN_FORCE_MASK),
            (OP_BASE + 0x14, GEN_ACTION_MASK),
            (OP_BASE + 0x18, GEN_ACTION_MASK),
            (UPDATE_CFG, UPDATE_CFG_MASK),
            (INT_ENA, INTERRUPT_MASK),
            (CLK, CLOCK_MASK),
            (VERSION, VERSION_MASK),
        ] {
            mcpwm
                .write(
                    offset,
                    AccessWidth::Word,
                    u64::from(u32::MAX),
                    SimTime::ZERO,
                )
                .unwrap();
            assert_eq!(
                mcpwm
                    .read(offset, AccessWidth::Word, SimTime::ZERO)
                    .unwrap(),
                u64::from(expected)
            );
        }
        assert!(
            mcpwm
                .write(0x10, AccessWidth::Word, 0, SimTime::ZERO)
                .is_err()
        );
        assert!(
            mcpwm
                .write(INT_ST, AccessWidth::Word, 0, SimTime::ZERO)
                .is_err()
        );
        assert!(
            mcpwm
                .read(INT_CLR, AccessWidth::Word, SimTime::ZERO)
                .is_err()
        );
        assert!(
            mcpwm
                .write(
                    CLK_CFG,
                    AccessWidth::Word,
                    u64::from(u32::MAX) + 1,
                    SimTime::ZERO
                )
                .is_err()
        );
    }

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
                OP_BASE + 0x10,
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
            .write(INT_ENA, AccessWidth::Word, 1 << 6, SimTime::ZERO)
            .unwrap();
        handle.poll(SimTime::from_ticks(3)).unwrap();
        assert_eq!(
            mcpwm
                .read(INT_ST, AccessWidth::Word, SimTime::from_ticks(3))
                .unwrap(),
            1 << 6
        );
        mcpwm
            .write(INT_CLR, AccessWidth::Word, 1 << 6, SimTime::from_ticks(3))
            .unwrap();
        assert_eq!(
            mcpwm
                .read(INT_RAW, AccessWidth::Word, SimTime::from_ticks(3))
                .unwrap(),
            0
        );
    }
}
