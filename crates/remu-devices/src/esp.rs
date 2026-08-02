use super::*;

/// ESP timer-group register layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EspTimerGroupKind {
    /// ESP32-C6 timer group with one general-purpose timer.
    Esp32C6,
    /// ESP32-S3 timer group with two general-purpose timers.
    Esp32S3,
}

impl EspTimerGroupKind {
    const fn timer_count(self) -> usize {
        match self {
            Self::Esp32C6 => 1,
            Self::Esp32S3 => 2,
        }
    }
}

#[derive(Clone, Copy)]
struct EspTimerCounter {
    base_value: u64,
    base_time: SimTime,
    latched_value: u64,
}

struct EspTimerGroupState {
    registers: Vec<u32>,
    counters: [EspTimerCounter; 2],
    kind: EspTimerGroupKind,
}

impl EspTimerGroupState {
    const TIMER_STRIDE: usize = 0x24;
    const CONFIG: usize = 0x00;
    const COUNTER_LOW: usize = 0x04;
    const COUNTER_HIGH: usize = 0x08;
    const UPDATE: usize = 0x0c;
    const ALARM_LOW: usize = 0x10;
    const ALARM_HIGH: usize = 0x14;
    const LOAD_LOW: usize = 0x18;
    const LOAD_HIGH: usize = 0x1c;
    const LOAD: usize = 0x20;
    const INTERRUPT_ENABLE: usize = 0x70;
    const INTERRUPT_RAW: usize = 0x74;
    const INTERRUPT_STATUS: usize = 0x78;
    const INTERRUPT_CLEAR: usize = 0x7c;
    const COUNTER_MASK: u64 = (1_u64 << 54) - 1;

    fn new(kind: EspTimerGroupKind) -> Self {
        let counter = EspTimerCounter {
            base_value: 0,
            base_time: SimTime::ZERO,
            latched_value: 0,
        };
        let mut state = Self {
            registers: vec![0; 0x1000 / 4],
            counters: [counter; 2],
            kind,
        };
        state.reset();
        state
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.counters.fill(EspTimerCounter {
            base_value: 0,
            base_time: SimTime::ZERO,
            latched_value: 0,
        });
        // The RTC calibration block completes synchronously in the functional
        // timing model. This represents a nominal 136 kHz slow clock measured
        // against a 40 MHz crystal.
        self.registers[0x68 / 4] = (1 << 12) | (1 << 15);
        self.registers[0x6c / 4] = (301_176 << 7) | 1;
        self.registers[0x80 / 4] = (3 << 3) | (0x01ff_ffff << 7);
        self.registers[0xf8 / 4] = 35_676_274;
    }

    fn timer_register(&self, offset: usize) -> Option<(usize, usize)> {
        let timer = offset / Self::TIMER_STRIDE;
        let register = offset % Self::TIMER_STRIDE;
        (timer < self.kind.timer_count()).then_some((timer, register))
    }

    fn register(&self, timer: usize, register: usize) -> u32 {
        self.registers[(timer * Self::TIMER_STRIDE + register) / 4]
    }

    fn set_register(&mut self, timer: usize, register: usize, value: u32) {
        self.registers[(timer * Self::TIMER_STRIDE + register) / 4] = value;
    }

    fn divider(config: u32) -> u64 {
        match (config >> 13) & 0xffff {
            0 => 65_536,
            1 | 2 => 2,
            divider => u64::from(divider),
        }
    }

    fn counter_value(&self, timer: usize, now: SimTime) -> u64 {
        let counter = self.counters[timer];
        let config = self.register(timer, Self::CONFIG);
        if config & (1 << 31) == 0 {
            return counter.base_value;
        }
        let elapsed = now.ticks().saturating_sub(counter.base_time.ticks());
        // The functional timeline uses eight abstract source counts per
        // instruction. This preserves the divider relationship while avoiding
        // a claim of wall-clock or cycle accuracy.
        let increment = elapsed
            .saturating_mul(8)
            .checked_div(Self::divider(config))
            .unwrap_or(0);
        if config & (1 << 30) != 0 {
            counter.base_value.wrapping_add(increment) & Self::COUNTER_MASK
        } else {
            counter.base_value.wrapping_sub(increment) & Self::COUNTER_MASK
        }
    }

    fn materialize(&mut self, timer: usize, now: SimTime) {
        let value = self.counter_value(timer, now);
        self.counters[timer].base_value = value;
        self.counters[timer].base_time = now;
    }

    fn load_value(&self, timer: usize) -> u64 {
        (u64::from(self.register(timer, Self::LOAD_HIGH) & 0x003f_ffff) << 32)
            | u64::from(self.register(timer, Self::LOAD_LOW))
    }

    fn alarm_value(&self, timer: usize) -> u64 {
        (u64::from(self.register(timer, Self::ALARM_HIGH) & 0x003f_ffff) << 32)
            | u64::from(self.register(timer, Self::ALARM_LOW))
    }

    fn advance(&mut self, now: SimTime) {
        for timer in 0..self.kind.timer_count() {
            let config = self.register(timer, Self::CONFIG);
            let mask = 1_u32 << timer;
            if config & ((1 << 31) | (1 << 10)) != ((1 << 31) | (1 << 10))
                || self.registers[Self::INTERRUPT_RAW / 4] & mask != 0
            {
                continue;
            }
            let counter = self.counter_value(timer, now);
            let alarm = self.alarm_value(timer);
            let reached = if config & (1 << 30) != 0 {
                counter >= alarm
            } else {
                counter <= alarm
            };
            if !reached {
                continue;
            }

            self.registers[Self::INTERRUPT_RAW / 4] |= mask;
            self.set_register(timer, Self::CONFIG, config & !(1 << 10));
            if config & (1 << 29) != 0 {
                self.counters[timer].base_value = self.load_value(timer);
            } else {
                self.counters[timer].base_value = counter;
            }
            self.counters[timer].base_time = now;
        }
        self.registers[Self::INTERRUPT_STATUS / 4] =
            self.registers[Self::INTERRUPT_RAW / 4] & self.registers[Self::INTERRUPT_ENABLE / 4];
    }
}

