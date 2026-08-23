use super::*;

struct TimerState {
    enabled: bool,
    periodic: bool,
    compare: u64,
    period: u64,
    pending: bool,
}

/// Host/machine-facing timer state.
#[derive(Clone)]
pub struct TimerHandle {
    state: Rc<RefCell<TimerState>>,
}

impl TimerHandle {
    /// Whether the timer can currently change or assert its interrupt line.
    pub fn active(&self) -> bool {
        let state = self.state.borrow();
        state.enabled || state.pending
    }

    /// Updates pending state at the current simulation time.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        if state.enabled && now.ticks() >= state.compare {
            state.pending = true;
            if state.periodic && state.period != 0 {
                while state.compare <= now.ticks() {
                    state.compare = state.compare.saturating_add(state.period);
                }
            } else {
                state.enabled = false;
            }
        }
        state.pending
    }

    /// Clears the interrupt pending latch.
    pub fn clear(&self) {
        self.state.borrow_mut().pending = false;
    }

    /// Current pending state.
    pub fn pending(&self) -> bool {
        self.state.borrow().pending
    }
}

/// Functional timer with counter, compare, control, period, and status words.
pub struct FunctionalTimer {
    name: String,
    state: Rc<RefCell<TimerState>>,
}

impl FunctionalTimer {
    /// Counter offset.
    pub const COUNTER: u64 = 0x00;
    /// Compare offset.
    pub const COMPARE: u64 = 0x08;
    /// Control offset: bit 0 enable, bit 1 periodic.
    pub const CONTROL: u64 = 0x10;
    /// Period offset.
    pub const PERIOD: u64 = 0x18;
    /// Status offset: bit 0 pending; write bit 0 to clear.
    pub const STATUS: u64 = 0x20;

    /// Creates a stopped timer and machine handle.
    pub fn new(name: impl Into<String>) -> (Self, TimerHandle) {
        let state = Rc::new(RefCell::new(TimerState {
            enabled: false,
            periodic: false,
            compare: u64::MAX,
            period: 0,
            pending: false,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            TimerHandle { state },
        )
    }
}

impl Device for FunctionalTimer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if !matches!(width, AccessWidth::Word | AccessWidth::DoubleWord) {
            return Err(DeviceError::new(
                "timer requires word or double-word access",
            ));
        }
        let state = self.state.borrow();
        match offset {
            Self::COUNTER => Ok(at.ticks()),
            Self::COMPARE => Ok(state.compare),
            Self::CONTROL => Ok(u64::from(state.enabled) | (u64::from(state.periodic) << 1)),
            Self::PERIOD => Ok(state.period),
            Self::STATUS => Ok(u64::from(state.pending)),
            _ => Err(DeviceError::new(format!(
                "unmodeled timer read at offset {offset:#x}"
            ))),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if !matches!(width, AccessWidth::Word | AccessWidth::DoubleWord) {
            return Err(DeviceError::new(
                "timer requires word or double-word access",
            ));
        }
        let mut state = self.state.borrow_mut();
        match offset {
            Self::COMPARE => state.compare = value,
            Self::CONTROL => {
                state.enabled = value & 1 != 0;
                state.periodic = value & 2 != 0;
            }
            Self::PERIOD => state.period = value,
            Self::STATUS if value & 1 != 0 => state.pending = false,
            Self::STATUS => {}
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled timer write at offset {offset:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.borrow_mut();
        state.enabled = false;
        state.periodic = false;
        state.compare = u64::MAX;
        state.period = 0;
        state.pending = false;
    }
}

/// Host handle for a deterministic exit convention.
#[derive(Clone, Default)]
pub struct ExitHandle {
    code: Rc<Cell<Option<u32>>>,
}

impl ExitHandle {
    /// Returns a requested exit code.
    pub fn code(&self) -> Option<u32> {
        self.code.get()
    }
}

/// Write-only MMIO exit device.
pub struct ExitDevice {
    name: String,
    handle: ExitHandle,
}

impl ExitDevice {
    /// Creates an exit device and observation handle.
    pub fn new(name: impl Into<String>) -> (Self, ExitHandle) {
        let handle = ExitHandle::default();
        (
            Self {
                name: name.into(),
                handle: handle.clone(),
            },
            handle,
        )
    }
}

impl Device for ExitDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(
        &mut self,
        _offset: u64,
        _width: AccessWidth,
        _at: SimTime,
    ) -> Result<u64, DeviceError> {
        Ok(self.handle.code().map_or(0, u64::from))
    }

    fn write(
        &mut self,
        offset: u64,
        _width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if offset != 0 {
            return Err(DeviceError::new("exit device only implements offset zero"));
        }
        self.handle.code.set(Some(
            u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits in u32"),
        ));
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.handle.code.set(None);
    }
}

/// Sparse functional register bank for clock/reset facades and documented reset values.
pub struct RegisterBank {
    name: String,
    reset: BTreeMap<u64, u32>,
    values: BTreeMap<u64, u32>,
    writable_masks: BTreeMap<u64, u32>,
}

impl RegisterBank {
    /// Constructs a bank from `(offset, reset_value, writable_mask)` entries.
    pub fn new(
        name: impl Into<String>,
        registers: impl IntoIterator<Item = (u64, u32, u32)>,
    ) -> Self {
        let mut reset = BTreeMap::new();
        let mut writable_masks = BTreeMap::new();
        for (offset, value, mask) in registers {
            reset.insert(offset, value);
            writable_masks.insert(offset, mask);
        }
        Self {
            name: name.into(),
            values: reset.clone(),
            reset,
            writable_masks,
        }
    }
}

impl Device for RegisterBank {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("register bank requires word access"));
        }
        self.values
            .get(&offset)
            .copied()
            .map(u64::from)
            .ok_or_else(|| {
                DeviceError::new(format!("unmodeled register read at offset {offset:#x}"))
            })
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("register bank requires word access"));
        }
        let current = self.values.get_mut(&offset).ok_or_else(|| {
            DeviceError::new(format!("unmodeled register write at offset {offset:#x}"))
        })?;
        let mask = self.writable_masks[&offset];
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked value always fits in u32");
        *current = (*current & !mask) | (value & mask);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.values.clone_from(&self.reset);
    }
}
