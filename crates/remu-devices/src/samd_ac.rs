use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use remu_signals::{SignalId, SignalValue};
use std::sync::{Arc, Mutex};

use super::SignalHub;

fn width_bytes(width: AccessWidth) -> usize {
    usize::from(width.bytes())
}

fn read_le(bytes: &[u8], offset: usize, width: AccessWidth) -> Result<u64, DeviceError> {
    let end = offset
        .checked_add(width_bytes(width))
        .ok_or_else(|| DeviceError::new("AC register access overflow"))?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| DeviceError::new("AC register access exceeds register"))?;
    Ok(slice
        .iter()
        .enumerate()
        .fold(0_u64, |value, (shift, byte)| {
            value | (u64::from(*byte) << (shift * 8))
        }))
}

fn write_le(
    bytes: &mut [u8],
    offset: usize,
    width: AccessWidth,
    value: u64,
) -> Result<(), DeviceError> {
    let end = offset
        .checked_add(width_bytes(width))
        .ok_or_else(|| DeviceError::new("AC register access overflow"))?;
    let slice = bytes
        .get_mut(offset..end)
        .ok_or_else(|| DeviceError::new("AC register access exceeds register"))?;
    for (shift, byte) in slice.iter_mut().enumerate() {
        *byte = (value >> (shift * 8)) as u8;
    }
    Ok(())
}

/// Native ATSAMD21 analog-comparator register identifiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Samd21AcRegister {
    /// Control A, offset `0x00`.
    Ctrla,
    /// Single-shot start bits, offset `0x01`.
    Ctrlb,
    /// Event input/output selection, offset `0x02`.
    Evctrl,
    /// Interrupt enable clear alias, offset `0x04`.
    Intenclr,
    /// Interrupt enable set alias, offset `0x05`.
    Intenset,
    /// Interrupt flags, offset `0x06`.
    Intflag,
    /// Comparator state, offset `0x08`.
    Statusa,
    /// Comparator ready state, offset `0x09`.
    Statusb,
    /// Comparator state with single-shot read trigger, offset `0x0a`.
    Statusc,
    /// Window mode, offset `0x0c`.
    Winctrl,
    /// Comparator 0 control, offset `0x10`.
    Compctrl0,
    /// Comparator 1 control, offset `0x14`.
    Compctrl1,
    /// Comparator 0 VDD scaler, offset `0x20`.
    Scaler0,
    /// Comparator 1 VDD scaler, offset `0x21`.
    Scaler1,
}

impl Samd21AcRegister {
    /// Returns the native byte offset of this register.
    pub const fn offset(self) -> usize {
        match self {
            Self::Ctrla => 0x00,
            Self::Ctrlb => 0x01,
            Self::Evctrl => 0x02,
            Self::Intenclr => 0x04,
            Self::Intenset => 0x05,
            Self::Intflag => 0x06,
            Self::Statusa => 0x08,
            Self::Statusb => 0x09,
            Self::Statusc => 0x0a,
            Self::Winctrl => 0x0c,
            Self::Compctrl0 => 0x10,
            Self::Compctrl1 => 0x14,
            Self::Scaler0 => 0x20,
            Self::Scaler1 => 0x21,
        }
    }

    const fn size(self) -> usize {
        match self {
            Self::Evctrl => 2,
            Self::Compctrl0 | Self::Compctrl1 => 4,
            _ => 1,
        }
    }

