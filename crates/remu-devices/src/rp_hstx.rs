use super::*;

const CSR_MASK: u32 = 0xff1f_1f73;
const CSR_RESET: u32 = 0x1005_0600;
const CSR_EN: u32 = 1;
const BIT_MASK: u32 = 0x0003_1f1f;
const EXPAND_SHIFT_MASK: u32 = 0x1f1f_1f1f;
const EXPAND_SHIFT_RESET: u32 = 0x0100_0100;
const EXPAND_TMDS_MASK: u32 = 0x00ff_ffff;
const FIFO_WOF: u32 = 1 << 10;
const FIFO_EMPTY: u32 = 1 << 9;
const FIFO_FULL: u32 = 1 << 8;
const FIFO_CAPACITY: usize = 8;

fn atomic_update(current: u32, alias: u64, value: u32) -> Result<u32, DeviceError> {
    match alias {
        0 => Ok(value),
        1 => Ok(current ^ value),
        2 => Ok(current | value),
        3 => Ok(current & !value),
        _ => Err(DeviceError::new("invalid RP2350 HSTX atomic alias")),
    }
}

fn aligned_word(width: AccessWidth, offset: u64) -> Result<(), DeviceError> {
    if width != AccessWidth::Word || !width.is_aligned(offset) {
        Err(DeviceError::new(
            "RP2350 HSTX register requires aligned word access",
        ))
    } else {
        Ok(())
    }
}

/// One functional HSTX output sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HstxSample {
    /// Word consumed from the HSTX FIFO.
    pub word: u32,
    /// Values presented during the first half of the output cycle.
    pub positive: [bool; 8],
    /// Values presented during the second half of the output cycle.
    pub negative: [bool; 8],
    /// Generated clock phase for this sample.
    pub clock: bool,
}

/// Host-facing HSTX output capture.
#[derive(Clone, Default)]
pub struct HstxHandle {
    samples: Arc<Mutex<Vec<HstxSample>>>,
}

impl HstxHandle {
    /// Returns the deterministic samples emitted since reset or the last clear.
    pub fn samples(&self) -> Vec<HstxSample> {
        self.samples
            .lock()
            .expect("HSTX capture lock poisoned")
            .clone()
    }

    /// Clears captured HSTX samples.
    pub fn clear(&self) {
        self.samples
            .lock()
            .expect("HSTX capture lock poisoned")
            .clear();
    }
}

struct HstxState {
    csr: u32,
    bit: [u32; 8],
    expand_shift: u32,
    expand_tmds: u32,
    fifo: VecDeque<u32>,
    write_overflow: bool,
    phase: bool,
    hub: SignalHub,
    positive: Vec<SignalId>,
    negative: Vec<SignalId>,
    clock: SignalId,
    handle: HstxHandle,
}

impl HstxState {
    fn new(path: &str, hub: SignalHub) -> Result<(Self, HstxHandle), SignalError> {
        let mut positive = Vec::with_capacity(8);
        let mut negative = Vec::with_capacity(8);
        for lane in 0..8 {
            positive.push(hub.declare(
                format!("{path}.lane{lane}.p"),
                SignalValue::repeat(Logic::Zero, 1)?,
                Some(format!("HSTX lane {lane} positive half-cycle")),
            )?);
            negative.push(hub.declare(
                format!("{path}.lane{lane}.n"),
                SignalValue::repeat(Logic::Zero, 1)?,
                Some(format!("HSTX lane {lane} negative half-cycle")),
            )?);
        }
        let clock = hub.declare(
            format!("{path}.clock"),
            SignalValue::repeat(Logic::Zero, 1)?,
            Some("HSTX generated clock".to_owned()),
        )?;
        let handle = HstxHandle::default();
        Ok((
            Self {
                csr: CSR_RESET,
                bit: [0; 8],
                expand_shift: EXPAND_SHIFT_RESET,
                expand_tmds: 0,
                fifo: VecDeque::with_capacity(FIFO_CAPACITY),
                write_overflow: false,
                phase: false,
                hub,
                positive,
                negative,
                clock,
                handle: handle.clone(),
            },
            handle,
        ))
    }

    fn reset_state(&mut self) {
        self.csr = CSR_RESET;
        self.bit = [0; 8];
        self.expand_shift = EXPAND_SHIFT_RESET;
        self.expand_tmds = 0;
        self.fifo.clear();
        self.write_overflow = false;
        self.phase = false;
        self.handle.clear();
    }

    fn fifo_status(&self) -> u32 {
        let level = u32::try_from(self.fifo.len()).expect("HSTX FIFO level fits");
        level
            | ((self.fifo.len() == FIFO_CAPACITY) as u32 * FIFO_FULL)
            | ((self.fifo.is_empty() as u32) * FIFO_EMPTY)
            | ((self.write_overflow as u32) * FIFO_WOF)
    }

    fn push_fifo(&mut self, word: u32, at: SimTime) -> Result<(), DeviceError> {
        if self.fifo.len() == FIFO_CAPACITY {
            self.write_overflow = true;
            return Ok(());
        }
        self.fifo.push_back(word);
        if self.csr & CSR_EN != 0 {
            self.drain_fifo(at)?;
        }
        Ok(())
    }

    fn drain_fifo(&mut self, at: SimTime) -> Result<(), DeviceError> {
        while self.csr & CSR_EN != 0 {
            let Some(word) = self.fifo.pop_front() else {
                break;
            };
            self.emit_word(word, at)?;
        }
        Ok(())
    }

