use super::*;

/// Functional RP2040 reset controller, including the peripheral atomic-register aliases.
pub struct Rp2040Resets {
    name: String,
    reset: u32,
    watchdog_select: u32,
}

impl Rp2040Resets {
    pub(crate) const VALID_MASK: u32 = 0x01ff_ffff;

    /// Creates a reset controller in the boot-ROM handoff state.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            reset: 0,
            watchdog_select: 0,
        }
    }

    fn register_value(&self, register: u64) -> Result<u32, DeviceError> {
        match register {
            0x00 => Ok(self.reset),
            0x04 => Ok(self.watchdog_select),
            0x08 => Ok(!self.reset & Self::VALID_MASK),
            _ => Err(DeviceError::new(format!(
                "unmodeled RP2040 RESETS read at offset {register:#x}"
            ))),
        }
    }

    pub(crate) fn update(register: &mut u32, alias: u64, value: u32) -> Result<(), DeviceError> {
        match alias {
            0 => *register = value,
            1 => *register ^= value,
            2 => *register |= value,
            3 => *register &= !value,
            _ => return Err(DeviceError::new("invalid RP2040 atomic alias")),
        }
        Ok(())
    }
}

impl Device for Rp2040Resets {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 RESETS requires word access"));
        }
        Ok(u64::from(self.register_value(offset & 0x0fff)?))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 RESETS requires word access"));
        }
        let alias = (offset >> 12) & 3;
        let register = offset & 0x0fff;
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked RP2040 register value fits");
        match register {
            0x00 => {
                Self::update(&mut self.reset, alias, value)?;
                self.reset &= Self::VALID_MASK;
            }
            0x04 => {
                Self::update(&mut self.watchdog_select, alias, value)?;
                self.watchdog_select &= Self::VALID_MASK;
            }
            0x08 => {
                return Err(DeviceError::new(
                    "RP2040 RESET_DONE is a read-only register",
                ));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 RESETS write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.reset = 0;
        self.watchdog_select = 0;
    }
}

/// Functional RP2040 clock controller with immediate source selection.
pub struct Rp2040Clocks {
    name: String,
    registers: [u32; 50],
}

impl Rp2040Clocks {
    /// Creates the reset-state clock register bank.
    pub fn new(name: impl Into<String>) -> Self {
        let mut registers = [0; 50];
        for offset in [0x04_usize, 0x10, 0x1c, 0x28, 0x34, 0x40, 0x58, 0x64, 0x70] {
            registers[offset / 4] = 0x100;
        }
        for offset in [
            0x08_usize, 0x14, 0x20, 0x2c, 0x38, 0x44, 0x50, 0x5c, 0x68, 0x74,
        ] {
            registers[offset / 4] = 1;
        }
        Self {
            name: name.into(),
            registers,
        }
    }

    fn update(register: &mut u32, alias: u64, value: u32) -> Result<(), DeviceError> {
        match alias {
            0 => *register = value,
            1 => *register ^= value,
            2 => *register |= value,
            3 => *register &= !value,
            _ => return Err(DeviceError::new("invalid RP2040 CLOCKS atomic alias")),
        }
        Ok(())
    }
}

impl Device for Rp2040Clocks {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 CLOCKS requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        // Source switching is functional and instantaneous. The selected registers remain
        // one-hot because the SDK also uses exact equality while moving through the glitchless
        // reference and system-clock muxes.
        if register_offset == 0x38 {
            return Ok(u64::from(1_u32 << (self.registers[0x30 / 4] & 3)));
        }
        if register_offset == 0x44 {
            return Ok(u64::from(1_u32 << (self.registers[0x3c / 4] & 1)));
        }
        if matches!(
            register_offset,
            0x08 | 0x14 | 0x20 | 0x2c | 0x50 | 0x5c | 0x68 | 0x74
        ) {
            return Ok(1);
        }
        if register_offset == 0xb0 || register_offset == 0xb4 {
            return Ok(u64::from(u32::MAX));
        }
        let index = usize::try_from(register_offset / 4).expect("small clock offset fits");
        self.registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| {
                DeviceError::new(format!(
                    "unmodeled RP2040 CLOCKS read at offset {register_offset:#x}"
                ))
            })
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
                "RP2040 CLOCKS requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small clock offset fits");
        let register = self.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2040 CLOCKS write at offset {register_offset:#x}"
            ))
        })?;
        let value =
            u32::try_from(value & u64::from(u32::MAX)).expect("masked clock register value fits");
        Self::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

/// Functional RP2040 crystal oscillator with immediate stabilization.
pub struct Rp2040Xosc {
    name: String,
    control: u32,
    startup: u32,
    count: u32,
}

impl Rp2040Xosc {
    /// Creates a reset-state crystal oscillator.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            control: 0,
            startup: 0,
            count: 0,
        }
    }
}