/// Interrupt view of one ESP timer group.
#[derive(Clone)]
pub struct EspTimerGroupHandle {
    state: Rc<RefCell<EspTimerGroupState>>,
}

impl EspTimerGroupHandle {
    /// Advances the timers and returns masked timer interrupt levels.
    pub fn pending(&self, now: SimTime) -> [bool; 2] {
        let mut state = self.state.borrow_mut();
        state.advance(now);
        let status = state.registers[EspTimerGroupState::INTERRUPT_STATUS / 4];
        [status & 1 != 0, status & 2 != 0]
    }
}

/// Functional ESP32-C6/S3 general-purpose timer group and RTC calibration block.
pub struct EspTimerGroup {
    name: String,
    state: Rc<RefCell<EspTimerGroupState>>,
}

impl EspTimerGroup {
    /// Creates a reset timer group and scheduler-facing interrupt handle.
    pub fn new(name: impl Into<String>, kind: EspTimerGroupKind) -> (Self, EspTimerGroupHandle) {
        let state = Rc::new(RefCell::new(EspTimerGroupState::new(kind)));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspTimerGroupHandle { state },
        )
    }
}

impl Device for EspTimerGroup {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP timer group requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("timer-group offset fits");
        let mut state = self.state.borrow_mut();
        state.advance(at);
        state
            .registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
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
                "ESP timer group requires aligned word access",
            ));
        }
        let offset = usize::try_from(offset).expect("timer-group offset fits");
        let index = offset / 4;
        let value = value as u32;
        let mut state = self.state.borrow_mut();
        if index >= state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        state.advance(at);
        if std::env::var_os("REMU_DEBUG_TIMERS").is_some() && offset <= 0x80 {
            eprintln!(
                "{} write at={} offset={offset:#04x} value={value:#010x}",
                self.name,
                at.ticks(),
            );
        }

        if let Some((timer, register)) = state.timer_register(offset) {
            match register {
                EspTimerGroupState::CONFIG => {
                    state.materialize(timer, at);
                    state.set_register(timer, register, value);
                }
                EspTimerGroupState::UPDATE => {
                    state.counters[timer].latched_value = state.counter_value(timer, at);
                    let latched = state.counters[timer].latched_value;
                    state.set_register(timer, EspTimerGroupState::COUNTER_LOW, latched as u32);
                    state.set_register(
                        timer,
                        EspTimerGroupState::COUNTER_HIGH,
                        u32::try_from(latched >> 32).expect("54-bit timer high word fits"),
                    );
                }
                EspTimerGroupState::LOAD => {
                    let load = state.load_value(timer);
                    state.counters[timer].base_value = load;
                    state.counters[timer].base_time = at;
                    state.counters[timer].latched_value = load;
                    state.set_register(timer, register, 0);
                }
                _ => state.set_register(timer, register, value),
            }
        } else {
            match offset {
                EspTimerGroupState::INTERRUPT_ENABLE => {
                    state.registers[index] = value & 3;
                }
                EspTimerGroupState::INTERRUPT_RAW | EspTimerGroupState::INTERRUPT_STATUS => {}
                EspTimerGroupState::INTERRUPT_CLEAR => {
                    state.registers[EspTimerGroupState::INTERRUPT_RAW / 4] &= !(value & 3);
                    state.registers[index] = 0;
                }
                _ => state.registers[index] = value,
            }
        }

        if offset == 0x68 && value & (1 << 31) != 0 {
            let calibration_cycles = ((value >> 16) & 0x7fff).max(1);
            let measured_xtal_cycles = (40_000_000_u64 * u64::from(calibration_cycles)) / 136_000;
            state.registers[0x68 / 4] |= 1 << 15;
            state.registers[0x6c / 4] =
                (u32::try_from(measured_xtal_cycles).unwrap_or(u32::MAX) & 0x01ff_ffff) << 7;
            state.registers[0x80 / 4] &= !1;
        }
        state.advance(at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}

/// Functional ESP32-S3 RTC control block with a latched 48-bit time counter.
pub struct EspRtcControl {
    name: String,
    registers: Vec<u32>,
}

impl EspRtcControl {
    /// Creates the RTC control page in its power-on state.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: vec![0; 0x1000 / 4],
        }
    }
}

impl Device for EspRtcControl {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP RTC control requires aligned word access",
            ));
        }
        self.registers
            .get(usize::try_from(offset / 4).expect("RTC offset fits"))
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
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
                "ESP RTC control requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("RTC offset fits");
        let register = self
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        *register = value as u32;
        if offset == 0x0c && value & (1 << 31) != 0 {
            let counter = at.ticks();
            self.registers[0x10 / 4] = counter as u32;
            self.registers[0x14 / 4] = (counter >> 32) as u32;
        }
        // SENS_SAR_MEAS1_CTRL2 shares the RTC peripheral page at 0x800.
        // A software-triggered functional conversion completes immediately.
        // Keep the selected pad/control fields, clear START, assert DONE, and
        // return a deterministic zero sample in the low 16 bits.
        if matches!(offset, 0x80c | 0x830) && value & (1 << 17) != 0 {
            self.registers[index] = (value as u32 & !((1 << 17) | 0xffff)) | (1 << 16);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
    }
}