    fn emit_word(&mut self, word: u32, at: SimTime) -> Result<(), DeviceError> {
        let clock = self.phase;
        let mut positive = [false; 8];
        let mut negative = [false; 8];
        for lane in 0..8 {
            let config = self.bit[lane];
            let invert = config & (1 << 16) != 0;
            if config & (1 << 17) != 0 {
                positive[lane] = clock;
                negative[lane] = !clock;
            } else {
                let p = ((word >> (config & 0x1f)) & 1) != 0;
                let n = ((word >> ((config >> 8) & 0x1f)) & 1) != 0;
                positive[lane] = p;
                negative[lane] = n;
            }
            if invert {
                positive[lane] = !positive[lane];
                negative[lane] = !negative[lane];
            }
            self.hub
                .set(
                    self.positive[lane],
                    SignalValue::repeat(
                        if positive[lane] {
                            Logic::One
                        } else {
                            Logic::Zero
                        },
                        1,
                    )
                    .map_err(|error| DeviceError::new(error.to_string()))?,
                    at,
                )
                .map_err(|error| DeviceError::new(error.to_string()))?;
            self.hub
                .set(
                    self.negative[lane],
                    SignalValue::repeat(
                        if negative[lane] {
                            Logic::One
                        } else {
                            Logic::Zero
                        },
                        1,
                    )
                    .map_err(|error| DeviceError::new(error.to_string()))?,
                    at,
                )
                .map_err(|error| DeviceError::new(error.to_string()))?;
        }
        self.hub
            .set(
                self.clock,
                SignalValue::repeat(if clock { Logic::One } else { Logic::Zero }, 1)
                    .map_err(|error| DeviceError::new(error.to_string()))?,
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))?;
        let sample = HstxSample {
            word,
            positive,
            negative,
            clock,
        };
        let mut samples = self
            .handle
            .samples
            .lock()
            .expect("HSTX capture lock poisoned");
        samples.push(sample);
        if samples.len() > 4096 {
            samples.remove(0);
        }
        self.phase = !self.phase;
        Ok(())
    }

    fn write_control(
        &mut self,
        register: u64,
        alias: u64,
        value: u32,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        match register {
            0x00 => {
                let was_enabled = self.csr & CSR_EN != 0;
                self.csr = atomic_update(self.csr, alias, value)? & CSR_MASK;
                if self.csr & CSR_EN == 0 {
                    self.phase = false;
                } else if !was_enabled || !self.fifo.is_empty() {
                    self.drain_fifo(at)?;
                }
            }
            0x04..=0x20 if (register - 0x04) % 4 == 0 => {
                let index = usize::try_from((register - 0x04) / 4).expect("HSTX lane fits");
                self.bit[index] = atomic_update(self.bit[index], alias, value)? & BIT_MASK;
            }
            0x24 => {
                self.expand_shift =
                    atomic_update(self.expand_shift, alias, value)? & EXPAND_SHIFT_MASK;
            }
            0x28 => {
                self.expand_tmds =
                    atomic_update(self.expand_tmds, alias, value)? & EXPAND_TMDS_MASK;
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 HSTX control write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn read_control(&self, register: u64) -> Result<u32, DeviceError> {
        match register {
            0x00 => Ok(self.csr),
            0x04..=0x20 if (register - 0x04) % 4 == 0 => {
                Ok(self.bit[usize::try_from((register - 0x04) / 4).expect("HSTX lane fits")])
            }
            0x24 => Ok(self.expand_shift),
            0x28 => Ok(self.expand_tmds),
            _ => Err(DeviceError::new(format!(
                "unmodeled RP2350 HSTX control read at offset {register:#x}"
            ))),
        }
    }
}

/// Functional HSTX control/register block at `0x400c0000`.
pub struct Rp2350HstxCtrl {
    name: String,
    state: Rc<RefCell<HstxState>>,
}

/// Functional HSTX FIFO block at `0x50600000`.
pub struct Rp2350HstxFifo {
    name: String,
    state: Rc<RefCell<HstxState>>,
}

/// Creates the RP2350 HSTX control block, FIFO block, and output capture.
pub fn new_rp2350_hstx(
    name: impl Into<String>,
    path: &str,
    hub: SignalHub,
) -> Result<(Rp2350HstxCtrl, Rp2350HstxFifo, HstxHandle), SignalError> {
    let name = name.into();
    let ctrl_name = format!("{name}.ctrl");
    let fifo_name = format!("{name}.fifo");
    let (state, handle) = HstxState::new(path, hub)?;
    let state = Rc::new(RefCell::new(state));
    Ok((
        Rp2350HstxCtrl {
            name: ctrl_name,
            state: state.clone(),
        },
        Rp2350HstxFifo {
            name: fifo_name,
            state,
        },
        handle,
    ))
}

impl Device for Rp2350HstxCtrl {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        aligned_word(width, offset)?;
        Ok(u64::from(
            self.state.borrow().read_control(offset & 0x0fff)?,
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        aligned_word(width, offset)?;
        self.state
            .borrow_mut()
            .write_control(offset & 0x0fff, (offset >> 12) & 3, value as u32, at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset_state();
    }
}

impl Device for Rp2350HstxFifo {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        aligned_word(width, offset)?;
        if offset & 0x0fff != 0 {
            return Err(DeviceError::new(format!(
                "unmodeled RP2350 HSTX FIFO read at offset {:#x}",
                offset & 0x0fff
            )));
        }
        Ok(u64::from(self.state.borrow().fifo_status()))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        aligned_word(width, offset)?;
        let register = offset & 0x0fff;
        let mut state = self.state.borrow_mut();
        match register {
            0x00 => {
                if value as u32 & FIFO_WOF != 0 {
                    state.write_overflow = false;
                }
            }
            0x04 => state.push_fifo(value as u32, at)?,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 HSTX FIFO write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset_state();
    }
}