impl Device for Rp2040Xosc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 XOSC requires word access"));
        }
        let value = match offset & 0x0fff {
            0x00 => self.control,
            0x04 => 0x8000_1000 | (self.control & 3),
            0x08 => 0,
            0x0c => self.startup,
            0x1c => self.count,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 XOSC read at offset {register:#x}"
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 XOSC requires word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked XOSC value fits");
        match offset & 0x0fff {
            0x00 => self.control = value & 0x00ff_ffff,
            0x04 => {}
            0x08 => {}
            0x0c => self.startup = value & 0x0010_3fff,
            0x1c => self.count = value & 0xff,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 XOSC write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.control = 0;
        self.startup = 0;
        self.count = 0;
    }
}

/// Functional RP2040 PLL with immediate lock acquisition.
pub struct Rp2040Pll {
    name: String,
    control_status: u32,
    power: u32,
    feedback_divider: u32,
    primitive: u32,
}

impl Rp2040Pll {
    /// Creates a reset-state PLL.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            control_status: 1,
            power: 0x2d,
            feedback_divider: 0,
            primitive: 0x0007_7000,
        }
    }
}

impl Device for Rp2040Pll {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 PLL requires word access"));
        }
        let value = match offset & 0x0fff {
            0x00 => self.control_status | 0x8000_0000,
            0x04 => self.power,
            0x08 => self.feedback_divider,
            0x0c => self.primitive,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 PLL read at offset {register:#x}"
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
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 PLL requires word access"));
        }
        let alias = (offset >> 12) & 3;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked PLL value fits");
        let register = match offset & 0x0fff {
            0x00 => &mut self.control_status,
            0x04 => &mut self.power,
            0x08 => &mut self.feedback_divider,
            0x0c => &mut self.primitive,
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 PLL write at offset {register:#x}"
                )));
            }
        };
        Rp2040Clocks::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

/// RP2040 WATCHDOG register identifiers.
///
/// Keeping the offsets named prevents the device model and its tests from
/// silently drifting when a register is added or moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Rp2040WatchdogRegister {
    /// Watchdog control and remaining time.
    Ctrl = 0x00,
    /// Reload value for the watchdog counter.
    Load = 0x04,
    /// Reset reason bits.
    Reason = 0x08,
    /// Persistent scratch register zero.
    Scratch0 = 0x0c,
    /// Persistent scratch register one.
    Scratch1 = 0x10,
    /// Persistent scratch register two.
    Scratch2 = 0x14,
    /// Persistent scratch register three.
    Scratch3 = 0x18,
    /// Persistent scratch register four.
    Scratch4 = 0x1c,
    /// Persistent scratch register five.
    Scratch5 = 0x20,
    /// Persistent scratch register six.
    Scratch6 = 0x24,
    /// Persistent scratch register seven.
    Scratch7 = 0x28,
    /// Watchdog tick-generator configuration and status.
    Tick = 0x2c,
}

impl TryFrom<u64> for Rp2040WatchdogRegister {
    type Error = ();

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x00 => Ok(Self::Ctrl),
            0x04 => Ok(Self::Load),
            0x08 => Ok(Self::Reason),
            0x0c => Ok(Self::Scratch0),
            0x10 => Ok(Self::Scratch1),
            0x14 => Ok(Self::Scratch2),
            0x18 => Ok(Self::Scratch3),
            0x1c => Ok(Self::Scratch4),
            0x20 => Ok(Self::Scratch5),
            0x24 => Ok(Self::Scratch6),
            0x28 => Ok(Self::Scratch7),
            0x2c => Ok(Self::Tick),
            _ => Err(()),
        }
    }
}

const WATCHDOG_CTRL_TIME_MASK: u32 = 0x00ff_ffff;
const WATCHDOG_CTRL_PAUSE_MASK: u32 = 0x0700_0000;
const WATCHDOG_CTRL_ENABLE: u32 = 1 << 30;
const WATCHDOG_CTRL_TRIGGER: u32 = 1 << 31;
const WATCHDOG_TICK_CYCLES_MASK: u32 = 0x0000_01ff;
const WATCHDOG_TICK_ENABLE: u32 = 1 << 9;
const WATCHDOG_TICK_RUNNING: u32 = 1 << 10;
const WATCHDOG_TICK_COUNT_MASK: u32 = 0x000f_f800;
const WATCHDOG_REASON_TIMER: u32 = 1;
const WATCHDOG_REASON_FORCE: u32 = 1 << 1;

#[derive(Clone)]
struct Rp2040WatchdogState {
    ctrl: u32,
    load: u32,
    reason: u32,
    scratch: [u32; 8],
    tick: u32,
    tick_countdown: u64,
    remaining_counter: u64,
    last_time: SimTime,
    reset_pending: bool,
}