    fn locate(offset: u64, width: AccessWidth) -> Result<(Self, usize), DeviceError> {
        let offset =
            usize::try_from(offset).map_err(|_| DeviceError::new("AC register offset overflow"))?;
        let requested = width_bytes(width);
        let registers = [
            Self::Ctrla,
            Self::Ctrlb,
            Self::Evctrl,
            Self::Intenclr,
            Self::Intenset,
            Self::Intflag,
            Self::Statusa,
            Self::Statusb,
            Self::Statusc,
            Self::Winctrl,
            Self::Compctrl0,
            Self::Compctrl1,
            Self::Scaler0,
            Self::Scaler1,
        ];
        registers
            .into_iter()
            .find_map(|register| {
                let start = register.offset();
                let end = start.checked_add(register.size())?;
                let access_end = offset.checked_add(requested)?;
                (offset >= start && access_end <= end).then_some((register, offset - start))
            })
            .ok_or_else(|| DeviceError::new(format!("unmodeled AC access at {offset:#x}")))
    }
}

const FLAG_COMP0: u8 = 1 << 0;
const FLAG_COMP1: u8 = 1 << 1;
const FLAG_WINDOW: u8 = 1 << 4;
const COMPCTRL_MASK: u32 = 0x070b_b76f;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AcRegisters {
    ctrla: u8,
    evctrl: u16,
    interrupt_enable: u8,
    interrupt_flags: u8,
    states: u8,
    ready: u8,
    winctrl: u8,
    compctrl: [u32; 2],
    scaler: [u8; 2],
    window_state: u8,
}

impl Default for AcRegisters {
    fn default() -> Self {
        Self {
            ctrla: 0,
            evctrl: 0,
            interrupt_enable: 0,
            interrupt_flags: 0,
            states: 0,
            ready: 0,
            winctrl: 0,
            compctrl: [0; 2],
            scaler: [0; 2],
            window_state: 0,
        }
    }
}

#[derive(Default)]
struct AcState {
    registers: AcRegisters,
    inputs: [u16; 8],
}

/// Host-facing deterministic input and interrupt handle for the AC pair.
#[derive(Clone)]
pub struct Samd21AcHandle {
    state: Arc<Mutex<AcState>>,
    signals: SignalHub,
    output_signals: [SignalId; 2],
}

impl Samd21AcHandle {
    /// Supplies a digitalized analog code for one of the eight AIN inputs.
    pub fn inject_input(&self, input: u8, value: u16) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("AC lock poisoned");
        let Some(sample) = state.inputs.get_mut(usize::from(input)) else {
            return Err(DeviceError::new(format!(
                "AC input {input} is out of range"
            )));
        };
        *sample = value;
        Ok(())
    }

    /// Evaluates continuous comparators and returns the enabled IRQ level.
    pub fn poll(&self, at: SimTime) -> Result<bool, DeviceError> {
        let mut state = self.state.lock().expect("AC lock poisoned");
        for comparator in 0..2 {
            let control = state.registers.compctrl[comparator];
            if state.registers.ctrla & 2 != 0 && control & 1 != 0 && control & 2 == 0 {
                evaluate_comparator(&mut state, comparator);
            }
        }
        refresh_outputs(&state, &self.signals, &self.output_signals, at)?;
        Ok(state.registers.interrupt_flags & state.registers.interrupt_enable != 0)
    }

    /// Returns the current comparator output bit.
    pub fn output(&self, comparator: u8) -> Result<bool, DeviceError> {
        if comparator >= 2 {
            return Err(DeviceError::new("AC comparator is out of range"));
        }
        Ok(self
            .state
            .lock()
            .expect("AC lock poisoned")
            .registers
            .states
            & (1 << comparator)
            != 0)
    }

    /// Returns the current interrupt flags, including disabled sources.
    pub fn interrupt_flags(&self) -> u8 {
        self.state
            .lock()
            .expect("AC lock poisoned")
            .registers
            .interrupt_flags
    }
}

/// Functional ATSAMD21 analog-comparator pair.
pub struct Samd21Ac {
    name: String,
    state: Arc<Mutex<AcState>>,
    signals: SignalHub,
    output_signals: [SignalId; 2],
}