#[derive(Default)]
struct EspSystemState {
    registers: Vec<u32>,
    from_cpu_pending: [bool; 4],
}

/// Observation handle for the ESP32-S3 system block's cross-core interrupts.
#[derive(Clone)]
pub struct EspSystemHandle {
    state: Rc<RefCell<EspSystemState>>,
}

impl EspSystemHandle {
    /// Reports whether one FROM_CPU interrupt source is asserted.
    pub fn from_cpu_pending(&self, source: usize) -> bool {
        self.state
            .borrow()
            .from_cpu_pending
            .get(source)
            .copied()
            .unwrap_or(false)
    }
}

/// Functional ESP32-S3 system register page.
///
/// Most registers retain software-written configuration. FROM_CPU trigger
/// registers additionally expose their level to the machine interrupt router.
pub struct EspSystem {
    name: String,
    state: Rc<RefCell<EspSystemState>>,
}

impl EspSystem {
    /// Creates the system register page and its interrupt observation handle.
    pub fn new(name: impl Into<String>) -> (Self, EspSystemHandle) {
        let state = Rc::new(RefCell::new(EspSystemState {
            registers: vec![0; 0x1000 / 4],
            from_cpu_pending: [false; 4],
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspSystemHandle { state },
        )
    }
}

impl Device for EspSystem {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP system block requires aligned word access",
            ));
        }
        self.state
            .borrow()
            .registers
            .get(usize::try_from(offset / 4).expect("system offset fits"))
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP system block requires aligned word access",
            ));
        }
        let mut state = self.state.borrow_mut();
        let index = usize::try_from(offset / 4).expect("system offset fits");
        let register = state
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        *register = value as u32;
        match offset {
            0x30 => state.from_cpu_pending[0] = value & 1 != 0,
            0x34 => state.from_cpu_pending[1] = value & 1 != 0,
            0x38 => state.from_cpu_pending[2] = value & 1 != 0,
            0x3c => state.from_cpu_pending[3] = value & 1 != 0,
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.borrow_mut();
        state.registers.fill(0);
        state.from_cpu_pending = [false; 4];
    }
}

#[derive(Default)]
struct EspMmuTableState {
    registers: Vec<u32>,
    pending: Vec<(usize, u32)>,
}

/// Observation handle for ESP32-S3 cache-MMU entry updates.
#[derive(Clone)]
pub struct EspMmuTableHandle {
    state: Arc<Mutex<EspMmuTableState>>,
}

impl EspMmuTableHandle {
    /// Drains page-table writes in architectural order.
    pub fn drain_mappings(&self) -> Vec<(usize, u32)> {
        let mut state = self.state.lock().expect("ESP MMU state lock poisoned");
        std::mem::take(&mut state.pending)
    }

    /// Establishes one boot-time MMU entry and queues its backing-store map.
    pub fn set_mapping(&self, index: usize, entry: u32) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("ESP MMU state lock poisoned");
        let register = state
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("ESP MMU entry {index} is out of range")))?;
        *register = entry;
        state.pending.push((index, entry));
        Ok(())
    }
}

/// Functional ESP32-S3 cache-MMU table.
pub struct EspMmuTable {
    name: String,
    state: Arc<Mutex<EspMmuTableState>>,
}

impl EspMmuTable {
    /// Creates the MMU table and its mapping observation handle.
    pub fn new(name: impl Into<String>) -> (Self, EspMmuTableHandle) {
        let state = Arc::new(Mutex::new(EspMmuTableState {
            // ESP32-S3 uses bit 14, rather than the older ESP32 bit-8
            // convention, to mark a cache-MMU entry invalid.
            registers: vec![0x4000; 0x1000 / 4],
            pending: Vec::new(),
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspMmuTableHandle { state },
        )
    }
}

impl Device for EspMmuTable {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP MMU table requires aligned word access",
            ));
        }
        self.state
            .lock()
            .expect("ESP MMU state lock poisoned")
            .registers
            .get(usize::try_from(offset / 4).expect("MMU offset fits"))
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP MMU table requires aligned word access",
            ));
        }
        let mut state = self.state.lock().expect("ESP MMU state lock poisoned");
        let index = usize::try_from(offset / 4).expect("MMU offset fits");
        let value = value as u32;
        let register = state
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        *register = value;
        state.pending.push((index, value));
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ESP MMU state lock poisoned");
        state.registers.fill(0x4000);
        state.pending.clear();
    }
}

#[derive(Default)]
struct EspSystimerState {
    registers: Vec<u32>,
    latched: [u64; 2],
}

/// Observation and interrupt handle for the ESP32-S3 system timer.
#[derive(Clone)]
pub struct EspSystimerHandle {
    state: Rc<RefCell<EspSystimerState>>,
}

impl EspSystimerHandle {
    /// Advances comparator state and returns enabled target interrupts.
    pub fn pending(&self, now: SimTime) -> [bool; 3] {
        const COUNTER_MASK: u64 = (1_u64 << 52) - 1;
        let mut state = self.state.borrow_mut();
        let current = now.ticks() & COUNTER_MASK;
        let config = state.registers[0];
        for target in 0..3 {
            let work_enable = 1_u32 << (24 - target);
            if config & work_enable == 0 {
                continue;
            }
            let high = u64::from(state.registers[(0x1c + target * 8) / 4] & 0x000f_ffff);
            let low = u64::from(state.registers[(0x20 + target * 8) / 4]);
            let compare = ((high << 32) | low) & COUNTER_MASK;
            if current >= compare {
                state.registers[0x68 / 4] |= 1 << target;
                let target_config = state.registers[(0x34 + target * 4) / 4];
                if target_config & (1 << 30) != 0 {
                    let period = u64::from(target_config & 0x03ff_ffff).max(1);
                    let elapsed_periods = current.saturating_sub(compare) / period + 1;
                    let next =
                        compare.wrapping_add(elapsed_periods.saturating_mul(period)) & COUNTER_MASK;
                    state.registers[(0x1c + target * 8) / 4] =
                        u32::try_from(next >> 32).expect("52-bit high word fits");
                    state.registers[(0x20 + target * 8) / 4] = next as u32;
                } else {
                    state.registers[0] &= !work_enable;
                }
            }
        }
        let asserted = state.registers[0x68 / 4] & state.registers[0x64 / 4];
        [asserted & 1 != 0, asserted & 2 != 0, asserted & 4 != 0]
    }
}