impl Rp2040WatchdogState {
    fn reset_state() -> Self {
        let tick = WATCHDOG_TICK_ENABLE;
        Self {
            ctrl: 0x0700_0000,
            load: 0,
            reason: 0,
            scratch: [0; 8],
            tick,
            tick_countdown: Self::divider(tick),
            remaining_counter: 0,
            last_time: SimTime::ZERO,
            reset_pending: false,
        }
    }

    fn divider(tick: u32) -> u64 {
        // CYCLES is encoded as the number of extra clk_tick cycles.  A zero
        // setting therefore still produces one tick per abstract simulation
        // tick, which keeps the default reset state live and deterministic.
        u64::from(tick & WATCHDOG_TICK_CYCLES_MASK) + 1
    }

    fn advance(&mut self, now: SimTime) {
        let elapsed = now.ticks().saturating_sub(self.last_time.ticks());
        self.last_time = now;
        if elapsed == 0 || self.tick & WATCHDOG_TICK_ENABLE == 0 {
            return;
        }

        let divider = Self::divider(self.tick);
        let tick_countdown = self.tick_countdown.max(1);
        let generated = if elapsed < tick_countdown {
            self.tick_countdown = tick_countdown - elapsed;
            0
        } else {
            let after_first = elapsed - tick_countdown;
            let generated = 1 + after_first / divider;
            self.tick_countdown = divider - after_first % divider;
            generated
        };
        if self.ctrl & WATCHDOG_CTRL_ENABLE == 0 || self.remaining_counter == 0 {
            return;
        }

        // RP2040-E1: the hardware counter is decremented twice per generated
        // watchdog tick, so LOAD=2 represents one abstract watchdog tick.
        let decrement = generated.saturating_mul(2);
        self.remaining_counter = self.remaining_counter.saturating_sub(decrement);
        if self.remaining_counter == 0 {
            self.reason |= WATCHDOG_REASON_TIMER;
            self.reset_pending = true;
        }
    }

    fn ctrl_value(&self) -> u32 {
        let time = self
            .remaining_counter
            .div_ceil(2)
            .min(u64::from(WATCHDOG_CTRL_TIME_MASK));
        (self.ctrl & (WATCHDOG_CTRL_PAUSE_MASK | WATCHDOG_CTRL_ENABLE))
            | u32::try_from(time).expect("watchdog time is masked to 24 bits")
    }

    fn tick_value(&self) -> u32 {
        let running = self.tick & WATCHDOG_TICK_ENABLE != 0;
        let count =
            u32::try_from(self.tick_countdown.min(0x1ff)).expect("watchdog tick count fits");
        (self.tick & (WATCHDOG_TICK_CYCLES_MASK | WATCHDOG_TICK_ENABLE))
            | if running { WATCHDOG_TICK_RUNNING } else { 0 }
            | ((count << 11) & WATCHDOG_TICK_COUNT_MASK)
    }

    fn apply_alias(register: &mut u32, alias: u64, value: u32) -> Result<(), DeviceError> {
        Rp2040Clocks::update(register, alias, value)
    }

    fn load_counter(&mut self) {
        self.remaining_counter = u64::from(self.load);
        self.reset_pending = false;
    }

    fn reset(&mut self, kind: ResetKind) {
        let scratch = match kind {
            ResetKind::Software | ResetKind::Watchdog => self.scratch,
            ResetKind::PowerOn | ResetKind::External => [0; 8],
        };
        let reason = (kind == ResetKind::Watchdog)
            .then_some(self.reason)
            .unwrap_or(0);
        *self = Self::reset_state();
        self.scratch = scratch;
        self.reason = reason;
    }
}

/// Shareable RP2040 watchdog reset/tick view used by the machine scheduler.
#[derive(Clone)]
pub struct Rp2040WatchdogHandle {
    state: Arc<Mutex<Rp2040WatchdogState>>,
}

impl Rp2040WatchdogHandle {
    /// Advances the watchdog and consumes one pending reset request.
    pub fn take_reset(&self, now: SimTime) -> bool {
        let mut state = self.state.lock().expect("RP2040 watchdog lock poisoned");
        state.advance(now);
        std::mem::take(&mut state.reset_pending)
    }

    /// Returns the reset reason bits latched by the previous trigger.
    pub fn reason(&self, now: SimTime) -> u32 {
        let mut state = self.state.lock().expect("RP2040 watchdog lock poisoned");
        state.advance(now);
        state.reason
    }
}

/// Functional RP2040 watchdog and microsecond-tick divider.
pub struct Rp2040Watchdog {
    name: String,
    state: Arc<Mutex<Rp2040WatchdogState>>,
}

impl Rp2040Watchdog {
    /// Creates the watchdog reset state without exposing a scheduler handle.
    pub fn new(name: impl Into<String>) -> Self {
        let (device, _) = Self::new_with_handle(name);
        device
    }

