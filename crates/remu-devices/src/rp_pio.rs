use super::*;

#[derive(Clone, Copy)]
struct RpPioStateMachine {
    clock_divider: u32,
    execution_control: u32,
    shift_control: u32,
    address: u8,
    instruction: u16,
    pin_control: u32,
    x: u32,
    y: u32,
    input_shift: u32,
    output_shift: u32,
    input_shift_count: u8,
    output_shift_count: u8,
    clock_accumulator: u32,
    delay_remaining: u8,
    stalled: bool,
    forced_instruction: Option<u16>,
    irq_wait_set: bool,
    pending_push: bool,
}

impl RpPioStateMachine {
    const fn reset() -> Self {
        Self {
            clock_divider: 0x0001_0000,
            execution_control: 0x0001_f000,
            shift_control: 0x000c_0000,
            address: 0,
            instruction: 0,
            pin_control: 0x1400_0000,
            x: 0,
            y: 0,
            input_shift: 0,
            output_shift: 0,
            input_shift_count: 0,
            output_shift_count: 32,
            clock_accumulator: 0,
            delay_remaining: 0,
            stalled: false,
            forced_instruction: None,
            irq_wait_set: false,
            pending_push: false,
        }
    }
}

/// RP PIO hardware generation used for reset values and configuration metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpPioVersion {
    /// RP2040 PIO version 0.
    Rp2040,
    /// RP2350 PIO version 1.
    Rp2350,
}

/// Named register identifiers for the common RP2040/RP2350 PIO surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpPioRegister {
    /// State-machine enable and restart control.
    Ctrl,
    /// FIFO empty/full flags.
    Fstat,
    /// Sticky FIFO fault flags.
    Fdebug,
    /// FIFO levels.
    Flevel,
    /// State-machine transmit FIFO.
    Txf(usize),
    /// State-machine receive FIFO.
    Rxf(usize),
    /// RP2350 random-access RX FIFO storage word.
    RxfPutGet {
        /// State-machine index.
        machine: usize,
        /// Random-access storage entry.
        entry: usize,
    },
    /// Internal state-machine IRQ flags.
    Irq,
    /// Internal IRQ force strobe.
    IrqForce,
    /// GPIO input synchronizer bypass.
    InputSyncBypass,
    /// RP2350 GPIO base selector.
    GpioBase,
    /// Current PIO pad output values.
    DbgPadout,
    /// Current PIO pad output-enable values.
    DbgPadoe,
    /// PIO implementation configuration.
    DbgCfginfo,
    /// Instruction memory word.
    InstrMem(usize),
    /// Raw processor-facing interrupt status.
    Intr,
    /// IRQ0 interrupt enable.
    Irq0Inte,
    /// IRQ0 interrupt force.
    Irq0Intf,
    /// IRQ0 masked interrupt status.
    Irq0Ints,
    /// RP2350 IRQ1 interrupt enable.
    Irq1Inte,
    /// RP2350 IRQ1 interrupt force.
    Irq1Intf,
    /// RP2350 IRQ1 masked interrupt status.
    Irq1Ints,
    /// One state-machine register.
    StateMachine {
        /// State-machine index.
        machine: usize,
        /// Register within that state machine.
        register: RpPioStateMachineRegister,
    },
}

/// Named state-machine register identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpPioStateMachineRegister {
    /// Clock divider.
    ClockDiv,
    /// Execution control.
    ExecCtrl,
    /// Shift control.
    ShiftCtrl,
    /// Current instruction address.
    Addr,
    /// Current/execute instruction.
    Instr,
    /// Pin mapping control.
    PinCtrl,
}

impl RpPioRegister {
    /// Decodes a native PIO offset, excluding atomic alias bits.
    pub fn try_from_offset(offset: u64) -> Result<Self, DeviceError> {
        Self::try_from_offset_for_version(offset, RpPioVersion::Rp2040)
    }