impl Samd21Ac {
    /// Constructs AC0/AC1 and its VCD-visible comparator outputs.
    pub fn new(
        name: impl Into<String>,
        path: &str,
        signals: SignalHub,
    ) -> Result<(Self, Samd21AcHandle), remu_signals::SignalError> {
        let output_signals = [
            signals.declare(
                format!("{path}.comp0"),
                SignalValue::from_u64(0, 1)?,
                Some("ATSAMD21 comparator 0 output".to_owned()),
            )?,
            signals.declare(
                format!("{path}.comp1"),
                SignalValue::from_u64(0, 1)?,
                Some("ATSAMD21 comparator 1 output".to_owned()),
            )?,
        ];
        let state = Arc::new(Mutex::new(AcState::default()));
        let handle = Samd21AcHandle {
            state: state.clone(),
            signals: signals.clone(),
            output_signals,
        };
        Ok((
            Self {
                name: name.into(),
                state,
                signals,
                output_signals,
            },
            handle,
        ))
    }
}

fn comparator_input(state: &AcState, comparator: usize, positive: bool) -> u16 {
    let control = state.registers.compctrl[comparator];
    if positive {
        let mux = ((control >> 12) & 0x03) as usize + comparator * 4;
        state.inputs[mux.min(state.inputs.len() - 1)]
    } else {
        match (control >> 8) & 0x07 {
            0..=3 => {
                let mux = ((control >> 8) & 0x03) as usize + comparator * 4;
                state.inputs[mux.min(state.inputs.len() - 1)]
            }
            4 => 0,
            5 => u16::from(state.registers.scaler[comparator]) * 0x40,
            6 => 0x0800,
            7 => 0,
            _ => 0,
        }
    }
}

fn evaluate_comparator(state: &mut AcState, comparator: usize) {
    let control = state.registers.compctrl[comparator];
    let positive = comparator_input(state, comparator, true);
    let negative = comparator_input(state, comparator, false);
    let swap = control & (1 << 15) != 0;
    let output = if swap {
        positive <= negative
    } else {
        positive > negative
    };
    let bit = 1 << comparator;
    let previous = state.registers.states & bit != 0;
    if output {
        state.registers.states |= bit;
    } else {
        state.registers.states &= !bit;
    }
    state.registers.ready |= bit;
    let interrupt_selection = (control >> 5) & 0x03;
    let matched = match interrupt_selection {
        0 => previous != output,
        1 => !previous && output,
        2 => previous && !output,
        3 => control & 2 != 0,
        _ => false,
    };
    if matched {
        state.registers.interrupt_flags |= if comparator == 0 {
            FLAG_COMP0
        } else {
            FLAG_COMP1
        };
    }
    if state.registers.winctrl & 1 != 0 {
        let comp0 = state.registers.states & 1 != 0;
        let comp1 = state.registers.states & 2 != 0;
        state.registers.window_state = match (comp0, comp1) {
            (true, true) => 0,
            (true, false) => 1,
            (false, false) => 2,
            (false, true) => 3,
        };
        let window_match = match (state.registers.winctrl >> 1) & 0x03 {
            0 => state.registers.window_state == 0,
            1 => state.registers.window_state == 1,
            2 => state.registers.window_state == 2,
            3 => matches!(state.registers.window_state, 0 | 2),
            _ => false,
        };
        if window_match {
            state.registers.interrupt_flags |= FLAG_WINDOW;
        }
    }
}

fn refresh_outputs(
    state: &AcState,
    signals: &SignalHub,
    output_signals: &[SignalId; 2],
    at: SimTime,
) -> Result<(), DeviceError> {
    for (comparator, signal) in output_signals.iter().enumerate() {
        signals
            .set(
                *signal,
                SignalValue::from_u64(u64::from((state.registers.states >> comparator) & 1), 1)
                    .map_err(|error| DeviceError::new(error.to_string()))?,
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))?;
    }
    Ok(())
}