    /// Creates the watchdog and a handle that reports functional reset requests.
    pub fn new_with_handle(name: impl Into<String>) -> (Self, Rp2040WatchdogHandle) {
        let state = Arc::new(Mutex::new(Rp2040WatchdogState::reset_state()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Rp2040WatchdogHandle { state },
        )
    }

    fn register(offset: u64) -> Result<Rp2040WatchdogRegister, DeviceError> {
        Rp2040WatchdogRegister::try_from(offset).map_err(|()| {
            DeviceError::new(format!(
                "unmodeled RP2040 WATCHDOG register at offset {offset:#x}"
            ))
        })
    }
}

impl Device for Rp2040Watchdog {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 WATCHDOG requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        let register = Self::register(register_offset)?;
        let mut state = self.state.lock().expect("RP2040 watchdog lock poisoned");
        state.advance(at);
        let value = match register {
            Rp2040WatchdogRegister::Ctrl => state.ctrl_value(),
            Rp2040WatchdogRegister::Load => state.load,
            Rp2040WatchdogRegister::Reason => state.reason,
            Rp2040WatchdogRegister::Scratch0
            | Rp2040WatchdogRegister::Scratch1
            | Rp2040WatchdogRegister::Scratch2
            | Rp2040WatchdogRegister::Scratch3
            | Rp2040WatchdogRegister::Scratch4
            | Rp2040WatchdogRegister::Scratch5
            | Rp2040WatchdogRegister::Scratch6
            | Rp2040WatchdogRegister::Scratch7 => {
                let index = usize::try_from((register_offset - 0x0c) / 4)
                    .expect("watchdog scratch index fits");
                state.scratch[index]
            }
            Rp2040WatchdogRegister::Tick => state.tick_value(),
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
                "RP2040 WATCHDOG requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let register = Self::register(register_offset)?;
        let mut state = self.state.lock().expect("RP2040 watchdog lock poisoned");
        state.advance(at);
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked watchdog value fits");
        match register {
            Rp2040WatchdogRegister::Ctrl => {
                let mut config = state.ctrl & (WATCHDOG_CTRL_PAUSE_MASK | WATCHDOG_CTRL_ENABLE);
                Rp2040WatchdogState::apply_alias(
                    &mut config,
                    alias,
                    value & (WATCHDOG_CTRL_PAUSE_MASK | WATCHDOG_CTRL_ENABLE),
                )?;
                state.ctrl = config;
                if value & WATCHDOG_CTRL_TRIGGER != 0 {
                    state.reason |= WATCHDOG_REASON_FORCE;
                    state.reset_pending = true;
                }
            }
            Rp2040WatchdogRegister::Load => {
                let mut load = state.load;
                Rp2040WatchdogState::apply_alias(
                    &mut load,
                    alias,
                    value & WATCHDOG_CTRL_TIME_MASK,
                )?;
                state.load = load & WATCHDOG_CTRL_TIME_MASK;
                state.load_counter();
            }
            Rp2040WatchdogRegister::Reason => {
                return Err(DeviceError::new("RP2040 WATCHDOG REASON is read-only"));
            }
            Rp2040WatchdogRegister::Scratch0
            | Rp2040WatchdogRegister::Scratch1
            | Rp2040WatchdogRegister::Scratch2
            | Rp2040WatchdogRegister::Scratch3
            | Rp2040WatchdogRegister::Scratch4
            | Rp2040WatchdogRegister::Scratch5
            | Rp2040WatchdogRegister::Scratch6
            | Rp2040WatchdogRegister::Scratch7 => {
                let index = usize::try_from((register_offset - 0x0c) / 4)
                    .expect("watchdog scratch index fits");
                Rp2040WatchdogState::apply_alias(&mut state.scratch[index], alias, value)?;
            }
            Rp2040WatchdogRegister::Tick => {
                let mut tick = state.tick;
                Rp2040WatchdogState::apply_alias(
                    &mut tick,
                    alias,
                    value & (WATCHDOG_TICK_CYCLES_MASK | WATCHDOG_TICK_ENABLE),
                )?;
                state.tick = tick & (WATCHDOG_TICK_CYCLES_MASK | WATCHDOG_TICK_ENABLE);
                state.tick_countdown = Rp2040WatchdogState::divider(state.tick);
            }
        }
        Ok(())
    }