/// Functional ESP32-S3 system timer with synchronous counter latching.
pub struct EspSystimer {
    name: String,
    state: Rc<RefCell<EspSystimerState>>,
}

impl EspSystimer {
    /// Creates a continuously running 52-bit system timer.
    pub fn new(name: impl Into<String>) -> (Self, EspSystimerHandle) {
        let mut registers = vec![0; 0x1000 / 4];
        registers[0] = 1 << 30;
        let state = Rc::new(RefCell::new(EspSystimerState {
            registers,
            latched: [0; 2],
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspSystimerHandle { state },
        )
    }
}

impl Device for EspSystimer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP system timer requires aligned word access",
            ));
        }
        let state = self.state.borrow();
        let value = match offset {
            0x04 | 0x08 => 1 << 29,
            0x40 => (state.latched[0] >> 32) as u32,
            0x44 => state.latched[0] as u32,
            0x48 => (state.latched[1] >> 32) as u32,
            0x4c => state.latched[1] as u32,
            _ => state
                .registers
                .get(usize::try_from(offset / 4).expect("systimer offset fits"))
                .copied()
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?,
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
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP system timer requires aligned word access",
            ));
        }
        let mut state = self.state.borrow_mut();
        if matches!(offset, 0x04 | 0x08) {
            let unit = usize::from(offset == 0x08);
            state.latched[unit] = at.ticks() & ((1_u64 << 52) - 1);
            return Ok(());
        }
        if offset == 0x6c {
            state.registers[0x68 / 4] &= !(value as u32 & 0x7);
            return Ok(());
        }
        if matches!(offset, 0x50 | 0x54 | 0x58) && value & 1 != 0 {
            let target = usize::try_from((offset - 0x50) / 4).expect("three targets fit usize");
            let target_config = state.registers[(0x34 + target * 4) / 4];
            let period = u64::from(target_config & 0x03ff_ffff).max(1);
            let compare = at.ticks().wrapping_add(period) & ((1_u64 << 52) - 1);
            state.registers[(0x1c + target * 8) / 4] =
                u32::try_from(compare >> 32).expect("52-bit high word fits");
            state.registers[(0x20 + target * 8) / 4] = compare as u32;
        }
        let register = state
            .registers
            .get_mut(usize::try_from(offset / 4).expect("systimer offset fits"))
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        *register = value as u32;
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.borrow_mut();
        state.registers.fill(0);
        state.registers[0] = 1 << 30;
        state.latched = [0; 2];
    }
}

/// Functional register slice of the ESP32-S3 Synopsys DWC2 USB OTG core.
///
/// The core reset handshake is synchronous in Renvo Emulator's abstract-time model.
/// Endpoint and host-enumeration behavior is layered onto this register file
/// by the machine as qualification reaches the TinyUSB device path.
pub struct EspUsbOtg {
    name: String,
    state: Arc<Mutex<EspUsbOtgState>>,
}

struct EspUsbOtgState {
    registers: Vec<u32>,
    rx_status: VecDeque<u32>,
    rx_fifo: VecDeque<u32>,
    tx_fifo: Vec<Vec<u8>>,
    in_transfer_size: [usize; 16],
    reset_injected: bool,
}

const DWC2_EPENA: u32 = 1 << 31;
const DWC2_EPDIS: u32 = 1 << 30;
const DWC2_GINT_RXFLVL: u32 = 1 << 4;
const DWC2_GINT_IEPINT: u32 = 1 << 18;
const DWC2_GINT_OEPINT: u32 = 1 << 19;

/// Host-side control surface for an ESP32-S3 DWC2 device controller.
#[derive(Clone)]
pub struct EspUsbOtgHandle {
    state: Arc<Mutex<EspUsbOtgState>>,
}

impl EspUsbOtg {
    /// Creates a reset device-mode DWC2 core.
    pub fn new(name: impl Into<String>) -> (Self, EspUsbOtgHandle) {
        let state = Arc::new(Mutex::new(EspUsbOtgState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspUsbOtgHandle { state },
        )
    }
}

impl EspUsbOtgState {
    fn register(&self, register: EspUsbOtgRegister) -> u32 {
        self.registers[register.index()]
    }

    fn register_mut(&mut self, register: EspUsbOtgRegister) -> &mut u32 {
        &mut self.registers[register.index()]
    }

