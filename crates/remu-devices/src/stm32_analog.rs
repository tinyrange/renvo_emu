use super::*;

const COMP_EN: u32 = 1 << 0;
const COMP_CONFIG_MASK: u32 = 0x07df_81ff;
const COMP_POLARITY: u32 = 1 << 15;
const COMP_VALUE: u32 = 1 << 30;
const COMP_LOCK: u32 = 1 << 31;

const OPAMP_ENABLE: u32 = 1 << 0;
const OPAMP_MODE_MASK: u32 = 0x0c;
const OPAMP_GAIN_MASK: u32 = 0x30;
const OPAMP_CONFIG_MASK: u32 = 0x0000_7f3f;

/// Named register identifiers for the STM32 comparator block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stm32ComparatorRegister {
    /// One comparator control/status register.
    Csr(usize),
}

/// Named register identifiers for the STM32 OPAMP block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stm32OpampRegister {
    /// Control/status register.
    Csr,
    /// Normal-mode offset trim register.
    Otr,
    /// Low-power-mode offset trim register.
    Lpotr,
}

struct ComparatorState {
    csr: [u32; 2],
    plus: [u16; 2],
    minus: [u16; 2],
    pending: [bool; 2],
}

/// Host-facing comparator input and output handle.
#[derive(Clone)]
pub struct Stm32ComparatorHandle {
    state: Arc<Mutex<ComparatorState>>,
}

impl Stm32ComparatorHandle {
    /// Supplies the two deterministic comparator input levels.
    pub fn set_inputs(&self, comparator: usize, plus: u16, minus: u16) -> bool {
        let mut state = self.state.lock().expect("STM32 comparator lock poisoned");
        if comparator >= state.csr.len() {
            return false;
        }
        state.plus[comparator] = plus;
        state.minus[comparator] = minus;
        Self::evaluate(&mut state, comparator);
        true
    }

    /// Returns the resolved digital output level of one comparator.
    pub fn output(&self, comparator: usize) -> Option<bool> {
        let state = self.state.lock().expect("STM32 comparator lock poisoned");
        state.csr.get(comparator).map(|csr| csr & COMP_VALUE != 0)
    }

    /// Returns true when either comparator has changed output since clearing.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("STM32 comparator lock poisoned");
        state.pending.iter().copied().any(|pending| pending)
    }

    /// Clears comparator output-change notification state.
    pub fn clear_interrupt(&self) {
        let mut state = self.state.lock().expect("STM32 comparator lock poisoned");
        state.pending = [false; 2];
    }

    fn evaluate(state: &mut ComparatorState, comparator: usize) {
        let old = state.csr[comparator] & COMP_VALUE != 0;
        let mut output = state.csr[comparator] & COMP_EN != 0
            && state.plus[comparator] > state.minus[comparator];
        if state.csr[comparator] & COMP_POLARITY != 0 {
            output = !output;
        }
        if output {
            state.csr[comparator] |= COMP_VALUE;
        } else {
            state.csr[comparator] &= !COMP_VALUE;
        }
        if old != output {
            state.pending[comparator] = true;
        }
    }
}

/// Functional STM32L432KC COMP1/COMP2 register block.
pub struct Stm32Comparators {
    name: String,
    state: Arc<Mutex<ComparatorState>>,
}

impl Stm32Comparators {
    /// Creates reset comparators and a host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, Stm32ComparatorHandle) {
        let state = Arc::new(Mutex::new(ComparatorState {
            csr: [0; 2],
            plus: [0; 2],
            minus: [0; 2],
            pending: [false; 2],
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32ComparatorHandle { state },
        )
    }

    fn decode(offset: u64) -> Option<Stm32ComparatorRegister> {
        match offset {
            0x00 => Some(Stm32ComparatorRegister::Csr(0)),
            0x04 => Some(Stm32ComparatorRegister::Csr(1)),
            _ => None,
        }
    }

    fn require_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "STM32 comparator requires aligned word accesses",
            ));
        }
        Ok(())
    }
}

impl Device for Stm32Comparators {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_access(offset, width)?;
        let register = Self::decode(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} read outside registers at {offset:#x}",
                self.name
            ))
        })?;
        let state = self.state.lock().expect("STM32 comparator lock poisoned");
        let Stm32ComparatorRegister::Csr(comparator) = register;
        Ok(u64::from(state.csr[comparator]))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        Self::require_access(offset, width)?;
        let value = u32::try_from(value).expect("word access value fits u32");
        let register = Self::decode(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write outside registers at {offset:#x}",
                self.name
            ))
        })?;
        let mut state = self.state.lock().expect("STM32 comparator lock poisoned");
        let Stm32ComparatorRegister::Csr(comparator) = register;
        if state.csr[comparator] & COMP_LOCK != 0 {
            return Ok(());
        }
        state.csr[comparator] = (value & COMP_CONFIG_MASK) | (value & COMP_LOCK);
        Stm32ComparatorHandle::evaluate(&mut state, comparator);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("STM32 comparator lock poisoned");
        state.csr = [0; 2];
        state.plus = [0; 2];
        state.minus = [0; 2];
        state.pending = [false; 2];
    }
}

struct OpampState {
    csr: u32,
    otr: u32,
    lpotr: u32,
    plus: u16,
    minus: u16,
    output: u16,
}

