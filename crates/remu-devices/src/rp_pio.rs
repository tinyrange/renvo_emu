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
    irq: u8,
    irq0_inte: u16,
    irq0_intf: u16,
    irq1_inte: u16,
    irq1_intf: u16,
    input_sync_bypass: u32,
    gpio_base: u32,
    output: u32,
    direction: u32,
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
            irq: 0,
            irq0_inte: 0,
            irq0_intf: 0,
            irq1_inte: 0,
            irq1_intf: 0,
            input_sync_bypass: 0,
            gpio_base: 0,
            output: 0,
            direction: 0,
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
    ///
    /// PIO clock dividers and delay fields are deliberately interpreted as one
    /// deterministic abstract tick in the baseline model.
    pub fn poll(&self, now: SimTime) -> Result<bool, SignalError> {
        let mut state = self.state.borrow_mut();
        let before = state.output;
        for machine in 0..state.machines.len() {
            if state.control & (1 << machine) == 0 {
                continue;
            }
            let address = usize::from(state.machines[machine].address);
            let instruction = state.instructions[address];
            execute_rp_pio_instruction(&mut state, machine, instruction, true);
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
        let Some(fifo) = state.rx_fifo.get_mut(machine) else {
            return false;
        };
        if fifo.len() >= 4 {
            state.debug |= 1 << (machine + 0);
            return false;
        }
        fifo.push_back(value);
        true
    }

    /// Returns the current TX and RX FIFO levels for a state machine.
    pub fn fifo_levels(&self, machine: usize) -> Option<(usize, usize)> {
        let state = self.state.borrow();
        Some((
            state.tx_fifo.get(machine)?.len(),
            state.rx_fifo.get(machine)?.len(),
        ))
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
) {
    const JMP: u16 = 0x0000;
    const SET: u16 = 0xe000;
    let major = instruction & 0xe000;
    let argument = (instruction >> 5) & 7;
    let data = u32::from(instruction & 0x1f);
    let sm = &mut state.machines[machine];
    sm.instruction = instruction;
    let mut jumped = false;
    match major {
        JMP if argument == 0 => {
            sm.address = u8::try_from(data).expect("five-bit PIO address fits u8");
            jumped = true;
        }
        SET => {
            let base = (sm.pin_control >> 5) & 0x1f;
            let count = (sm.pin_control >> 26) & 7;
            let mask = if count == 0 {
                0
            } else {
                ((1_u32 << count) - 1).rotate_left(base)
            };
            let value = data.rotate_left(base) & mask;
            match argument {
                0 => state.output = (state.output & !mask) | value,
                1 => sm.x = data,
                2 => sm.y = data,
                4 => state.direction = (state.direction & !mask) | value,
                _ => {}
            }
        }
        _ => {}
    }
    if advance && !jumped {
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
}

/// Functional RP PIO register and execution slice.
///
/// The baseline covers instruction memory, state-machine configuration,
/// direct execution, unconditional `JMP`, `SET` to pins/directions/X/Y,
/// four-word host FIFOs, FIFO status/fault flags, and processor-facing IRQ0
/// masks. RP2350-native IRQ0/IRQ1 and GPIOBASE register placement is preserved;
/// `WAIT`, shift, side-set, DMA, exact divider timing, FIFO PUT/GET, and
/// cross-PIO PIO-v1 execution extensions remain outside this deliberately
/// small proof.
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
                    if state.tx_fifo[machine].len() >= 4 {
                        value |= 1 << (16 + machine);
                    }
                    if state.rx_fifo[machine].is_empty() {
                        value |= 1 << (8 + machine);
                    }
                    if state.rx_fifo[machine].len() >= 4 {
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
                    state.debug |= 1 << machine;
                    0
                })
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
                    RpPioStateMachineRegister::ExecCtrl => sm.execution_control,
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
                }
                RpPioRegister::Fstat | RpPioRegister::Flevel => {
                    return Err(DeviceError::new("RP PIO FIFO status is read-only"));
                }
                RpPioRegister::Fdebug => state.debug &= !(value & 0x0f0f_0f0f),
                RpPioRegister::Txf(machine) => {
                    if alias != 0 {
                        return Err(DeviceError::new("RP PIO TX FIFO does not support aliases"));
                    }
                    if state.tx_fifo[machine].len() < 4 {
                        state.tx_fifo[machine].push_back(value);
                    } else {
                        state.debug |= 1 << (16 + machine);
                    }
                }
                RpPioRegister::Rxf(_) => {
                    return Err(DeviceError::new("RP PIO RX FIFO is read-only"));
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
                            RpPioVersion::Rp2040 => 0xffff_ff9f,
                            RpPioVersion::Rp2350 => 0xffff_ffff,
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
                        if updated & 0xc000_0000 != current & 0xc000_0000 {
                            state.tx_fifo[machine].clear();
                            state.rx_fifo[machine].clear();
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
                        execute_rp_pio_instruction(&mut state, machine, instruction, false);
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
