use super::*;

const EHR_VALID: u32 = 1;
const STATUS_MASK: u32 = 0x0f;
const ICR_CLEARABLE: u32 = 0x0d;
const AUTOCORR_STATISTIC_MASK: u32 = 0x003f_ffff;
const BIST_COUNTER_MASK: u32 = 0x003f_ffff;
const VERSION_MASK: u32 = 0x0000_00ff;

/// Register offsets in the RP2350 TRNG block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Rp2350TrngRegister {
    /// Interrupt mask.
    InterruptMask = 0x100,
    /// Read-only interrupt/status register.
    InterruptStatus = 0x104,
    /// Write-one-to-clear interrupt/status register.
    InterruptClear = 0x108,
    /// Ring-oscillator source selection.
    Config = 0x10c,
    /// Read-only 192-bit collection indication.
    Valid = 0x110,
    /// Entropy holding register word 0.
    EhrData0 = 0x114,
    /// Entropy holding register word 1.
    EhrData1 = 0x118,
    /// Entropy holding register word 2.
    EhrData2 = 0x11c,
    /// Entropy holding register word 3.
    EhrData3 = 0x120,
    /// Entropy holding register word 4.
    EhrData4 = 0x124,
    /// Entropy holding register word 5; reading it consumes the result.
    EhrData5 = 0x128,
    /// Entropy-source enable.
    SourceEnable = 0x12c,
    /// Sampling period.
    SampleCount = 0x130,
    /// Autocorrelation statistics.
    Autocorrelation = 0x134,
    /// Test bypass controls.
    DebugControl = 0x138,
    /// Software reset.
    SoftwareReset = 0x140,
    /// Debug-mode enable.
    DebugEnable = 0x1b4,
    /// Read-only busy status.
    Busy = 0x1b8,
    /// Reset the collected-bits counter.
    ResetBitsCounter = 0x1bc,
    /// Read-only TRNG feature version.
    Version = 0x1c0,
    /// Read-only BIST counter 0.
    BistCounter0 = 0x1e0,
    /// Read-only BIST counter 1.
    BistCounter1 = 0x1e4,
    /// Read-only BIST counter 2.
    BistCounter2 = 0x1e8,
}