    fn reset() -> Self {
        let mut registers = vec![0; 0x1_0000 / 4];
        // GRSTCTL.AHBIDL and Espressif's fixed DWC2 release identifier.
        registers[EspUsbOtgRegister::GrstCtl.index()] = 1 << 31;
        registers[EspUsbOtgRegister::GsnpsId.index()] = 0x4f54_400a;
        // Slave-only full-speed device core, six non-control endpoints and
        // dynamic FIFO sizing. TinyUSB uses NUM_DEV_EP to bound DAINT scans.
        registers[EspUsbOtgRegister::GhwCfg2.index()] = 4 | (1 << 8) | (6 << 10) | (1 << 19);
        // DSTS.ENUMSPD reports full speed on the S3's dedicated 48-MHz PHY.
        registers[EspUsbOtgRegister::Dsts.index()] = 3 << 1;
        // The functional FIFO drains synchronously into the host packet
        // queue, so each IN endpoint always reports the full 1-KiB shared
        // FIFO as available to TinyUSB.
        for endpoint in 0..16 {
            registers[EspUsbOtgRegister::DtxfSts(endpoint as u8).index()] = 256;
        }
        Self {
            registers,
            rx_status: VecDeque::new(),
            rx_fifo: VecDeque::new(),
            tx_fifo: vec![Vec::new(); 16],
            in_transfer_size: [0; 16],
            reset_injected: false,
        }
    }

    fn endpoint_interrupts(&self) -> u32 {
        let mut daint = 0_u32;
        let diepmsk = self.register(EspUsbOtgRegister::DiepMsk);
        let doepmsk = self.register(EspUsbOtgRegister::DoepMsk);
        let fifo_empty_mask = self.register(EspUsbOtgRegister::DiepEmpMsk);
        for endpoint in 0..16 {
            let endpoint = endpoint as u8;
            let mut input = self.register(EspUsbOtgRegister::DiepInt(endpoint));
            let fifo_empty = fifo_empty_mask & (1 << endpoint) != 0
                && self.register(EspUsbOtgRegister::DiepCtl(endpoint)) & DWC2_EPENA != 0;
            if fifo_empty {
                input |= 1 << 7;
            }
            // TXFE has its own DIEPEMPMSK hierarchy and does not pass
            // through the common DIEPMSK register.
            if input & diepmsk != 0 || fifo_empty {
                daint |= 1 << endpoint;
            }
            let output = self.register(EspUsbOtgRegister::DoepInt(endpoint));
            if output & doepmsk != 0 {
                daint |= 1 << (16 + endpoint);
            }
        }
        daint
    }

    fn interrupt_status(&self) -> u32 {
        let mut status = self.register(EspUsbOtgRegister::GintSts);
        if !self.rx_status.is_empty() {
            status |= DWC2_GINT_RXFLVL;
        }
        let endpoints = self.endpoint_interrupts() & self.register(EspUsbOtgRegister::DaintMsk);
        if endpoints & 0x0000_ffff != 0 {
            status |= DWC2_GINT_IEPINT;
        }
        if endpoints & 0xffff_0000 != 0 {
            status |= DWC2_GINT_OEPINT;
        }
        status
    }

    fn pop_rx_status(&mut self) -> u32 {
        let status = self.rx_status.pop_front().unwrap_or(0);
        let endpoint = u8::try_from(status & 0xf).expect("endpoint number fits");
        match (status >> 17) & 0xf {
            // SETUP_DONE asserts DOEPINT.SETUP after its status entry is popped.
            4 => *self.register_mut(EspUsbOtgRegister::DoepInt(endpoint)) |= 1 << 3,
            // RX_COMPLETE asserts the transfer-complete endpoint interrupt.
            3 => {
                *self.register_mut(EspUsbOtgRegister::DoepCtl(endpoint)) &= !(1 << 31);
                *self.register_mut(EspUsbOtgRegister::DoepInt(endpoint)) |= 1;
            }
            _ => {}
        }
        status
    }

    fn write_fifo(&mut self, endpoint: usize, value: u32) {
        let endpoint_id = endpoint as u8;
        let size_index = EspUsbOtgRegister::DiepTsiz(endpoint_id).index();
        let remaining =
            usize::try_from(self.register(EspUsbOtgRegister::DiepTsiz(endpoint_id)) & 0x7ffff)
                .expect("DWC2 transfer size fits usize");
        let count = remaining.min(4);
        self.tx_fifo[endpoint].extend_from_slice(&value.to_le_bytes()[..count]);
        self.registers[size_index] =
            (self.registers[size_index] & !0x7ffff) | (remaining - count) as u32;
    }
}

impl EspUsbOtgHandle {
    /// Returns whether TinyUSB has connected the device and enabled interrupts.
    pub fn device_connected(&self) -> bool {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        state.register(EspUsbOtgRegister::Dctl) & (1 << 1) == 0
            && state.register(EspUsbOtgRegister::GahbCfg) & 1 != 0
    }

    /// Injects full-speed bus reset and enumeration-complete conditions once.
    pub fn inject_bus_reset(&self) {
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        // A USB reset returns the device address and endpoint enable state to their
        // post-reset values while retaining the software interrupt masks.
        *state.register_mut(EspUsbOtgRegister::Dcfg) &= !(0x7f << 4);
        *state.register_mut(EspUsbOtgRegister::Dctl) &= !(DWC2_EPENA | DWC2_EPDIS);
        for endpoint in 0..16 {
            let endpoint_id = endpoint as u8;
            *state.register_mut(EspUsbOtgRegister::DiepCtl(endpoint_id)) &=
                !(DWC2_EPENA | DWC2_EPDIS);
            *state.register_mut(EspUsbOtgRegister::DoepCtl(endpoint_id)) &=
                !(DWC2_EPENA | DWC2_EPDIS);
            *state.register_mut(EspUsbOtgRegister::DiepInt(endpoint_id)) = 0;
            *state.register_mut(EspUsbOtgRegister::DoepInt(endpoint_id)) = 0;
            state.in_transfer_size[endpoint] = 0;
            state.tx_fifo[endpoint].clear();
        }
        state.rx_status.clear();
        state.rx_fifo.clear();
        *state.register_mut(EspUsbOtgRegister::GintSts) |= (1 << 12) | (1 << 13);
        state.reset_injected = true;
    }

