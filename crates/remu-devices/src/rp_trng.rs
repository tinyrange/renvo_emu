use super::*;

const EHR_VALID: u32 = 1;
const STATUS_MASK: u32 = 0x0f;

fn atomic_update(current: u32, alias: u64, value: u32) -> Result<u32, DeviceError> {
    match alias {
        0 => Ok(value),
        1 => Ok(current ^ value),
        2 => Ok(current | value),
        3 => Ok(current & !value),
        _ => Err(DeviceError::new("invalid RP2350 TRNG atomic alias")),
    }
}

fn word_access(width: AccessWidth, offset: u64) -> Result<(), DeviceError> {
    if width != AccessWidth::Word || !width.is_aligned(offset) {
        Err(DeviceError::new("RP2350 TRNG requires aligned word access"))
    } else {
        Ok(())
    }
}

struct Rp2350TrngState {
    interrupt_mask: u32,
    interrupt_status: u32,
    config: u32,
    valid: bool,
    entropy: [u32; 6],
    source_enable: u32,
    sample_count: u32,
    autocorrelation: u32,
    debug_control: u32,
    debug_enable: u32,
    busy: bool,
    version: u32,
    bist: [u32; 3],
    seed: u64,
}

impl Rp2350TrngState {
    fn reset() -> Self {
        Self {
            interrupt_mask: 0x0f,
            interrupt_status: 0,
            config: 0,
            valid: false,
            entropy: [0; 6],
            source_enable: 0,
            sample_count: 0x0000_ffff,
            autocorrelation: 0,
            debug_control: 0,
            debug_enable: 0,
            busy: false,
            version: 0,
            bist: [0; 3],
            seed: 0x9e37_79b9_7f4a_7c15,
        }
    }

    fn clear_result(&mut self) {
        self.valid = false;
        self.interrupt_status &= !EHR_VALID;
        self.entropy = [0; 6];
    }

    fn generate(&mut self) {
        // This is a deterministic entropy-shaped source for firmware tests. It intentionally
        // does not claim the statistical or security properties of the RP2350's analogue TRNG.
        let mut state = self.seed;
        for word in &mut self.entropy {
            state =
                state.wrapping_add(0x9e37_79b9_7f4a_7c15).rotate_left(17) ^ 0xa076_1d64_78bd_642f;
            let mixed = state ^ (state >> 29);
            *word = (mixed ^ (mixed >> 32)) as u32;
        }
        self.seed = state;
        self.valid = true;
        self.interrupt_status |= EHR_VALID;
    }
}

/// Host-facing view of the RP2350 true-random-number-generator status.
#[derive(Clone)]
pub struct Rp2350TrngHandle {
    state: Rc<RefCell<Rp2350TrngState>>,
}

impl Rp2350TrngHandle {
    /// Returns true when an unmasked TRNG interrupt is pending.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.borrow();
        state.interrupt_status & !state.interrupt_mask & STATUS_MASK != 0
    }

    /// Returns true when six deterministic entropy words are available to firmware.
    pub fn result_ready(&self) -> bool {
        self.state.borrow().valid
    }
}

/// Functional RP2350 TRNG register block.
///
/// Enabling the documented random source completes one deterministic 192-bit generation
/// immediately. Firmware sees the same valid/data/interrupt protocol as the hardware, while the
/// implementation remains reproducible for CI and replay.
pub struct Rp2350Trng {
    name: String,
    state: Rc<RefCell<Rp2350TrngState>>,
}

impl Rp2350Trng {
    /// Creates a TRNG and its interrupt/status handle.
    pub fn new(name: impl Into<String>) -> (Self, Rp2350TrngHandle) {
        let state = Rc::new(RefCell::new(Rp2350TrngState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Rp2350TrngHandle { state },
        )
    }

    fn read_entropy(state: &mut Rp2350TrngState, index: usize) -> u32 {
        let value = state.valid.then_some(state.entropy[index]).unwrap_or(0);
        if index == 5 && state.valid {
            state.clear_result();
        }
        value
    }
}

impl Device for Rp2350Trng {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        word_access(width, offset)?;
        let register = offset & 0x0fff;
        let mut state = self.state.borrow_mut();
        let value = match register {
            0x100 => state.interrupt_mask,
            0x104 => state.interrupt_status & STATUS_MASK,
            0x10c => state.config,
            0x110 => u32::from(state.valid),
            0x114..=0x128 if (register - 0x114) % 4 == 0 => {
                let index = usize::try_from((register - 0x114) / 4).expect("TRNG data index fits");
                Rp2350Trng::read_entropy(&mut state, index)
            }
            0x12c => state.source_enable,
            0x130 => state.sample_count,
            0x134 => state.autocorrelation,
            0x138 => state.debug_control,
            0x1b4 => state.debug_enable,
            0x1b8 => u32::from(state.busy),
            0x1c0 => state.version,
            0x1e0..=0x1e8 if (register - 0x1e0) % 4 == 0 => {
                let index = usize::try_from((register - 0x1e0) / 4).expect("TRNG BIST index fits");
                state.bist[index]
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 TRNG read at offset {register:#x}"
                )));
            }
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
        word_access(width, offset)?;
        let alias = (offset >> 12) & 3;
        let register = offset & 0x0fff;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked TRNG value fits");
        let mut state = self.state.borrow_mut();
        match register {
            0x100 => {
                state.interrupt_mask = atomic_update(state.interrupt_mask, alias, value)? & 0xf
            }
            0x108 => {
                state.interrupt_status &= !(value & STATUS_MASK);
                if value & EHR_VALID != 0 {
                    state.clear_result();
                }
            }
            0x10c => state.config = atomic_update(state.config, alias, value)? & 3,
            0x12c => {
                let before = state.source_enable;
                state.source_enable = atomic_update(state.source_enable, alias, value)? & 1;
                if before == 0 && state.source_enable != 0 {
                    state.generate();
                }
            }
            0x130 => state.sample_count = atomic_update(state.sample_count, alias, value)?,
            0x134 => state.autocorrelation = 0,
            0x138 => state.debug_control = atomic_update(state.debug_control, alias, value)? & 0xe,
            0x140 => {
                if value & 1 != 0 {
                    *state = Rp2350TrngState::reset();
                }
            }
            0x1b4 => state.debug_enable = atomic_update(state.debug_enable, alias, value)? & 1,
            0x1bc => {
                if state.source_enable == 0 {
                    state.clear_result();
                }
            }
            0x1e0..=0x1e8 if (register - 0x1e0) % 4 == 0 => {
                let index = usize::try_from((register - 0x1e0) / 4).expect("TRNG BIST index fits");
                state.bist[index] = value & 0x003f_ffff;
            }
            0x104 | 0x110 | 0x114..=0x128 | 0x1b8 | 0x1c0 => {
                return Err(DeviceError::new(format!(
                    "RP2350 TRNG register at offset {register:#x} is read-only"
                )));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 TRNG write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = Rp2350TrngState::reset();
    }
}