    /// Decodes a native PIO offset for one hardware generation, excluding
    /// atomic alias bits. RP2350 inserts the FIFO PUT/GET and GPIOBASE
    /// registers before the processor interrupt registers, so those offsets
    /// must not be interpreted as RP2040 IRQ registers.
    pub fn try_from_offset_for_version(
        offset: u64,
        version: RpPioVersion,
    ) -> Result<Self, DeviceError> {
        let offset = offset & 0x0fff;
        let register = match offset {
            0x000 => Self::Ctrl,
            0x004 => Self::Fstat,
            0x008 => Self::Fdebug,
            0x00c => Self::Flevel,
            0x010..=0x01c => Self::Txf(usize::try_from((offset - 0x010) / 4).unwrap()),
            0x020..=0x02c => Self::Rxf(usize::try_from((offset - 0x020) / 4).unwrap()),
            0x030 => Self::Irq,
            0x034 => Self::IrqForce,
            0x038 => Self::InputSyncBypass,
            0x03c => Self::DbgPadout,
            0x040 => Self::DbgPadoe,
            0x044 => Self::DbgCfginfo,
            0x048..=0x0c4 => Self::InstrMem(usize::try_from((offset - 0x048) / 4).unwrap()),
            0x0c8..=0x124 => {
                let machine = usize::try_from((offset - 0x0c8) / 0x18).unwrap();
                let register = match (offset - 0x0c8) % 0x18 {
                    0x00 => RpPioStateMachineRegister::ClockDiv,
                    0x04 => RpPioStateMachineRegister::ExecCtrl,
                    0x08 => RpPioStateMachineRegister::ShiftCtrl,
                    0x0c => RpPioStateMachineRegister::Addr,
                    0x10 => RpPioStateMachineRegister::Instr,
                    0x14 => RpPioStateMachineRegister::PinCtrl,
                    _ => {
                        return Err(DeviceError::new(format!(
                            "unmodeled RP PIO state-machine register at {offset:#x}"
                        )));
                    }
                };
                Self::StateMachine { machine, register }
            }
            0x128..=0x164 if version == RpPioVersion::Rp2350 => {
                let index = usize::try_from((offset - 0x128) / 4).unwrap();
                Self::RxfPutGet {
                    machine: index / 4,
                    entry: index % 4,
                }
            }
            0x128 if version == RpPioVersion::Rp2040 => Self::Intr,
            0x12c if version == RpPioVersion::Rp2040 => Self::Irq0Inte,
            0x130 if version == RpPioVersion::Rp2040 => Self::Irq0Intf,
            0x134 if version == RpPioVersion::Rp2040 => Self::Irq0Ints,
            0x168 if version == RpPioVersion::Rp2350 => Self::GpioBase,
            0x16c if version == RpPioVersion::Rp2350 => Self::Intr,
            0x170 if version == RpPioVersion::Rp2350 => Self::Irq0Inte,
            0x174 if version == RpPioVersion::Rp2350 => Self::Irq0Intf,
            0x178 if version == RpPioVersion::Rp2350 => Self::Irq0Ints,
            0x17c if version == RpPioVersion::Rp2350 => Self::Irq1Inte,
            0x180 if version == RpPioVersion::Rp2350 => Self::Irq1Intf,
            0x184 if version == RpPioVersion::Rp2350 => Self::Irq1Ints,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP PIO register at {offset:#x}"
                )));
            }
        };
        Ok(register)
    }

    /// Returns the native offset for this register identifier.
    pub fn offset(self) -> u64 {
        self.offset_for_version(RpPioVersion::Rp2040)
    }

    /// Returns the native offset for this register identifier and hardware
    /// generation.
    pub fn offset_for_version(self, version: RpPioVersion) -> u64 {
        match self {
            Self::Ctrl => 0x000,
            Self::Fstat => 0x004,
            Self::Fdebug => 0x008,
            Self::Flevel => 0x00c,
            Self::Txf(machine) => 0x010 + machine as u64 * 4,
            Self::Rxf(machine) => 0x020 + machine as u64 * 4,
            Self::RxfPutGet { machine, entry } => 0x128 + machine as u64 * 0x10 + entry as u64 * 4,
            Self::Irq => 0x030,
            Self::IrqForce => 0x034,
            Self::InputSyncBypass => 0x038,
            Self::GpioBase => 0x168,
            Self::DbgPadout => 0x03c,
            Self::DbgPadoe => 0x040,
            Self::DbgCfginfo => 0x044,
            Self::InstrMem(index) => 0x048 + index as u64 * 4,
            Self::Intr => match version {
                RpPioVersion::Rp2040 => 0x128,
                RpPioVersion::Rp2350 => 0x16c,
            },
            Self::Irq0Inte => match version {
                RpPioVersion::Rp2040 => 0x12c,
                RpPioVersion::Rp2350 => 0x170,
            },
            Self::Irq0Intf => match version {
                RpPioVersion::Rp2040 => 0x130,
                RpPioVersion::Rp2350 => 0x174,
            },
            Self::Irq0Ints => match version {
                RpPioVersion::Rp2040 => 0x134,
                RpPioVersion::Rp2350 => 0x178,
            },
            Self::Irq1Inte => 0x17c,
            Self::Irq1Intf => 0x180,
            Self::Irq1Ints => 0x184,
            Self::StateMachine { machine, register } => {
                0x0c8
                    + machine as u64 * 0x18
                    + match register {
                        RpPioStateMachineRegister::ClockDiv => 0x00,
                        RpPioStateMachineRegister::ExecCtrl => 0x04,
                        RpPioStateMachineRegister::ShiftCtrl => 0x08,
                        RpPioStateMachineRegister::Addr => 0x0c,
                        RpPioStateMachineRegister::Instr => 0x10,
                        RpPioStateMachineRegister::PinCtrl => 0x14,
                    }
            }
        }
    }
}

struct RpPioState {
    version: RpPioVersion,
    control: u32,
    debug: u32,
    instructions: [u16; 32],
    machines: [RpPioStateMachine; 4],
    tx_fifo: [VecDeque<u32>; 4],
    rx_fifo: [VecDeque<u32>; 4],
    putget: [[u32; 4]; 4],
    irq: u8,
    irq0_inte: u16,
    irq0_intf: u16,
    irq1_inte: u16,
    irq1_intf: u16,
    input_sync_bypass: u32,
    gpio_base: u32,
    output: u32,
    direction: u32,
    input: u32,
}