    /// Returns whether a globally enabled DWC2 interrupt is asserted.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        state.register(EspUsbOtgRegister::GahbCfg) & 1 != 0
            && state.interrupt_status() & state.register(EspUsbOtgRegister::GintMsk) != 0
    }

    /// Returns key interrupt registers for deterministic diagnostics.
    pub fn interrupt_diagnostic(&self) -> (u32, u32, u32, u32, u32) {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        (
            state.register(EspUsbOtgRegister::GahbCfg),
            state.interrupt_status(),
            state.register(EspUsbOtgRegister::GintMsk),
            state.endpoint_interrupts(),
            state.register(EspUsbOtgRegister::DaintMsk),
        )
    }

    /// Returns endpoint register state for deterministic diagnostics.
    pub fn endpoint_diagnostic(&self, endpoint: u8) -> (u32, u32, u32, u32, u32, u32, u32) {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        let endpoint = usize::from(endpoint);
        (
            state.register(EspUsbOtgRegister::DiepCtl(endpoint as u8)),
            state.register(EspUsbOtgRegister::DiepInt(endpoint as u8)),
            state.register(EspUsbOtgRegister::DiepTsiz(endpoint as u8)),
            state.register(EspUsbOtgRegister::DoepCtl(endpoint as u8)),
            state.register(EspUsbOtgRegister::DoepInt(endpoint as u8)),
            state.register(EspUsbOtgRegister::DoepTsiz(endpoint as u8)),
            state.register(EspUsbOtgRegister::DiepEmpMsk),
        )
    }

    /// Places a host SETUP packet in receive FIFO zero.
    pub fn inject_setup(&self, setup: [u8; 8]) {
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        state.rx_fifo.push_back(u32::from_le_bytes(
            setup[0..4].try_into().expect("four bytes"),
        ));
        state.rx_fifo.push_back(u32::from_le_bytes(
            setup[4..8].try_into().expect("four bytes"),
        ));
        state.rx_status.push_back((8 << 4) | (6 << 17));
        state.rx_status.push_back(4 << 17);
    }

    /// Returns whether an IN endpoint has a complete packet ready for the host.
    pub fn input_ready(&self, endpoint: u8) -> bool {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        let endpoint = usize::from(endpoint);
        let control = state.register(EspUsbOtgRegister::DiepCtl(endpoint as u8));
        let remaining = state.register(EspUsbOtgRegister::DiepTsiz(endpoint as u8)) & 0x7ffff;
        control & DWC2_EPENA != 0
            && remaining == 0
            && state.tx_fifo[endpoint].len() >= state.in_transfer_size[endpoint]
    }

    /// Consumes one device-to-host packet and asserts transfer completion.
    pub fn take_input(&self, endpoint: u8) -> Option<Vec<u8>> {
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        let endpoint = usize::from(endpoint);
        let control = EspUsbOtgRegister::DiepCtl(endpoint as u8);
        let size = EspUsbOtgRegister::DiepTsiz(endpoint as u8);
        if state.register(control) & DWC2_EPENA == 0
            || state.register(size) & 0x7ffff != 0
            || state.tx_fifo[endpoint].len() < state.in_transfer_size[endpoint]
        {
            return None;
        }
        let length = state.in_transfer_size[endpoint];
        let packet = state.tx_fifo[endpoint].drain(..length).collect();
        *state.register_mut(control) &= !DWC2_EPENA;
        *state.register_mut(EspUsbOtgRegister::DiepInt(endpoint as u8)) |= 1;
        Some(packet)
    }

    /// Returns whether an OUT endpoint is armed to receive host data.
    pub fn output_ready(&self, endpoint: u8) -> bool {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        state.register(EspUsbOtgRegister::DoepCtl(endpoint)) & DWC2_EPENA != 0
    }

    /// Returns the number of bytes currently scheduled on an OUT endpoint.
    pub fn output_capacity(&self, endpoint: u8) -> usize {
        let state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        usize::try_from(state.register(EspUsbOtgRegister::DoepTsiz(endpoint)) & 0x7ffff)
            .expect("DWC2 transfer size fits usize")
    }