    fn reset(&mut self, kind: ResetKind) {
        self.state
            .lock()
            .expect("RP2040 watchdog lock poisoned")
            .reset(kind);
    }
}

/// Shared RP2040 timer interrupt view.
#[derive(Clone)]
pub struct Rp2040TimerHandle {
    state: Rc<RefCell<Rp2040TimerState>>,
}

impl Rp2040TimerHandle {
    /// Returns the four masked alarm interrupt bits at `now`.
    pub fn pending(&self, now: SimTime) -> u8 {
        let mut state = self.state.borrow_mut();
        let previous = state.raw_interrupt;
        state.update(now);
        let pending = (state.raw_interrupt | state.force_interrupt) & state.interrupt_enable;
        if state.raw_interrupt != previous && std::env::var_os("REMU_DEBUG_TIMERS").is_some() {
            eprintln!(
                "RP timer alarm at={} raw={:#x} enabled={:#x} pending={pending:#x}",
                now.ticks(),
                state.raw_interrupt,
                state.interrupt_enable,
            );
        }
        pending
    }
}

struct Rp2040TimerState {
    alarms: [u32; 4],
    armed: u8,
    raw_interrupt: u8,
    interrupt_enable: u8,
    force_interrupt: u8,
    debug_pause: u32,
    paused: bool,
}

impl Rp2040TimerState {
    fn update(&mut self, now: SimTime) {
        if self.paused {
            return;
        }
        let current = now.ticks() as u32;
        for alarm in 0..4 {
            let mask = 1_u8 << alarm;
            if self.armed & mask != 0 && current.wrapping_sub(self.alarms[alarm]) < 0x8000_0000 {
                self.armed &= !mask;
                self.raw_interrupt |= mask;
            }
        }
    }
}

/// Functional RP2040 64-bit microsecond timer and four alarms.
pub struct Rp2040Timer {
    name: String,
    layout: RpTimerLayout,
    state: Rc<RefCell<Rp2040TimerState>>,
}

/// Register layout implemented by the Raspberry Pi timer block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpTimerLayout {
    /// RP2040 timer layout with interrupt registers beginning at offset `0x34`.
    Rp2040,
    /// RP2350 timer layout with LOCKED/SOURCE and interrupts beginning at `0x3c`.
    Rp2350,
}

impl Rp2040Timer {
    /// Creates the free-running timer and a scheduler-facing handle.
    pub fn new(name: impl Into<String>, layout: RpTimerLayout) -> (Self, Rp2040TimerHandle) {
        let state = Rc::new(RefCell::new(Rp2040TimerState {
            alarms: [0; 4],
            armed: 0,
            raw_interrupt: 0,
            interrupt_enable: 0,
            force_interrupt: 0,
            debug_pause: 7,
            paused: false,
        }));
        let handle = Rp2040TimerHandle {
            state: state.clone(),
        };
        (
            Self {
                name: name.into(),
                layout,
                state,
            },
            handle,
        )
    }
}