impl TryFrom<u64> for Rp2350TrngRegister {
    type Error = DeviceError;

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x100 => Ok(Self::InterruptMask),
            0x104 => Ok(Self::InterruptStatus),
            0x108 => Ok(Self::InterruptClear),
            0x10c => Ok(Self::Config),
            0x110 => Ok(Self::Valid),
            0x114 => Ok(Self::EhrData0),
            0x118 => Ok(Self::EhrData1),
            0x11c => Ok(Self::EhrData2),
            0x120 => Ok(Self::EhrData3),
            0x124 => Ok(Self::EhrData4),
            0x128 => Ok(Self::EhrData5),
            0x12c => Ok(Self::SourceEnable),
            0x130 => Ok(Self::SampleCount),
            0x134 => Ok(Self::Autocorrelation),
            0x138 => Ok(Self::DebugControl),
            0x140 => Ok(Self::SoftwareReset),
            0x1b4 => Ok(Self::DebugEnable),
            0x1b8 => Ok(Self::Busy),
            0x1bc => Ok(Self::ResetBitsCounter),
            0x1c0 => Ok(Self::Version),
            0x1e0 => Ok(Self::BistCounter0),
            0x1e4 => Ok(Self::BistCounter1),
            0x1e8 => Ok(Self::BistCounter2),
            _ => Err(DeviceError::new(format!(
                "unmodeled RP2350 TRNG register at offset {offset:#x}"
            ))),
        }
    }
}

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

    fn clear_valid_status(&mut self) {
        self.valid = false;
        self.interrupt_status &= !EHR_VALID;
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
        let register = Rp2350TrngRegister::try_from(offset & 0x0fff)?;
        let mut state = self.state.borrow_mut();
        let value = match register {
            Rp2350TrngRegister::InterruptMask => state.interrupt_mask,
            Rp2350TrngRegister::InterruptStatus => state.interrupt_status & STATUS_MASK,
            Rp2350TrngRegister::Config => state.config,
            Rp2350TrngRegister::Valid => u32::from(state.valid),
            Rp2350TrngRegister::EhrData0
            | Rp2350TrngRegister::EhrData1
            | Rp2350TrngRegister::EhrData2
            | Rp2350TrngRegister::EhrData3
            | Rp2350TrngRegister::EhrData4
            | Rp2350TrngRegister::EhrData5 => {
                let index = match register {
                    Rp2350TrngRegister::EhrData0 => 0,
                    Rp2350TrngRegister::EhrData1 => 1,
                    Rp2350TrngRegister::EhrData2 => 2,
                    Rp2350TrngRegister::EhrData3 => 3,
                    Rp2350TrngRegister::EhrData4 => 4,
                    Rp2350TrngRegister::EhrData5 => 5,
                    _ => unreachable!("matched EHR data register"),
                };
                Rp2350Trng::read_entropy(&mut state, index)
            }
            Rp2350TrngRegister::SourceEnable => state.source_enable,
            Rp2350TrngRegister::SampleCount => state.sample_count,
            Rp2350TrngRegister::Autocorrelation => state.autocorrelation & AUTOCORR_STATISTIC_MASK,
            Rp2350TrngRegister::DebugControl => state.debug_control,
            Rp2350TrngRegister::SoftwareReset => 0,
            Rp2350TrngRegister::DebugEnable => state.debug_enable,
            Rp2350TrngRegister::Busy => u32::from(state.busy),
            Rp2350TrngRegister::ResetBitsCounter => 0,
            Rp2350TrngRegister::Version => state.version & VERSION_MASK,
            Rp2350TrngRegister::BistCounter0 => state.bist[0] & BIST_COUNTER_MASK,
            Rp2350TrngRegister::BistCounter1 => state.bist[1] & BIST_COUNTER_MASK,
            Rp2350TrngRegister::BistCounter2 => state.bist[2] & BIST_COUNTER_MASK,
            Rp2350TrngRegister::InterruptClear => 0,
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
        let register = Rp2350TrngRegister::try_from(offset & 0x0fff)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked TRNG value fits");
        let mut state = self.state.borrow_mut();
        match register {
            Rp2350TrngRegister::InterruptMask => {
                state.interrupt_mask = atomic_update(state.interrupt_mask, alias, value)? & 0xf
            }
            Rp2350TrngRegister::InterruptClear => {
                state.interrupt_status &= !(value & ICR_CLEARABLE);
                if value & EHR_VALID != 0 {
                    state.clear_valid_status();
                }
            }
            Rp2350TrngRegister::Config => {
                state.config = atomic_update(state.config, alias, value)? & 3
            }
            Rp2350TrngRegister::SourceEnable => {
                let before = state.source_enable;
                state.source_enable = atomic_update(state.source_enable, alias, value)? & 1;
                if before == 0 && state.source_enable != 0 {
                    state.generate();
                }
            }
            Rp2350TrngRegister::SampleCount => {
                state.sample_count = atomic_update(state.sample_count, alias, value)?
            }
            Rp2350TrngRegister::Autocorrelation => state.autocorrelation = 0,
            Rp2350TrngRegister::DebugControl => {
                state.debug_control = atomic_update(state.debug_control, alias, value)? & 0xe
            }
            Rp2350TrngRegister::SoftwareReset => {
                if value & 1 != 0 {
                    *state = Rp2350TrngState::reset();
                }
            }
            Rp2350TrngRegister::DebugEnable => {
                state.debug_enable = atomic_update(state.debug_enable, alias, value)? & 1
            }
            Rp2350TrngRegister::ResetBitsCounter => {
                if state.source_enable == 0 {
                    state.clear_result();
                }
            }
            Rp2350TrngRegister::InterruptStatus
            | Rp2350TrngRegister::Valid
            | Rp2350TrngRegister::EhrData0
            | Rp2350TrngRegister::EhrData1
            | Rp2350TrngRegister::EhrData2
            | Rp2350TrngRegister::EhrData3
            | Rp2350TrngRegister::EhrData4
            | Rp2350TrngRegister::EhrData5
            | Rp2350TrngRegister::Busy
            | Rp2350TrngRegister::Version
            | Rp2350TrngRegister::BistCounter0
            | Rp2350TrngRegister::BistCounter1
            | Rp2350TrngRegister::BistCounter2 => {
                return Err(DeviceError::new(format!(
                    "RP2350 TRNG register {register:?} is read-only"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = Rp2350TrngState::reset();
    }
}