/// Host-facing OPAMP input and output handle.
#[derive(Clone)]
pub struct Stm32OpampHandle {
    state: Arc<Mutex<OpampState>>,
}

impl Stm32OpampHandle {
    /// Supplies deterministic OPAMP input levels and recomputes its output.
    pub fn set_inputs(&self, plus: u16, minus: u16) {
        let mut state = self.state.lock().expect("STM32 OPAMP lock poisoned");
        state.plus = plus;
        state.minus = minus;
        Self::evaluate(&mut state);
    }

    /// Returns the functional OPAMP output level.
    pub fn output(&self) -> u16 {
        self.state.lock().expect("STM32 OPAMP lock poisoned").output
    }

    fn evaluate(state: &mut OpampState) {
        if state.csr & OPAMP_ENABLE == 0 {
            state.output = 0;
            return;
        }
        let mode = (state.csr & OPAMP_MODE_MASK) >> 2;
        let gain = match (state.csr & OPAMP_GAIN_MASK) >> 4 {
            0 => 1_u32,
            1 => 2,
            2 => 4,
            _ => 8,
        };
        let differential = i32::from(state.plus) - i32::from(state.minus);
        let value = if mode == 0 {
            i32::from(state.plus)
        } else {
            i32::from(state.minus) + differential.saturating_mul(gain as i32)
        };
        state.output = value.clamp(0, i32::from(u16::MAX)) as u16;
    }
}

/// Functional STM32L432KC OPAMP1 register block.
pub struct Stm32Opamp {
    name: String,
    state: Arc<Mutex<OpampState>>,
}

impl Stm32Opamp {
    /// Creates a reset OPAMP and host-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, Stm32OpampHandle) {
        let state = Arc::new(Mutex::new(OpampState {
            csr: 0,
            otr: 0,
            lpotr: 0,
            plus: 0,
            minus: 0,
            output: 0,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Stm32OpampHandle { state },
        )
    }

    fn decode(offset: u64) -> Option<Stm32OpampRegister> {
        match offset {
            0x00 => Some(Stm32OpampRegister::Csr),
            0x04 => Some(Stm32OpampRegister::Otr),
            0x08 => Some(Stm32OpampRegister::Lpotr),
            _ => None,
        }
    }

    fn require_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "STM32 OPAMP requires aligned word accesses",
            ));
        }
        Ok(())
    }
}

impl Device for Stm32Opamp {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_access(offset, width)?;
        let register = Self::decode(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} read outside registers at {offset:#x}",
                self.name
            ))
        })?;
        let state = self.state.lock().expect("STM32 OPAMP lock poisoned");
        let value = match register {
            Stm32OpampRegister::Csr => state.csr,
            Stm32OpampRegister::Otr => state.otr,
            Stm32OpampRegister::Lpotr => state.lpotr,
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        Self::require_access(offset, width)?;
        let value = u32::try_from(value).expect("word access value fits u32");
        let register = Self::decode(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write outside registers at {offset:#x}",
                self.name
            ))
        })?;
        let mut state = self.state.lock().expect("STM32 OPAMP lock poisoned");
        match register {
            Stm32OpampRegister::Csr => {
                state.csr = value & OPAMP_CONFIG_MASK;
                Stm32OpampHandle::evaluate(&mut state);
            }
            Stm32OpampRegister::Otr => state.otr = value & 0x1f_1f,
            Stm32OpampRegister::Lpotr => state.lpotr = value & 0x1f_1f,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("STM32 OPAMP lock poisoned");
        state.csr = 0;
        state.otr = 0;
        state.lpotr = 0;
        state.plus = 0;
        state.minus = 0;
        state.output = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_bus::{AddressSpace, Endianness};
    use remu_core::{AccessKind, Bus};

    #[test]
    fn comparator_output_tracks_host_inputs_and_polarity() {
        let mut bus = AddressSpace::new(Endianness::Little);
        let (comparators, handle) = Stm32Comparators::new("comp");
        bus.map_device("comp", 0x4001_0200, 0x100, Box::new(comparators))
            .unwrap();
        bus.write(
            0x4001_0200,
            AccessWidth::Word,
            u64::from(COMP_EN),
            SimTime::ZERO,
        )
        .unwrap();
        handle.set_inputs(0, 700, 400);
        assert_eq!(handle.output(0), Some(true));
        assert!(handle.interrupt_pending());
        let csr = bus
            .read(
                0x4001_0200,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap();
        assert_ne!(csr & u64::from(COMP_VALUE), 0);
        handle.clear_interrupt();
        bus.write(
            0x4001_0200,
            AccessWidth::Word,
            u64::from(COMP_EN | COMP_POLARITY),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.output(0), Some(false));
    }

    #[test]
    fn opamp_pga_produces_clamped_functional_output() {
        let mut bus = AddressSpace::new(Endianness::Little);
        let (opamp, handle) = Stm32Opamp::new("opamp");
        bus.map_device("opamp", 0x4000_7800, 0x100, Box::new(opamp))
            .unwrap();
        handle.set_inputs(1200, 200);
        bus.write(
            0x4000_7800,
            AccessWidth::Word,
            u64::from(OPAMP_ENABLE | (3 << 2) | (3 << 4)),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.output(), 8200);
        assert_eq!(
            bus.read(
                0x4000_7804,
                AccessWidth::Word,
                AccessKind::Read,
                SimTime::ZERO,
            )
            .unwrap(),
            0
        );
    }
}