fn read_register(state: &mut AcState, register: Samd21AcRegister) -> u32 {
    match register {
        Samd21AcRegister::Ctrla => u32::from(state.registers.ctrla & 0x06),
        Samd21AcRegister::Ctrlb => 0,
        Samd21AcRegister::Evctrl => u32::from(state.registers.evctrl & 0x0313),
        Samd21AcRegister::Intenclr | Samd21AcRegister::Intenset => {
            u32::from(state.registers.interrupt_enable & 0x13)
        }
        Samd21AcRegister::Intflag => u32::from(state.registers.interrupt_flags & 0x13),
        Samd21AcRegister::Statusa | Samd21AcRegister::Statusc => {
            u32::from(state.registers.states & 0x03)
                | (u32::from(state.registers.window_state & 0x03) << 4)
        }
        Samd21AcRegister::Statusb => u32::from(state.registers.ready & 0x03),
        Samd21AcRegister::Winctrl => u32::from(state.registers.winctrl & 0x07),
        Samd21AcRegister::Compctrl0 => state.registers.compctrl[0] & COMPCTRL_MASK,
        Samd21AcRegister::Compctrl1 => state.registers.compctrl[1] & COMPCTRL_MASK,
        Samd21AcRegister::Scaler0 => u32::from(state.registers.scaler[0] & 0x3f),
        Samd21AcRegister::Scaler1 => u32::from(state.registers.scaler[1] & 0x3f),
    }
}

fn write_register(
    state: &mut AcState,
    register: Samd21AcRegister,
    byte_offset: usize,
    width: AccessWidth,
    value: u64,
) -> Result<[usize; 2], DeviceError> {
    let mut raw = read_register(state, register);
    let mut bytes = [0_u8; 4];
    bytes[..register.size()].copy_from_slice(&raw.to_le_bytes()[..register.size()]);
    write_le(&mut bytes, byte_offset, width, value)?;
    raw = u32::from_le_bytes(bytes);
    let mut evaluate = [usize::MAX; 2];
    match register {
        Samd21AcRegister::Ctrla => {
            let write = raw as u8;
            if write & 1 != 0 {
                *state = AcState::default();
            } else {
                state.registers.ctrla = write & 0x06;
            }
        }
        Samd21AcRegister::Ctrlb => {
            for comparator in 0..2 {
                if raw as u8 & (1 << comparator) != 0 {
                    state.registers.ready &= !(1 << comparator);
                    evaluate[comparator] = comparator;
                }
            }
        }
        Samd21AcRegister::Evctrl => state.registers.evctrl = raw as u16 & 0x0313,
        Samd21AcRegister::Intenclr => state.registers.interrupt_enable &= !(raw as u8 & 0x13),
        Samd21AcRegister::Intenset => state.registers.interrupt_enable |= raw as u8 & 0x13,
        Samd21AcRegister::Intflag => state.registers.interrupt_flags &= !(raw as u8 & 0x13),
        Samd21AcRegister::Statusa | Samd21AcRegister::Statusb | Samd21AcRegister::Statusc => {}
        Samd21AcRegister::Winctrl => state.registers.winctrl = raw as u8 & 0x07,
        Samd21AcRegister::Compctrl0 | Samd21AcRegister::Compctrl1 => {
            let comparator = usize::from(register == Samd21AcRegister::Compctrl1);
            state.registers.compctrl[comparator] = raw & COMPCTRL_MASK;
        }
        Samd21AcRegister::Scaler0 | Samd21AcRegister::Scaler1 => {
            let comparator = usize::from(register == Samd21AcRegister::Scaler1);
            state.registers.scaler[comparator] = raw as u8 & 0x3f;
        }
    }
    for comparator in evaluate {
        if comparator != usize::MAX
            && state.registers.ctrla & 2 != 0
            && state.registers.compctrl[comparator] & 1 != 0
        {
            evaluate_comparator(state, comparator);
        }
    }
    Ok(evaluate)
}