    /// Delivers one host-to-device packet through the shared receive FIFO.
    pub fn inject_output(&self, endpoint: u8, bytes: &[u8]) {
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        for chunk in bytes.chunks(4) {
            let mut word = [0_u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            state.rx_fifo.push_back(u32::from_le_bytes(word));
        }
        let size = EspUsbOtgRegister::DoepTsiz(endpoint);
        let current = state.register(size);
        let remaining = current & 0x7ffff;
        let updated = (current & !0x7ffff) | remaining.saturating_sub(bytes.len() as u32);
        *state.register_mut(size) = updated;
        state
            .rx_status
            .push_back(u32::from(endpoint) | ((bytes.len() as u32) << 4) | (2 << 17));
        state.rx_status.push_back(u32::from(endpoint) | (3 << 17));
    }
}

impl Device for EspUsbOtg {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP USB OTG core requires aligned word access",
            ));
        }
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        let value = match offset {
            offset if offset == EspUsbOtgRegister::GintSts.offset() => state.interrupt_status(),
            offset if offset == EspUsbOtgRegister::GrxStsR.offset() => {
                state.rx_status.front().copied().unwrap_or(0)
            }
            offset if offset == EspUsbOtgRegister::GrxStsP.offset() => state.pop_rx_status(),
            offset if offset == EspUsbOtgRegister::Daint.offset() => state.endpoint_interrupts(),
            offset if (0x1000..0x1_0000).contains(&offset) => {
                state.rx_fifo.pop_front().unwrap_or(0)
            }
            offset
                if matches!(
                    EspUsbOtgRegister::from_offset(offset),
                    Some(EspUsbOtgRegister::DiepInt(_))
                ) =>
            {
                let register = EspUsbOtgRegister::from_offset(offset).expect("DIEPINT offset");
                let EspUsbOtgRegister::DiepInt(endpoint) = register else {
                    unreachable!();
                };
                let mut value = state.register(register);
                if state.register(EspUsbOtgRegister::DiepEmpMsk) & (1 << endpoint) != 0
                    && state.register(EspUsbOtgRegister::DiepCtl(endpoint)) & DWC2_EPENA != 0
                {
                    value |= 1 << 7;
                }
                value
            }
            _ => state
                .registers
                .get(usize::try_from(offset / 4).expect("USB OTG offset fits"))
                .copied()
                .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))?,
        };
        if std::env::var_os("REMU_DEBUG_USB").is_some()
            && (offset == EspUsbOtgRegister::GintSts.offset()
                || offset == EspUsbOtgRegister::Daint.offset()
                || matches!(
                    EspUsbOtgRegister::from_offset(offset),
                    Some(
                        EspUsbOtgRegister::DiepInt(_)
                            | EspUsbOtgRegister::DoepInt(_)
                            | EspUsbOtgRegister::DiepCtl(_)
                            | EspUsbOtgRegister::DoepCtl(_)
                    )
                ))
        {
            eprintln!("dwc2 reg read {offset:#x} -> {value:#x}");
        }
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP USB OTG core requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("USB OTG offset fits");
        let mut state = self.state.lock().expect("ESP USB OTG state lock poisoned");
        if std::env::var_os("REMU_DEBUG_USB").is_some()
            && (offset == EspUsbOtgRegister::GintSts.offset()
                || offset == EspUsbOtgRegister::Daint.offset()
                || matches!(
                    EspUsbOtgRegister::from_offset(offset),
                    Some(
                        EspUsbOtgRegister::DiepInt(_)
                            | EspUsbOtgRegister::DoepInt(_)
                            | EspUsbOtgRegister::DiepCtl(_)
                            | EspUsbOtgRegister::DoepCtl(_)
                    )
                ))
        {
            eprintln!("dwc2 reg write {offset:#x} <- {value:#x}");
        }
        if (0x1000..0x1_0000).contains(&offset) {
            let endpoint =
                usize::try_from((offset - 0x1000) / 0x1000).expect("endpoint number fits usize");
            state.write_fifo(endpoint, value as u32);
            return Ok(());
        }
        if index >= state.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        if offset == EspUsbOtgRegister::GotgInt.offset()
            || offset == EspUsbOtgRegister::GintSts.offset()
        {
            // GOTGINT and writable GINTSTS causes are write-one-to-clear.
            state.registers[index] &= !(value as u32);
        } else if offset == EspUsbOtgRegister::GrstCtl.offset() {
            // CSRST and the FIFO flush strobes self-clear once the functional
            // operation has completed. AHB remains idle for the next access.
            state.registers[index] = value as u32 & !((1 << 0) | (1 << 4) | (1 << 5));
            state.registers[index] |= 1 << 31;
            if value & (1 << 4) != 0 {
                state.rx_status.clear();
                state.rx_fifo.clear();
            }
            if value & (1 << 5) != 0 {
                for fifo in &mut state.tx_fifo {
                    fifo.clear();
                }
            }
        } else if offset == EspUsbOtgRegister::Dctl.offset() {
            state.registers[index] = value as u32;
            if value & (1 << 7) != 0 || value & (1 << 9) != 0 {
                // Global NAK effective is observable synchronously.
                *state.register_mut(EspUsbOtgRegister::GintSts) |= 1 << 7;
            }
            if value & (1 << 8) != 0 || value & (1 << 10) != 0 {
                *state.register_mut(EspUsbOtgRegister::GintSts) &= !(1 << 7);
            }
        } else if matches!(
            EspUsbOtgRegister::from_offset(offset),
            Some(EspUsbOtgRegister::DiepInt(_) | EspUsbOtgRegister::DoepInt(_))
        ) {
            // Endpoint interrupt registers are write-one-to-clear.
            state.registers[index] &= !(value as u32);
        } else if let Some(EspUsbOtgRegister::DiepCtl(endpoint)) =
            EspUsbOtgRegister::from_offset(offset)
        {
            let endpoint = usize::from(endpoint);
            state.registers[index] = value as u32;
            if value as u32 & DWC2_EPDIS != 0 {
                state.registers[index] &= !DWC2_EPENA;
                *state.register_mut(EspUsbOtgRegister::DiepInt(endpoint as u8)) |= 1 << 1;
            }
            if value as u32 & DWC2_EPENA != 0 {
                let size = usize::try_from(
                    state.register(EspUsbOtgRegister::DiepTsiz(endpoint as u8)) & 0x7ffff,
                )
                .expect("DWC2 transfer size fits usize");
                state.in_transfer_size[endpoint] = size;
                state.tx_fifo[endpoint].clear();
            }
        } else if let Some(EspUsbOtgRegister::DoepCtl(endpoint)) =
            EspUsbOtgRegister::from_offset(offset)
        {
            let endpoint = usize::from(endpoint);
            state.registers[index] = value as u32;
            if value as u32 & DWC2_EPDIS != 0 {
                state.registers[index] &= !DWC2_EPENA;
                *state.register_mut(EspUsbOtgRegister::DoepInt(endpoint as u8)) |= 1 << 1;
            }
        } else {
            state.registers[index] = value as u32;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("ESP USB OTG state lock poisoned") = EspUsbOtgState::reset();
    }
}