impl RpPioState {
    fn reset(version: RpPioVersion) -> Self {
        Self {
            version,
            control: 0,
            debug: 0,
            instructions: [0; 32],
            machines: [RpPioStateMachine::reset(); 4],
            tx_fifo: std::array::from_fn(|_| VecDeque::new()),
            rx_fifo: std::array::from_fn(|_| VecDeque::new()),
            putget: [[0; 4]; 4],
            irq: 0,
            irq0_inte: 0,
            irq0_intf: 0,
            irq1_inte: 0,
            irq1_intf: 0,
            input_sync_bypass: 0,
            gpio_base: 0,
            output: 0,
            direction: 0,
            input: 0,
        }
    }
}

/// Scheduler-facing handle for a functional Raspberry Pi PIO block.
#[derive(Clone)]
pub struct RpPioHandle {
    state: Rc<RefCell<RpPioState>>,
    hub: SignalHub,
    output_signal: SignalId,
    pins: u16,
}

impl RpPioHandle {
    /// Executes one instruction on each enabled state machine.
    pub fn poll(&self, now: SimTime) -> Result<bool, SignalError> {
        let mut state = self.state.borrow_mut();
        let before = state.output;
        for machine in 0..state.machines.len() {
            if state.control & (1 << machine) == 0 {
                continue;
            }
            let divider = pio_clock_divider(state.machines[machine].clock_divider);
            let accumulator = state.machines[machine]
                .clock_accumulator
                .saturating_add(0x100);
            if accumulator < divider {
                state.machines[machine].clock_accumulator = accumulator;
                continue;
            }
            state.machines[machine].clock_accumulator = accumulator - divider;
            if state.machines[machine].delay_remaining != 0 {
                state.machines[machine].delay_remaining -= 1;
                continue;
            }
            let forced = state.machines[machine].forced_instruction;
            let instruction = forced.unwrap_or_else(|| {
                state.instructions[usize::from(state.machines[machine].address)]
            });
            let completed =
                execute_rp_pio_instruction(&mut state, machine, instruction, forced.is_none());
            state.machines[machine].stalled = !completed;
            if completed && forced.is_some() {
                state.machines[machine].forced_instruction = None;
            }
        }
        if state.output != before {
            self.hub.set(
                self.output_signal,
                SignalValue::from_u64(u64::from(state.output), self.pins)?,
                now,
            )?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Injects one word into a state machine's RX FIFO.
    pub fn inject_rx(&self, machine: usize, value: u32) -> bool {
        let mut state = self.state.borrow_mut();
        if machine >= state.rx_fifo.len() || rx_capacity(&state, machine) == 0 {
            return false;
        }
        if state.rx_fifo[machine].len() >= rx_capacity(&state, machine) {
            state.debug |= 1 << (machine + 0);
            return false;
        }
        state.rx_fifo[machine].push_back(value);
        true
    }

    /// Updates the synchronized PIO input window used by WAIT, IN, MOV and JMP PIN.
    pub fn set_inputs(&self, value: u32) {
        self.state.borrow_mut().input = value;
    }

    /// Returns whether the state machine is requesting a transmit DMA word.
    pub fn tx_dreq(&self, machine: usize) -> bool {
        let state = self.state.borrow();
        machine < 4
            && tx_capacity(&state, machine) != 0
            && state.tx_fifo[machine].len() < tx_capacity(&state, machine)
    }

    /// Returns whether the state machine has a receive word for DMA.
    pub fn rx_dreq(&self, machine: usize) -> bool {
        self.state
            .borrow()
            .rx_fifo
            .get(machine)
            .is_some_and(|fifo| !fifo.is_empty())
    }

    /// Returns the current TX and RX FIFO levels for a state machine.
    pub fn fifo_levels(&self, machine: usize) -> Option<(usize, usize)> {
        let state = self.state.borrow();
        Some((
            state.tx_fifo.get(machine)?.len(),
            state.rx_fifo.get(machine)?.len(),
        ))
    }

    /// Returns the output value, output-enable mask, and RP2350 GPIO window base.
    pub fn pad_state(&self) -> (u32, u32, u8) {
        let state = self.state.borrow();
        (state.output, state.direction, state.gpio_base as u8)
    }

    /// Returns the masked processor-facing IRQ0 status.
    pub fn pending_interrupts(&self) -> u16 {
        let state = self.state.borrow();
        let raw = raw_interrupts(&state);
        (raw & state.irq0_inte) | state.irq0_intf
    }

    /// Returns the masked RP2350 IRQ1 status.
    pub fn pending_interrupts_1(&self) -> u16 {
        let state = self.state.borrow();
        let raw = raw_interrupts(&state);
        (raw & state.irq1_inte) | state.irq1_intf
    }
}

fn tx_capacity(state: &RpPioState, machine: usize) -> usize {
    let shift = state.machines[machine].shift_control;
    if shift & (1 << 31) != 0 {
        0
    } else if shift & (1 << 30) != 0 {
        8
    } else {
        4
    }
}

fn rx_capacity(state: &RpPioState, machine: usize) -> usize {
    let shift = state.machines[machine].shift_control;
    if shift & (1 << 30) != 0 {
        0
    } else if shift & (1 << 31) != 0 {
        8
    } else {
        4
    }
}

fn pio_clock_divider(register: u32) -> u32 {
    let fixed = register >> 8;
    if fixed == 0 { 1 << 24 } else { fixed }
}

fn raw_interrupts(state: &RpPioState) -> u16 {
    let mut raw = 0_u16;
    for machine in 0..4 {
        if !state.rx_fifo[machine].is_empty() {
            raw |= 1 << machine;
        }
        if state.tx_fifo[machine].len() < 4 {
            raw |= 1 << (machine + 4);
        }
        if state.irq & (1 << machine) != 0 {
            raw |= 1 << (machine + 8);
        }
    }
    raw
}

fn execute_rp_pio_instruction(
    state: &mut RpPioState,
    machine: usize,
    instruction: u16,
    advance: bool,
) -> bool {
    const JMP: u16 = 0x0000;
    const WAIT: u16 = 0x2000;
    const IN: u16 = 0x4000;
    const OUT: u16 = 0x6000;
    const PUSH_PULL: u16 = 0x8000;
    const MOV: u16 = 0xa000;
    const IRQ: u16 = 0xc000;
    const SET: u16 = 0xe000;
    let major = instruction & 0xe000;
    let argument = (instruction >> 5) & 7;
    let data = u32::from(instruction & 0x1f);
    let mut sm = state.machines[machine];
    sm.instruction = instruction;
    let mut jumped = false;
    let mut completed = true;

    let side_count = (sm.pin_control >> 29) & 7;
    let delay_and_side = u32::from((instruction >> 8) & 0x1f);
    let optional_side = sm.execution_control & (1 << 30) != 0;
    let side_enabled = !optional_side || delay_and_side & 0x10 != 0;
    let actual_side_count = side_count.saturating_sub(u32::from(optional_side));
    let delay_bits = 5_u32.saturating_sub(side_count);
    let delay_mask = if delay_bits == 0 {
        0
    } else {
        (1_u32 << delay_bits) - 1
    };
    let delay = (delay_and_side & delay_mask) as u8;
    if side_count != 0 && side_enabled {
        let side_value = (delay_and_side >> delay_bits) & bit_mask(actual_side_count);
        let side_base = (sm.pin_control >> 10) & 0x1f;
        if sm.execution_control & (1 << 29) != 0 {
            write_pin_range(
                &mut state.direction,
                side_base,
                actual_side_count,
                side_value,
            );
        } else {
            write_pin_range(&mut state.output, side_base, actual_side_count, side_value);
        }
    }

    if sm.pending_push {
        if push_isr(state, machine, &mut sm) {
            sm.pending_push = false;
        } else {
            state.machines[machine] = sm;
            return false;
        }
    } else {
        match major {
            JMP => {
                let condition = match argument {
                    0 => true,
                    1 => sm.x == 0,
                    2 => {
                        let condition = sm.x != 0;
                        sm.x = sm.x.wrapping_sub(1);
                        condition
                    }
                    3 => sm.y == 0,
                    4 => {
                        let condition = sm.y != 0;
                        sm.y = sm.y.wrapping_sub(1);
                        condition
                    }
                    5 => sm.x != sm.y,
                    6 => {
                        let pin = ((sm.execution_control >> 24) & 0x1f) as u8;
                        state.input & (1_u32 << pin) != 0
                    }
                    7 => sm.output_shift_count >= pull_threshold(sm.shift_control),
                    _ => unreachable!("three-bit PIO JMP condition"),
                };
                if condition {
                    sm.address = data as u8;
                    jumped = true;
                }
            }
            WAIT => {
                let polarity = argument & 4 != 0;
                let source = argument & 3;
                let index = data & 0xf;
                let level = match source {
                    0 => state.input & (1_u32 << (data & 0x1f)) != 0,
                    1 => {
                        let base = (sm.pin_control >> 15) & 0x1f;
                        state.input & (1_u32 << ((base + data) & 0x1f)) != 0
                    }
                    2 => {
                        let irq = if data & 0x10 != 0 {
                            (index + machine as u32) & 7
                        } else {
                            index & 7
                        };
                        state.irq & (1 << irq) != 0
                    }
                    // PIO v1 adds JMPPIN as WAIT source 3. Treating it as the
                    // configured JMP pin also gives deterministic behavior when
                    // a v0 program accidentally emits the reserved encoding.
                    3 => {
                        let pin = (sm.execution_control >> 24) & 0x1f;
                        state.input & (1_u32 << pin) != 0
                    }
                    _ => unreachable!(),
                };
                completed = level == polarity;
            }
            IN => {
                let count = shift_count(data);
                let source = match argument {
                    0 => {
                        let base = (sm.pin_control >> 15) & 0x1f;
                        state.input.rotate_right(base) & bit_mask(count)
                    }
                    1 => sm.x & bit_mask(count),
                    2 => sm.y & bit_mask(count),
                    3 => 0,
                    6 => sm.input_shift & bit_mask(count),
                    7 => sm.output_shift & bit_mask(count),
                    _ => 0,
                };
                shift_into_isr(&mut sm, source, count);
                if sm.shift_control & (1 << 16) != 0
                    && sm.input_shift_count >= push_threshold(sm.shift_control)
                    && !push_isr(state, machine, &mut sm)
                {
                    sm.pending_push = true;
                    completed = false;
                }
            }
            OUT => {
                if sm.shift_control & (1 << 17) != 0
                    && sm.output_shift_count >= pull_threshold(sm.shift_control)
                    && !pull_osr(state, machine, &mut sm, true)
                {
                    completed = false;
                }
                if completed {
                    let count = shift_count(data);
                    let value = shift_from_osr(&mut sm, count);
                    match argument {
                        0 => write_pin_range(
                            &mut state.output,
                            sm.pin_control & 0x1f,
                            (sm.pin_control >> 20) & 0x3f,
                            value,
                        ),
                        1 => sm.x = value,
                        2 => sm.y = value,
                        3 => {}
                        4 => write_pin_range(
                            &mut state.direction,
                            sm.pin_control & 0x1f,
                            (sm.pin_control >> 20) & 0x3f,
                            value,
                        ),
                        5 => {
                            sm.address = (value & 0x1f) as u8;
                            jumped = true;
                        }
                        6 => sm.input_shift = value,
                        7 => sm.forced_instruction = Some(value as u16),
                        _ => unreachable!(),
                    }
                }
            }
            PUSH_PULL => {
                let pull = argument & 4 != 0;
                let conditional = argument & 2 != 0;
                let block = argument & 1 != 0;
                if pull {
                    let needed =
                        !conditional || sm.output_shift_count >= pull_threshold(sm.shift_control);
                    if needed {
                        completed = pull_osr(state, machine, &mut sm, block);
                    }
                } else {
                    let needed =
                        !conditional || sm.input_shift_count >= push_threshold(sm.shift_control);
                    if needed {
                        completed = push_isr(state, machine, &mut sm);
                        if !completed && !block {
                            completed = true;
                        }
                    }
                }
            }
            MOV => {
                let source = match data & 7 {
                    0 => {
                        let base = (sm.pin_control >> 15) & 0x1f;
                        state.input.rotate_right(base)
                    }
                    1 => sm.x,
                    2 => sm.y,
                    3 => 0,
                    5 => {
                        let level = if sm.execution_control & (1 << 4) == 0 {
                            state.tx_fifo[machine].len()
                        } else {
                            state.rx_fifo[machine].len()
                        };
                        u32::from(level < (sm.execution_control & 0xf) as usize)
                    }
                    6 => sm.input_shift,
                    7 => sm.output_shift,
                    _ => 0,
                };
                let value = match (data >> 3) & 3 {
                    0 => source,
                    1 => !source,
                    2 => source.reverse_bits(),
                    _ => source,
                };
                match argument {
                    0 => write_pin_range(&mut state.output, 0, 32, value),
                    1 => sm.x = value,
                    2 => sm.y = value,
                    4 => sm.forced_instruction = Some(value as u16),
                    5 => {
                        sm.address = (value & 0x1f) as u8;
                        jumped = true;
                    }
                    6 => sm.input_shift = value,
                    7 => sm.output_shift = value,
                    _ => {}
                }
            }
            IRQ => {
                let irq = if data & 0x10 != 0 {
                    ((data & 7) + machine as u32) & 7
                } else {
                    data & 7
                };
                let mask = 1_u8 << irq;
                match argument & 3 {
                    0 => state.irq |= mask,
                    1 => state.irq &= !mask,
                    2 => {
                        if !sm.irq_wait_set {
                            state.irq |= mask;
                            sm.irq_wait_set = true;
                        }
                        if state.irq & mask != 0 {
                            completed = false;
                        } else {
                            sm.irq_wait_set = false;
                        }
                    }
                    _ => {}
                }
            }
            SET => {
                let base = (sm.pin_control >> 5) & 0x1f;
                let count = (sm.pin_control >> 26) & 7;
                match argument {
                    0 => write_pin_range(&mut state.output, base, count, data),
                    1 => sm.x = data,
                    2 => sm.y = data,
                    4 => write_pin_range(&mut state.direction, base, count, data),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if completed {
        sm.delay_remaining = delay;
    }
    if completed && advance && !jumped {
        let wrap_top = u8::try_from((sm.execution_control >> 12) & 0x1f)
            .expect("five-bit PIO wrap address fits u8");
        let wrap_bottom = u8::try_from((sm.execution_control >> 7) & 0x1f)
            .expect("five-bit PIO wrap address fits u8");
        sm.address = if sm.address == wrap_top {
            wrap_bottom
        } else {
            sm.address.wrapping_add(1) & 0x1f
        };
    }
    state.machines[machine] = sm;
    completed
}

fn bit_mask(count: u32) -> u32 {
    match count {
        0 => 0,
        32.. => u32::MAX,
        _ => (1_u32 << count) - 1,
    }
}

fn shift_count(encoded: u32) -> u32 {
    if encoded == 0 { 32 } else { encoded }
}

fn push_threshold(shift_control: u32) -> u8 {
    let encoded = ((shift_control >> 20) & 0x1f) as u8;
    if encoded == 0 { 32 } else { encoded }
}

fn pull_threshold(shift_control: u32) -> u8 {
    let encoded = ((shift_control >> 25) & 0x1f) as u8;
    if encoded == 0 { 32 } else { encoded }
}

fn write_pin_range(target: &mut u32, base: u32, count: u32, value: u32) {
    let count = count.min(32);
    let mask = bit_mask(count).rotate_left(base & 0x1f);
    *target = (*target & !mask) | (value & bit_mask(count)).rotate_left(base & 0x1f) & mask;
}

fn shift_into_isr(sm: &mut RpPioStateMachine, value: u32, count: u32) {
    let value = value & bit_mask(count);
    if sm.shift_control & (1 << 18) != 0 {
        sm.input_shift = if count == 32 {
            value
        } else {
            (sm.input_shift >> count) | (value << (32 - count))
        };
    } else {
        sm.input_shift = if count == 32 {
            value
        } else {
            (sm.input_shift << count) | value
        };
    }
    sm.input_shift_count = sm.input_shift_count.saturating_add(count as u8).min(32);
}

fn shift_from_osr(sm: &mut RpPioStateMachine, count: u32) -> u32 {
    let value;
    if sm.shift_control & (1 << 19) != 0 {
        value = sm.output_shift & bit_mask(count);
        sm.output_shift = if count == 32 {
            0
        } else {
            sm.output_shift >> count
        };
    } else {
        value = if count == 32 {
            sm.output_shift
        } else {
            sm.output_shift >> (32 - count)
        };
        sm.output_shift = if count == 32 {
            0
        } else {
            sm.output_shift << count
        };
    }
    sm.output_shift_count = sm.output_shift_count.saturating_add(count as u8).min(32);
    value
}

fn push_isr(state: &mut RpPioState, machine: usize, sm: &mut RpPioStateMachine) -> bool {
    let capacity = rx_capacity(state, machine);
    if capacity == 0 || state.rx_fifo[machine].len() >= capacity {
        state.debug |= 1 << machine;
        return false;
    }
    state.rx_fifo[machine].push_back(sm.input_shift);
    sm.input_shift = 0;
    sm.input_shift_count = 0;
    true
}

fn pull_osr(
    state: &mut RpPioState,
    machine: usize,
    sm: &mut RpPioStateMachine,
    block: bool,
) -> bool {
    if let Some(value) = state.tx_fifo[machine].pop_front() {
        sm.output_shift = value;
        sm.output_shift_count = 0;
        true
    } else if block {
        state.debug |= 1 << (24 + machine);
        false
    } else {
        sm.output_shift = sm.x;
        sm.output_shift_count = 0;
        true
    }
}

/// Functional RP PIO register and execution slice.
///
/// The baseline covers instruction memory, state-machine configuration,
/// direct execution, unconditional `JMP`, `SET` to pins/directions/X/Y,
/// four-word host FIFOs, FIFO status/fault flags, and processor-facing IRQ0
/// masks. The execution engine covers all eight instruction families,
/// wrap/delay/side-set behavior, 16.8 clock-divider pacing, shift registers,
/// joined FIFOs, automatic and explicit push/pull stalls, and DREQ state.
/// RP2350-native IRQ0/IRQ1 and GPIOBASE register placement is preserved.
pub struct RpPio {
    name: String,
    state: Rc<RefCell<RpPioState>>,
    hub: SignalHub,
    output_signal: SignalId,
    pins: u16,
}

impl RpPio {
    /// Creates a reset PIO block and scheduler handle.
    pub fn new(
        name: impl Into<String>,
        pins: u16,
        signal_path: &str,
        hub: SignalHub,
    ) -> Result<(Self, RpPioHandle), SignalError> {
        Self::new_with_version(name, pins, signal_path, hub, RpPioVersion::Rp2040)
    }

    /// Creates a PIO block with the selected hardware-generation metadata.
    pub fn new_with_version(
        name: impl Into<String>,
        pins: u16,
        signal_path: &str,
        hub: SignalHub,
        version: RpPioVersion,
    ) -> Result<(Self, RpPioHandle), SignalError> {
        let output_signal = hub.declare(
            signal_path,
            SignalValue::from_u64(0, pins)?,
            Some("Functional PIO output register".to_owned()),
        )?;
        let state = Rc::new(RefCell::new(RpPioState::reset(version)));
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
                hub: hub.clone(),
                output_signal,
                pins,
            },
            RpPioHandle {
                state,
                hub,
                output_signal,
                pins,
            },
        ))
    }

    fn update_register(current: u32, alias: u64, value: u32) -> u32 {
        match alias {
            0 => value,
            1 => current ^ value,
            2 => current | value,
            3 => current & !value,
            _ => unreachable!("two-bit RP atomic alias"),
        }
    }

    fn publish_output(&self, at: SimTime) -> Result<(), DeviceError> {
        let output = self.state.borrow().output;
        self.hub
            .set(
                self.output_signal,
                SignalValue::from_u64(u64::from(output), self.pins)
                    .map_err(|error| DeviceError::new(error.to_string()))?,
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }
}

impl Device for RpPio {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset % 4 != 0 {
            return Err(DeviceError::new("RP PIO requires aligned word access"));
        }
        let version = self.state.borrow().version;
        let register = RpPioRegister::try_from_offset_for_version(offset, version)?;
        let mut state = self.state.borrow_mut();
        let value = match register {
            RpPioRegister::Ctrl => state.control,
            RpPioRegister::Fstat => {
                let mut value = 0;
                for machine in 0..4 {
                    if state.tx_fifo[machine].is_empty() {
                        value |= 1 << (24 + machine);
                    }
                    if tx_capacity(&state, machine) == 0
                        || state.tx_fifo[machine].len() >= tx_capacity(&state, machine)
                    {
                        value |= 1 << (16 + machine);
                    }
                    if state.rx_fifo[machine].is_empty() {
                        value |= 1 << (8 + machine);
                    }
                    if rx_capacity(&state, machine) == 0
                        || state.rx_fifo[machine].len() >= rx_capacity(&state, machine)
                    {
                        value |= 1 << machine;
                    }
                }
                value
            }
            RpPioRegister::Fdebug => state.debug,
            RpPioRegister::Flevel => {
                let mut value = 0;
                for machine in 0..4 {
                    value |= (state.tx_fifo[machine].len() as u32) << (8 * machine);
                    value |= (state.rx_fifo[machine].len() as u32) << (8 * machine + 4);
                }
                value
            }
            RpPioRegister::Txf(_) => 0,
            RpPioRegister::Rxf(machine) => {
                state.rx_fifo[machine].pop_front().unwrap_or_else(|| {
                    // FDEBUG_RXUNDER occupies bits 8..11.  The low nibble is
                    // RXSTALL, which is a distinct condition raised by the
                    // state machine when PUSH/IN encounters a full FIFO.
                    state.debug |= 1 << (8 + machine);
                    0
                })
            }
            RpPioRegister::RxfPutGet { machine, entry } => {
                let shift = state.machines[machine].shift_control;
                if shift & (1 << 15) == 0 || shift & (1 << 14) != 0 {
                    return Err(DeviceError::new(
                        "RP2350 PIO PUTGET entry is not processor-readable",
                    ));
                }
                state.putget[machine][entry]
            }
            RpPioRegister::Irq => u32::from(state.irq),
            RpPioRegister::IrqForce => 0,
            RpPioRegister::InputSyncBypass => state.input_sync_bypass,
            RpPioRegister::GpioBase => state.gpio_base,
            RpPioRegister::DbgPadout => state.output,
            RpPioRegister::DbgPadoe => state.direction,
            RpPioRegister::DbgCfginfo => {
                let version = match state.version {
                    RpPioVersion::Rp2040 => 0,
                    RpPioVersion::Rp2350 => 1,
                };
                (version << 28) | (32 << 16) | (4 << 8) | 4
            }
            RpPioRegister::InstrMem(_) => {
                return Err(DeviceError::new("RP PIO instruction memory is write-only"));
            }
            RpPioRegister::Intr => u32::from(raw_interrupts(&state)),
            RpPioRegister::Irq0Inte => u32::from(state.irq0_inte),
            RpPioRegister::Irq0Intf => u32::from(state.irq0_intf),
            RpPioRegister::Irq0Ints => {
                u32::from(raw_interrupts(&state) & state.irq0_inte | state.irq0_intf)
            }
            RpPioRegister::Irq1Inte => u32::from(state.irq1_inte),
            RpPioRegister::Irq1Intf => u32::from(state.irq1_intf),
            RpPioRegister::Irq1Ints => {
                u32::from(raw_interrupts(&state) & state.irq1_inte | state.irq1_intf)
            }
            RpPioRegister::StateMachine { machine, register } => {
                let sm = state.machines[machine];
                match register {
                    RpPioStateMachineRegister::ClockDiv => sm.clock_divider,
                    RpPioStateMachineRegister::ExecCtrl => {
                        sm.execution_control | (u32::from(sm.stalled) << 31)
                    }
                    RpPioStateMachineRegister::ShiftCtrl => sm.shift_control,
                    RpPioStateMachineRegister::Addr => u32::from(sm.address),
                    RpPioStateMachineRegister::Instr => u32::from(sm.instruction),
                    RpPioStateMachineRegister::PinCtrl => sm.pin_control,
                }
            }
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
        if width != AccessWidth::Word || offset % 4 != 0 {
            return Err(DeviceError::new("RP PIO requires aligned word access"));
        }
        let version = self.state.borrow().version;
        let register = RpPioRegister::try_from_offset_for_version(offset, version)?;
        let alias = (offset >> 12) & 3;
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked PIO register value fits u32");
        let mut publish = false;
        {
            let mut state = self.state.borrow_mut();
            match register {
                RpPioRegister::Ctrl => {
                    let enable = Self::update_register(state.control & 0xf, alias, value & 0xf);
                    state.control = enable;
                    for machine in 0..4 {
                        if value & (1 << (4 + machine)) != 0 {
                            let sm = &mut state.machines[machine];
                            sm.x = 0;
                            sm.y = 0;
                            sm.input_shift = 0;
                            sm.output_shift = 0;
                            sm.input_shift_count = 0;
                            sm.output_shift_count = 32;
                            sm.delay_remaining = 0;
                            sm.stalled = false;
                            sm.forced_instruction = None;
                            sm.irq_wait_set = false;
                            sm.pending_push = false;
                        }
                        if value & (1 << (8 + machine)) != 0 {
                            state.machines[machine].clock_accumulator = 0;
                        }
                    }
                }
                RpPioRegister::Fstat | RpPioRegister::Flevel => {
                    return Err(DeviceError::new("RP PIO FIFO status is read-only"));
                }
                RpPioRegister::Fdebug => state.debug &= !(value & 0x0f0f_0f0f),
                RpPioRegister::Txf(machine) => {
                    if alias != 0 {
                        return Err(DeviceError::new("RP PIO TX FIFO does not support aliases"));
                    }
                    let capacity = tx_capacity(&state, machine);
                    if capacity != 0 && state.tx_fifo[machine].len() < capacity {
                        state.tx_fifo[machine].push_back(value);
                    } else {
                        state.debug |= 1 << (16 + machine);
                    }
                }
                RpPioRegister::Rxf(_) => {
                    return Err(DeviceError::new("RP PIO RX FIFO is read-only"));
                }
                RpPioRegister::RxfPutGet { machine, entry } => {
                    if alias != 0 {
                        return Err(DeviceError::new(
                            "RP2350 PIO PUTGET entries do not support aliases",
                        ));
                    }
                    let shift = state.machines[machine].shift_control;
                    if shift & (1 << 14) == 0 || shift & (1 << 15) != 0 {
                        return Err(DeviceError::new(
                            "RP2350 PIO PUTGET entry is not processor-writable",
                        ));
                    }
                    state.putget[machine][entry] = value;
                }
                RpPioRegister::Irq => {
                    if alias != 0 {
                        return Err(DeviceError::new("RP PIO IRQ does not support aliases"));
                    }
                    state.irq &= !(value as u8 & 0xff);
                }
                RpPioRegister::IrqForce => {
                    if alias != 0 {
                        return Err(DeviceError::new(
                            "RP PIO IRQ_FORCE does not support aliases",
                        ));
                    }
                    state.irq |= value as u8 & 0xff;
                }
                RpPioRegister::InputSyncBypass => {
                    state.input_sync_bypass =
                        Self::update_register(state.input_sync_bypass, alias, value);
                }
                RpPioRegister::GpioBase => {
                    state.gpio_base = Self::update_register(state.gpio_base, alias, value) & 0x10;
                }
                RpPioRegister::DbgPadout
                | RpPioRegister::DbgPadoe
                | RpPioRegister::DbgCfginfo
                | RpPioRegister::Intr
                | RpPioRegister::Irq0Ints
                | RpPioRegister::Irq1Ints => {
                    return Err(DeviceError::new("RP PIO register is read-only"));
                }
                RpPioRegister::Irq0Inte => {
                    state.irq0_inte =
                        Self::update_register(u32::from(state.irq0_inte), alias, value & 0x0fff)
                            as u16;
                }
                RpPioRegister::Irq0Intf => {
                    state.irq0_intf =
                        Self::update_register(u32::from(state.irq0_intf), alias, value & 0x0fff)
                            as u16;
                }
                RpPioRegister::Irq1Inte => {
                    state.irq1_inte =
                        Self::update_register(u32::from(state.irq1_inte), alias, value & 0x0fff)
                            as u16;
                }
                RpPioRegister::Irq1Intf => {
                    state.irq1_intf =
                        Self::update_register(u32::from(state.irq1_intf), alias, value & 0x0fff)
                            as u16;
                }
                RpPioRegister::InstrMem(index) => {
                    if alias != 0 {
                        return Err(DeviceError::new(
                            "RP PIO instruction memory does not support aliases",
                        ));
                    }
                    state.instructions[index] = (value & u32::from(u16::MAX)) as u16;
                }
                RpPioRegister::StateMachine { machine, register } => match register {
                    RpPioStateMachineRegister::ClockDiv => {
                        let current = state.machines[machine].clock_divider;
                        state.machines[machine].clock_divider =
                            Self::update_register(current, alias, value) & 0xffff_ff00;
                    }
                    RpPioStateMachineRegister::ExecCtrl => {
                        let current = state.machines[machine].execution_control;
                        let mask = match state.version {
                            // EXEC_STALLED (bit 31) is read-only on both
                            // generations. RP2040 also reserves bits 7:5.
                            RpPioVersion::Rp2040 => 0x7fff_ff9f,
                            RpPioVersion::Rp2350 => 0x7fff_ffff,
                        };
                        state.machines[machine].execution_control =
                            Self::update_register(current, alias, value) & mask;
                    }
                    RpPioStateMachineRegister::ShiftCtrl => {
                        let current = state.machines[machine].shift_control;
                        let mask = match state.version {
                            RpPioVersion::Rp2040 => 0xffff_0000,
                            RpPioVersion::Rp2350 => 0xffff_c01f,
                        };
                        let updated = Self::update_register(current, alias, value) & mask;
                        if updated & 0xc000_c000 != current & 0xc000_c000 {
                            state.tx_fifo[machine].clear();
                            state.rx_fifo[machine].clear();
                            state.putget[machine] = [0; 4];
                        }
                        state.machines[machine].shift_control = updated;
                    }
                    RpPioStateMachineRegister::Addr => {
                        return Err(DeviceError::new(
                            "RP PIO state-machine address is read-only",
                        ));
                    }
                    RpPioStateMachineRegister::Instr => {
                        let before = state.output;
                        let instruction = (value & u32::from(u16::MAX)) as u16;
                        let completed =
                            execute_rp_pio_instruction(&mut state, machine, instruction, false);
                        state.machines[machine].stalled = !completed;
                        if !completed {
                            state.machines[machine].forced_instruction = Some(instruction);
                        }
                        publish = state.output != before;
                    }
                    RpPioStateMachineRegister::PinCtrl => {
                        let current = state.machines[machine].pin_control;
                        state.machines[machine].pin_control =
                            Self::update_register(current, alias, value);
                    }
                },
            }
        }
        if publish {
            self.publish_output(at)?;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let version = self.state.borrow().version;
        *self.state.borrow_mut() = RpPioState::reset(version);
        let _ = self.publish_output(SimTime::ZERO);
    }
}