impl Device for Rp2040Timer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 TIMER requires word access"));
        }
        let mut state = self.state.borrow_mut();
        state.update(at);
        let ticks = at.ticks();
        let value = match offset & 0x0fff {
            0x00 | 0x08 | 0x24 => (ticks >> 32) as u32,
            0x04 | 0x0c | 0x28 => ticks as u32,
            0x10 | 0x14 | 0x18 | 0x1c => {
                state.alarms
                    [usize::try_from(((offset & 0x0fff) - 0x10) / 4).expect("alarm index fits")]
            }
            0x20 => u32::from(state.armed),
            0x2c => state.debug_pause,
            0x30 => u32::from(state.paused),
            0x34 if self.layout == RpTimerLayout::Rp2040 => u32::from(state.raw_interrupt),
            0x38 if self.layout == RpTimerLayout::Rp2040 => u32::from(state.interrupt_enable),
            0x3c if self.layout == RpTimerLayout::Rp2040 => u32::from(state.force_interrupt),
            0x40 if self.layout == RpTimerLayout::Rp2040 => {
                u32::from((state.raw_interrupt | state.force_interrupt) & state.interrupt_enable)
            }
            // RP2350 inserts read-only LOCKED and SOURCE registers before INTR.
            0x34 | 0x38 if self.layout == RpTimerLayout::Rp2350 => 0,
            0x3c if self.layout == RpTimerLayout::Rp2350 => u32::from(state.raw_interrupt),
            0x40 if self.layout == RpTimerLayout::Rp2350 => u32::from(state.interrupt_enable),
            0x44 if self.layout == RpTimerLayout::Rp2350 => u32::from(state.force_interrupt),
            0x48 if self.layout == RpTimerLayout::Rp2350 => {
                u32::from((state.raw_interrupt | state.force_interrupt) & state.interrupt_enable)
            }
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 TIMER read at offset {register:#x}"
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
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word {
            return Err(DeviceError::new("RP2040 TIMER requires word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked timer value fits");
        let mut state = self.state.borrow_mut();
        state.update(at);
        if std::env::var_os("REMU_DEBUG_TIMERS").is_some() {
            eprintln!(
                "RP timer write at={} offset={offset:#06x} value={value:#010x}",
                at.ticks()
            );
        }
        let register = offset & 0x0fff;
        let alias = (offset >> 12) & 3;
        let update_alias = |current: u8, value: u8| match alias {
            0 => value,
            1 => current ^ value,
            2 => current | value,
            3 => current & !value,
            _ => unreachable!("two-bit alias"),
        };
        match register {
            register @ (0x10 | 0x14 | 0x18 | 0x1c) => {
                let alarm = usize::try_from((register - 0x10) / 4).expect("alarm index fits");
                state.alarms[alarm] = value;
                state.armed |= 1 << alarm;
                state.raw_interrupt &= !(1 << alarm);
                if std::env::var_os("REMU_DEBUG_TIMERS").is_some() {
                    eprintln!(
                        "RP timer arm alarm={alarm} at={} compare={value:#010x}",
                        at.ticks()
                    );
                }
            }
            0x20 => state.armed &= !(value as u8),
            0x2c => state.debug_pause = value & 6,
            0x30 => state.paused = value & 1 != 0,
            0x34 if self.layout == RpTimerLayout::Rp2040 => {
                state.raw_interrupt &= !(value as u8);
            }
            0x38 if self.layout == RpTimerLayout::Rp2040 => {
                state.interrupt_enable = update_alias(state.interrupt_enable, value as u8 & 0xf);
                if std::env::var_os("REMU_DEBUG_TIMERS").is_some() {
                    eprintln!(
                        "RP timer interrupt enable at={} mask={:#x}",
                        at.ticks(),
                        state.interrupt_enable
                    );
                }
            }
            0x3c if self.layout == RpTimerLayout::Rp2040 => {
                state.force_interrupt = update_alias(state.force_interrupt, value as u8 & 0xf);
            }
            // LOCKED is read-only. SOURCE selection is not timing-visible in
            // the functional model because both supported sources advance on
            // the same deterministic simulation timeline.
            0x34 | 0x38 if self.layout == RpTimerLayout::Rp2350 => {}
            0x3c if self.layout == RpTimerLayout::Rp2350 => {
                state.raw_interrupt &= !(value as u8);
            }
            0x40 if self.layout == RpTimerLayout::Rp2350 => {
                state.interrupt_enable = update_alias(state.interrupt_enable, value as u8 & 0xf);
                if std::env::var_os("REMU_DEBUG_TIMERS").is_some() {
                    eprintln!(
                        "RP2350 timer interrupt enable at={} mask={:#x}",
                        at.ticks(),
                        state.interrupt_enable
                    );
                }
            }
            0x44 if self.layout == RpTimerLayout::Rp2350 => {
                state.force_interrupt = update_alias(state.force_interrupt, value as u8 & 0xf);
            }
            register => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2040 TIMER write at offset {register:#x}"
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.borrow_mut();
        state.alarms = [0; 4];
        state.armed = 0;
        state.raw_interrupt = 0;
        state.interrupt_enable = 0;
        state.force_interrupt = 0;
        state.debug_pause = 7;
        state.paused = false;
    }
}

/// Storage-backed RP2040 APB register slice with atomic XOR, SET, and CLEAR aliases.
///
/// This is used for configuration-only blocks whose values affect observability but do not yet
/// schedule independent events, such as pad and GPIO-function selection registers.
pub struct Rp2040RegisterBank {
    name: String,
    reset_values: Vec<u32>,
    registers: Vec<u32>,
}

const RP2040_GPIO_CTRL_BITS: u32 = 0x3003_331f;

/// RP2040/RP2350 IO_BANK0 interrupt and GPIO-function register slice.
///
/// The SIO block remains the owner of GPIO output latches. This device models
/// the APB-side per-pin status/control registers and the processor interrupt
/// enable/force/status windows. The machine-facing handle records external
/// input transitions so edge events become visible to firmware deterministically.
pub struct Rp2040IoBank {
    name: String,
    pins: usize,
    gpio: GpioHandle,
    state: Arc<Mutex<Rp2040IoBankState>>,
}

/// Host-facing input-transition handle for an RP IO_BANK block.
#[derive(Clone)]
pub struct Rp2040IoBankHandle {
    gpio: GpioHandle,
    state: Arc<Mutex<Rp2040IoBankState>>,
}

struct Rp2040IoBankState {
    control: Vec<u32>,
    last_input: u32,
    raw_interrupt: [u32; 4],
    proc0_enable: [u32; 4],
    proc0_force: [u32; 4],
    proc1_enable: [u32; 4],
    proc1_force: [u32; 4],
}

impl Rp2040IoBank {
    /// Creates an IO_BANK0 block sharing the SIO GPIO input net.
    pub fn new(name: impl Into<String>, gpio: GpioHandle) -> (Self, Rp2040IoBankHandle) {
        let pins = gpio.pin_count().min(32);
        let state = Arc::new(Mutex::new(Rp2040IoBankState {
            control: vec![0x1f; pins],
            last_input: 0,
            raw_interrupt: [0; 4],
            proc0_enable: [0; 4],
            proc0_force: [0; 4],
            proc1_enable: [0; 4],
            proc1_force: [0; 4],
        }));
        let handle = Rp2040IoBankHandle {
            gpio: gpio.clone(),
            state: state.clone(),
        };
        (
            Self {
                name: name.into(),
                pins,
                gpio,
                state,
            },
            handle,
        )
    }

    fn event_bank(pin: usize) -> (usize, u32) {
        (pin / 8, 1_u32 << ((pin % 8) * 4))
    }

    fn input_value(&self) -> u32 {
        (0..self.pins).fold(0, |value, pin| {
            if self.gpio.resolved(pin as u8).ok() == Some(Logic::One) {
                value | (1_u32 << pin)
            } else {
                value
            }
        })
    }

    fn override_value(value: bool, mode: u32) -> bool {
        match mode & 3 {
            0 => value,
            1 => !value,
            2 => false,
            3 => true,
            _ => unreachable!(),
        }
    }

    fn gpio_status(&self, pin: usize, state: &Rp2040IoBankState) -> u32 {
        let mask = 1_u32 << pin;
        let input = self.input_value() & mask != 0;
        let output = self.gpio.output() & mask != 0;
        let output_enable = self.gpio.direction() & mask != 0;
        let (bank, event_mask) = Self::event_bank(pin);
        let irq = state.raw_interrupt[bank] & (event_mask * 0xf) != 0;
        let control = state.control[pin];
        u32::from(Self::override_value(irq, control >> 28)) << 26
            | u32::from(irq) << 24
            | u32::from(Self::override_value(input, control >> 16)) << 19
            | u32::from(input) << 17
            | u32::from(Self::override_value(output_enable, control >> 12)) << 15
            | u32::from(output_enable) << 13
            | u32::from(Self::override_value(output, control >> 8)) << 9
            | u32::from(output) << 8
    }

    fn interrupt_bank(offset: u64, base: u64) -> Option<(usize, u64)> {
        if (base..base + 0x10).contains(&offset) {
            Some((usize::try_from((offset - base) / 4).ok()?, 0))
        } else {
            None
        }
    }
}

impl Rp2040IoBankHandle {
    /// Returns whether PROC0's masked or forced interrupt output is asserted.
    pub fn proc0_pending(&self) -> bool {
        let state = self.state.lock().expect("RP IO_BANK lock poisoned");
        (0..state.raw_interrupt.len()).any(|bank| {
            state.raw_interrupt[bank] & state.proc0_enable[bank] != 0
                || state.proc0_force[bank] != 0
        })
    }

    /// Records the current resolved value after an external pin stimulus.
    pub fn record_input(&self, pin: u8) -> Result<(), DeviceError> {
        let value = self.gpio.resolved(pin)?;
        let mut state = self.state.lock().expect("RP IO_BANK lock poisoned");
        let index = usize::from(pin);
        if index >= state.control.len() {
            return Err(DeviceError::new(format!("GPIO pin {pin} is out of range")));
        }
        let mask = 1_u32 << index;
        let was_high = state.last_input & mask != 0;
        let is_high = value == Logic::One;
        if was_high != is_high {
            let (bank, event_mask) = Rp2040IoBank::event_bank(index);
            let edge_mask = if is_high {
                event_mask << 3
            } else {
                event_mask << 2
            };
            state.raw_interrupt[bank] |= edge_mask;
        }
        if is_high {
            state.last_input |= mask;
        } else {
            state.last_input &= !mask;
        }
        Ok(())
    }
}

impl Device for Rp2040IoBank {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RP IO_BANK requires aligned word access"));
        }
        if offset < 0xf0 {
            let pin = usize::try_from(offset / 8).expect("GPIO index fits usize");
            if pin >= self.pins {
                return Err(DeviceError::new(format!(
                    "{} read outside GPIO pins at offset {offset:#x}",
                    self.name
                )));
            }
            let state = self.state.lock().expect("RP IO_BANK lock poisoned");
            return Ok(if offset % 8 == 4 {
                u64::from(state.control[pin])
            } else {
                u64::from(self.gpio_status(pin, &state))
            });
        }
        let values = if let Some((index, _)) = Self::interrupt_bank(offset, 0xf0) {
            let state = self.state.lock().expect("RP IO_BANK lock poisoned");
            state.raw_interrupt[index]
        } else if let Some((index, _)) = Self::interrupt_bank(offset, 0x100) {
            let state = self.state.lock().expect("RP IO_BANK lock poisoned");
            state.proc0_enable[index]
        } else if let Some((index, _)) = Self::interrupt_bank(offset, 0x110) {
            let state = self.state.lock().expect("RP IO_BANK lock poisoned");
            state.proc0_force[index]
        } else if let Some((index, _)) = Self::interrupt_bank(offset, 0x120) {
            let state = self.state.lock().expect("RP IO_BANK lock poisoned");
            (state.raw_interrupt[index] & state.proc0_enable[index]) | state.proc0_force[index]
        } else if let Some((index, _)) = Self::interrupt_bank(offset, 0x130) {
            let state = self.state.lock().expect("RP IO_BANK lock poisoned");
            state.proc1_enable[index]
        } else if let Some((index, _)) = Self::interrupt_bank(offset, 0x140) {
            let state = self.state.lock().expect("RP IO_BANK lock poisoned");
            state.proc1_force[index]
        } else if let Some((index, _)) = Self::interrupt_bank(offset, 0x150) {
            let state = self.state.lock().expect("RP IO_BANK lock poisoned");
            (state.raw_interrupt[index] & state.proc1_enable[index]) | state.proc1_force[index]
        } else {
            return Err(DeviceError::new(format!(
                "{} read outside modeled registers at offset {offset:#x}",
                self.name
            )));
        };
        Ok(u64::from(values))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RP IO_BANK requires aligned word access"));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits");
        if offset < 0xf0 {
            let pin = usize::try_from(offset / 8).expect("GPIO index fits usize");
            if pin >= self.pins || offset % 8 != 4 {
                return Err(DeviceError::new(format!(
                    "{} write outside GPIO control registers at offset {offset:#x}",
                    self.name
                )));
            }
            self.state.lock().expect("RP IO_BANK lock poisoned").control[pin] =
                value & RP2040_GPIO_CTRL_BITS;
            return Ok(());
        }
        let mut state = self.state.lock().expect("RP IO_BANK lock poisoned");
        if let Some((index, _)) = Self::interrupt_bank(offset, 0xf0) {
            state.raw_interrupt[index] &= !value;
        } else if let Some((index, _)) = Self::interrupt_bank(offset, 0x100) {
            state.proc0_enable[index] = value;
        } else if let Some((index, _)) = Self::interrupt_bank(offset, 0x110) {
            state.proc0_force[index] = value;
        } else if let Some((index, _)) = Self::interrupt_bank(offset, 0x130) {
            state.proc1_enable[index] = value;
        } else if let Some((index, _)) = Self::interrupt_bank(offset, 0x140) {
            state.proc1_force[index] = value;
        } else if offset >= 0x120 && offset < 0x160 {
            return Err(DeviceError::new("RP IO_BANK interrupt status is read-only"));
        } else {
            return Err(DeviceError::new(format!(
                "{} write outside modeled registers at offset {offset:#x}",
                self.name
            )));
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("RP IO_BANK lock poisoned");
        state.control.fill(0x1f);
        state.last_input = 0;
        state.raw_interrupt = [0; 4];
        state.proc0_enable = [0; 4];
        state.proc0_force = [0; 4];
        state.proc1_enable = [0; 4];
        state.proc1_force = [0; 4];
    }
}

impl Rp2040RegisterBank {
    /// Creates a word-addressed register slice initialized from `reset_values`.
    pub fn new(name: impl Into<String>, reset_values: Vec<u32>) -> Self {
        Self {
            name: name.into(),
            registers: reset_values.clone(),
            reset_values,
        }
    }
}

impl Device for Rp2040RegisterBank {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "RP2040 register bank requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small register offset fits");
        self.registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| {
                DeviceError::new(format!(
                    "{} read outside modeled registers at offset {register_offset:#x}",
                    self.name
                ))
            })
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
                "RP2040 register bank requires aligned word access",
            ));
        }
        let alias = (offset >> 12) & 3;
        let register_offset = offset & 0x0fff;
        let index = usize::try_from(register_offset / 4).expect("small register offset fits");
        let register = self.registers.get_mut(index).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write outside modeled registers at offset {register_offset:#x}",
                self.name
            ))
        })?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked register value fits");
        Rp2040Clocks::update(register, alias, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers.clone_from(&self.reset_values);
    }
}

/// Deterministic word-oriented hardware random-number register block.
///
/// Functional MCU models use this at the vendor RNG data-register address.
/// It deliberately provides reproducible pseudo-random words: firmware sees
/// changing entropy-like input while repeat traces remain byte-for-byte
/// stable.
pub struct DeterministicRng {
    name: String,
    data_offset: u64,
    seed: u32,
    state: u32,
}

impl DeterministicRng {
    /// Creates a deterministic RNG block with one readable data register.
    pub fn new(name: impl Into<String>, data_offset: u64, seed: u32) -> Self {
        let seed = if seed == 0 { 0x6d2b_79f5 } else { seed };
        Self {
            name: name.into(),
            data_offset,
            seed,
            state: seed,
        }
    }

    fn next(&mut self) -> u32 {
        let mut value = self.state;
        value ^= value << 13;
        value ^= value >> 17;
        value ^= value << 5;
        self.state = value;
        value
    }
}

impl Device for DeterministicRng {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "deterministic RNG requires aligned word access",
            ));
        }
        Ok(if offset == self.data_offset {
            u64::from(self.next())
        } else {
            0
        })
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        _value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "deterministic RNG requires aligned word access",
            ));
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state = self.seed;
    }
}