/// ESP32-C6 analog-register I²C master and its internal byte registers.
///
/// ESP-IDF accesses calibration and regulator state by writing packed
/// slave/address/data commands to the two master control words. Commands
/// complete synchronously in the functional model.
pub struct EspAnalogI2c {
    name: String,
    registers: Vec<u32>,
    analog: BTreeMap<(u8, u8), u8>,
}

impl EspAnalogI2c {
    /// Creates a reset analog I²C master.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: vec![0; 0x1000 / 4],
            analog: BTreeMap::new(),
        }
    }
}

impl Device for EspAnalogI2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP analog I2C requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("analog-I2C offset fits");
        self.registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP analog I2C requires aligned word access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("analog-I2C offset fits");
        let command = value as u32;
        if index >= self.registers.len() {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        self.registers[index] = command;

        if matches!(offset, 0x804 | 0x808) {
            let slave = command as u8;
            let address = (command >> 8) as u8;
            if command & (1 << 24) != 0 {
                let data = (command >> 16) as u8;
                self.analog.insert((slave, address), data);
                // A completed BBPLL configuration makes the hardware
                // calibration-done status visible in I2C_MST_ANA_CONF0.
                // Functional time completes the calibration synchronously.
                if slave == 0x66 {
                    self.registers[0x818 / 4] |= 1 << 24;
                }
                // Releasing the ULP analog reset completes the deterministic
                // O-code and band-gap calibration.
                if slave == 0x61 && address == 0 && data & 1 != 0 {
                    self.analog
                        .entry((0x61, 3))
                        .and_modify(|value| *value |= 0x09)
                        .or_insert(0x09);
                }
            } else {
                let data = self.analog.get(&(slave, address)).copied().unwrap_or(0);
                self.registers[index] = (command & !(0xff << 16)) | (u32::from(data) << 16);
            }
            self.registers[index] &= !(1 << 25);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
        self.analog.clear();
    }
}

/// Functional ESP SPI-memory controller command window.
///
/// User commands complete synchronously. The facade currently exposes the
/// identification/status responses needed to discover a conventional 4 MiB
/// JEDEC flash; memory-mapped application bytes remain owned by the machine's
/// flash mapping.
pub struct EspSpiMem {
    name: String,
    registers: Vec<u32>,
    write_enabled: bool,
}

impl EspSpiMem {
    /// Creates a reset SPI-memory controller.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            registers: vec![0; 0x1000 / 4],
            write_enabled: false,
        }
    }

    fn execute_user_command(&mut self) {
        let command = self.registers[0x20 / 4] as u8;
        let response = match command {
            // RDID: GigaDevice GD25Q32-compatible 4 MiB part. ESP's ROM
            // helper consumes the bytes in this little-endian word order.
            0x9f => 0x0016_40c8,
            // RDSR / RDSR2. Flash is idle; preserve WEL while applicable.
            0x05 => u32::from(self.write_enabled) << 1,
            0x35 => 0,
            // RDSFDP returns an unavailable signature for now, causing IDF
            // to use its JEDEC-ID fallback table deterministically.
            0x5a => 0,
            0x06 => {
                self.write_enabled = true;
                0
            }
            0x04 => {
                self.write_enabled = false;
                0
            }
            _ => 0,
        };
        self.registers[0x58 / 4] = response;
        self.registers[0] &= !(1 << 18);
    }
}

impl Device for EspSpiMem {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width == AccessWidth::DoubleWord || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "ESP SPI memory controller requires naturally aligned access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("SPI-memory offset fits");
        self.registers
            .get(index)
            .copied()
            .map(|value| {
                let shift = (offset & 3) * 8;
                let mask = match width {
                    AccessWidth::Byte => 0xff,
                    AccessWidth::HalfWord => 0xffff,
                    AccessWidth::Word => u64::from(u32::MAX),
                    AccessWidth::DoubleWord => unreachable!("double-word access rejected"),
                };
                (u64::from(value) >> shift) & mask
            })
            .ok_or_else(|| DeviceError::new(format!("{} read at {offset:#x}", self.name)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width == AccessWidth::DoubleWord || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "ESP SPI memory controller requires naturally aligned access",
            ));
        }
        let index = usize::try_from(offset / 4).expect("SPI-memory offset fits");
        let register = self
            .registers
            .get_mut(index)
            .ok_or_else(|| DeviceError::new(format!("{} write at {offset:#x}", self.name)))?;
        let shift = ((offset & 3) * 8) as u32;
        let mask = match width {
            AccessWidth::Byte => 0xff_u32,
            AccessWidth::HalfWord => 0xffff,
            AccessWidth::Word => u32::MAX,
            AccessWidth::DoubleWord => unreachable!("double-word access rejected"),
        } << shift;
        *register = (*register & !mask) | (((value as u32) << shift) & mask);
        if offset & !3 == 0 {
            let command = *register;
            if command & (1 << 30) != 0 {
                self.write_enabled = true;
            }
            if command & (1 << 29) != 0 {
                self.write_enabled = false;
            }
            if command & (1 << 28) != 0 {
                self.registers[0x58 / 4] = 0x0016_40c8;
            }
            if command & (1 << 27) != 0 {
                self.registers[0x58 / 4] = u32::from(self.write_enabled) << 1;
            }
            if command & (1 << 18) != 0 {
                self.execute_user_command();
            }
            // Every operation trigger in CMD[31:17] is self-clearing after
            // the synchronous functional transaction completes.
            self.registers[0] &= 0x0001_ffff;
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.fill(0);
        self.write_enabled = false;
    }
}