impl Device for Samd21Ac {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        let (register, byte_offset) = Samd21AcRegister::locate(offset, width)?;
        let mut state = self.state.lock().expect("AC lock poisoned");
        if register == Samd21AcRegister::Statusc {
            for comparator in 0..2 {
                let control = state.registers.compctrl[comparator];
                if state.registers.ctrla & 2 != 0 && control & 3 == 3 {
                    state.registers.ready &= !(1 << comparator);
                    evaluate_comparator(&mut state, comparator);
                }
            }
        }
        let raw = read_register(&mut state, register);
        let value = read_le(&raw.to_le_bytes(), byte_offset, width)?;
        refresh_outputs(&state, &self.signals, &self.output_signals, at)?;
        Ok(value)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let (register, byte_offset) = Samd21AcRegister::locate(offset, width)?;
        let mut state = self.state.lock().expect("AC lock poisoned");
        write_register(&mut state, register, byte_offset, width, value)?;
        refresh_outputs(&state, &self.signals, &self.output_signals, at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("AC lock poisoned") = AcState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_registers_match_samd21_ac_offsets() {
        assert_eq!(Samd21AcRegister::Ctrla.offset(), 0x00);
        assert_eq!(Samd21AcRegister::Compctrl0.offset(), 0x10);
        assert_eq!(Samd21AcRegister::Compctrl1.offset(), 0x14);
        assert_eq!(Samd21AcRegister::Scaler1.offset(), 0x21);
    }

    #[test]
    fn single_shot_comparison_sets_state_ready_and_rising_flag() {
        let signals = SignalHub::new();
        let (mut ac, handle) = Samd21Ac::new("ac", "board.ac", signals).unwrap();
        handle.inject_input(0, 0x0900).unwrap();
        ac.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO).unwrap();
        // COMP0: positive AIN0, negative GND, rising-edge interrupt, single-shot.
        ac.write(
            0x10,
            AccessWidth::Word,
            (1 << 5) | (1 << 1) | (4 << 8) | 1,
            SimTime::ZERO,
        )
        .unwrap();
        ac.write(0x05, AccessWidth::Byte, FLAG_COMP0 as u64, SimTime::ZERO)
            .unwrap();
        ac.write(0x01, AccessWidth::Byte, 1, SimTime::ZERO).unwrap();
        assert!(handle.output(0).unwrap());
        assert_eq!(handle.interrupt_flags(), FLAG_COMP0);
        assert_eq!(ac.read(0x08, AccessWidth::Byte, SimTime::ZERO).unwrap(), 1);
        assert_eq!(ac.read(0x09, AccessWidth::Byte, SimTime::ZERO).unwrap(), 1);
        ac.write(0x06, AccessWidth::Byte, FLAG_COMP0 as u64, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.interrupt_flags(), 0);
    }

    #[test]
    fn continuous_inputs_update_outputs_and_window_state() {
        let signals = SignalHub::new();
        let (mut ac, handle) = Samd21Ac::new("ac", "board.ac", signals).unwrap();
        handle.inject_input(0, 0x0900).unwrap();
        handle.inject_input(1, 0x0100).unwrap();
        ac.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO).unwrap();
        // Comparator 0 sees AIN0 > AIN1 in continuous mode.
        ac.write(
            0x10,
            AccessWidth::Word,
            1 | (1 << 8) | (1 << 5),
            SimTime::ZERO,
        )
        .unwrap();
        ac.write(0x05, AccessWidth::Byte, FLAG_COMP0 as u64, SimTime::ZERO)
            .unwrap();
        assert!(handle.poll(SimTime::from_ticks(1)).unwrap());
        assert!(handle.output(0).unwrap());
        ac.write(0x06, AccessWidth::Byte, FLAG_COMP0 as u64, SimTime::ZERO)
            .unwrap();
        handle.inject_input(0, 0x0000).unwrap();
        assert!(!handle.poll(SimTime::from_ticks(2)).unwrap());
        assert!(!handle.output(0).unwrap());
    }
}
