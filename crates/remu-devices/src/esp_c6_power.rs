use super::*;

struct EspC6PmuState {
    registers: Vec<u32>,
    hp_sleep: bool,
    lp_sleep: bool,
    hp_trigger_lp: bool,
    lp_trigger_hp: bool,
}

impl EspC6PmuState {
    fn new() -> Self {
        let mut registers = vec![0; 0x400 / 4];
        registers[0x04 / 4] = u32::MAX;
        registers[0x08 / 4] = u32::MAX;
        registers[0x17c / 4] = (1 << 20) | (0xff << 21);
        registers[0x3fc / 4] = 35_676_752;
        Self {
            registers,
            hp_sleep: false,
            lp_sleep: false,
            hp_trigger_lp: false,
            lp_trigger_hp: false,
        }
    }
}

/// Scheduler-facing ESP32-C6 power-management transitions.
#[derive(Clone)]
pub struct EspC6PmuHandle {
    state: Rc<RefCell<EspC6PmuState>>,
}

impl EspC6PmuHandle {
    /// Consumes an HP sleep request.
    pub fn take_hp_sleep(&self) -> bool {
        std::mem::take(&mut self.state.borrow_mut().hp_sleep)
    }

    /// Consumes an HP-to-LP wake trigger.
    pub fn take_hp_trigger_lp(&self) -> bool {
        std::mem::take(&mut self.state.borrow_mut().hp_trigger_lp)
    }

    /// Consumes an LP sleep request.
    pub fn take_lp_sleep(&self) -> bool {
        std::mem::take(&mut self.state.borrow_mut().lp_sleep)
    }

    /// Consumes an LP-to-HP wake trigger.
    pub fn take_lp_trigger_hp(&self) -> bool {
        std::mem::take(&mut self.state.borrow_mut().lp_trigger_hp)
    }

    /// Reports whether the LP CPU is force-stalled.
    pub fn lp_force_stalled(&self) -> bool {
        self.state.borrow().registers[0x17c / 4] & (1 << 18) != 0
    }

    /// Reports whether an LP sleep transition requests a core reset.
    pub fn lp_reset_on_sleep(&self) -> bool {
        self.state.borrow().registers[0x17c / 4] & (1 << 30) != 0
    }

    /// Returns the enabled LP-core wake-source mask.
    pub fn lp_wakeup_mask(&self) -> u16 {
        self.state.borrow().registers[0x180 / 4] as u16
    }

    /// Records a completed wake transition and its source bit.
    pub fn record_hp_wakeup(&self, source: u32) {
        let mut state = self.state.borrow_mut();
        state.registers[0x140 / 4] = source;
        state.registers[0x15c / 4] |= 1 << 31;
    }
}

/// Functional ESP32-C6 PMU transition and retention register page.
pub struct EspC6Pmu {
    name: String,
    state: Rc<RefCell<EspC6PmuState>>,
}

impl EspC6Pmu {
    /// Creates a PMU and its machine scheduler handle.
    pub fn new(name: impl Into<String>) -> (Self, EspC6PmuHandle) {
        let state = Rc::new(RefCell::new(EspC6PmuState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspC6PmuHandle { state },
        )
    }

    fn index(offset: u64, width: AccessWidth) -> Result<usize, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) || offset >= 0x400 {
            return Err(DeviceError::new(
                "ESP32-C6 PMU requires an aligned word access",
            ));
        }
        Ok(offset as usize / 4)
    }
}

impl Device for EspC6Pmu {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let index = Self::index(offset, width)?;
        let state = self.state.borrow();
        let value = match offset {
            0x160 => state.registers[0x15c / 4] & state.registers[0x164 / 4],
            0x170 => state.registers[0x16c / 4] & state.registers[0x174 / 4],
            0x168 | 0x178 => 0,
            _ => state.registers[index],
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
        let index = Self::index(offset, width)?;
        let value = value as u32;
        let mut state = self.state.borrow_mut();
        match offset {
            0x120 => state.hp_sleep |= value & (1 << 31) != 0,
            0x140 | 0x144 | 0x160 | 0x170 | 0x18c..=0x1a0 => {}
            0x168 => state.registers[0x15c / 4] &= !value,
            0x178 => state.registers[0x16c / 4] &= !value,
            0x17c => state.registers[index] = value & 0xfffc_0000,
            0x180 => {
                state.registers[index] = value & 0xffff;
                state.lp_sleep |= value & (1 << 31) != 0;
            }
            0x184 => {
                state.lp_trigger_hp |= value & (1 << 30) != 0;
                state.hp_trigger_lp |= value & (1 << 31) != 0;
            }
            0x3fc => state.registers[index] = value & 0x7fff_ffff,
            _ => state.registers[index] = value,
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = EspC6PmuState::new();
    }
}

struct EspC6LpAonState {
    registers: Vec<u32>,
    system_reset: bool,
    cpu_reset: bool,
}

impl EspC6LpAonState {
    fn new() -> Self {
        let mut registers = vec![0; 0x400 / 4];
        registers[0x38 / 4] = 1 << 30;
        registers[0x48 / 4] = (4 << 19) | (1 << 28) | (1 << 29) | (1 << 31);
        registers[0x4c / 4] = 10 << 22;
        registers[0x3fc / 4] = 35_672_704;
        Self {
            registers,
            system_reset: false,
            cpu_reset: false,
        }
    }
}

/// Scheduler-facing always-on reset and LP-core controls.
#[derive(Clone)]
pub struct EspC6LpAonHandle {
    state: Rc<RefCell<EspC6LpAonState>>,
}

impl EspC6LpAonHandle {
    /// Consumes an HP-system software reset request.
    pub fn take_system_reset(&self) -> bool {
        std::mem::take(&mut self.state.borrow_mut().system_reset)
    }

    /// Consumes a CPU0-only software reset request.
    pub fn take_cpu_reset(&self) -> bool {
        std::mem::take(&mut self.state.borrow_mut().cpu_reset)
    }

    /// Reports whether the LP core is disabled.
    pub fn lp_core_disabled(&self) -> bool {
        self.state.borrow().registers[0x50 / 4] & (1 << 31) != 0
    }

    /// Reports whether HP or LP owns the 16 KiB LP fast-memory window.
    pub fn hp_owns_fast_memory(&self) -> bool {
        self.state.borrow().registers[0x48 / 4] & (1 << 29) != 0
    }
}

/// ESP32-C6 always-on retention registers and reset controls.
pub struct EspC6LpAon {
    name: String,
    state: Rc<RefCell<EspC6LpAonState>>,
}

impl EspC6LpAon {
    /// Creates the always-on page and transition handle.
    pub fn new(name: impl Into<String>) -> (Self, EspC6LpAonHandle) {
        let state = Rc::new(RefCell::new(EspC6LpAonState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspC6LpAonHandle { state },
        )
    }
}

impl Device for EspC6LpAon {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) || offset >= 0x400 {
            return Err(DeviceError::new(
                "ESP32-C6 LP AON requires an aligned word access",
            ));
        }
        Ok(u64::from(
            self.state.borrow().registers[offset as usize / 4],
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) || offset >= 0x400 {
            return Err(DeviceError::new(
                "ESP32-C6 LP AON requires an aligned word access",
            ));
        }
        let value = value as u32;
        let mut state = self.state.borrow_mut();
        match offset {
            0x34 => {
                state.system_reset |= value & (1 << 31) != 0;
                state.registers[offset as usize / 4] = value & (1 << 30);
            }
            0x38 => {
                state.cpu_reset |= value & (1 << 28) != 0;
                state.registers[offset as usize / 4] = value & 0xe000_00ff;
            }
            0x48 => {
                let selected = value & (1 << 31) != 0;
                state.registers[offset as usize / 4] =
                    (value & 0x80ff_0000) | (1 << 28) | (u32::from(selected) << 29);
            }
            0x50 => {
                if value & 1 != 0 {
                    state.registers[offset as usize / 4] &= !(1 << 1);
                }
                state.registers[offset as usize / 4] =
                    (state.registers[offset as usize / 4] & (1 << 1)) | (value & (1 << 31));
            }
            0x3fc => state.registers[offset as usize / 4] = value & 0x7fff_ffff,
            _ => state.registers[offset as usize / 4] = value,
        }
        Ok(())
    }

    fn reset(&mut self, kind: ResetKind) {
        let retained = if kind == ResetKind::PowerOn {
            [0; 10]
        } else {
            let state = self.state.borrow();
            std::array::from_fn(|index| state.registers[index])
        };
        let mut reset = EspC6LpAonState::new();
        reset.registers[..10].copy_from_slice(&retained);
        *self.state.borrow_mut() = reset;
    }
}

struct EspC6LpTimerState {
    targets: [u64; 2],
    enabled: [bool; 2],
    raw: u32,
    hp_enable: u32,
    lp_enable: u32,
    latched: u64,
    update: u32,
    date: u32,
}

impl EspC6LpTimerState {
    fn new() -> Self {
        Self {
            targets: [0; 2],
            enabled: [false; 2],
            raw: 0,
            hp_enable: 0,
            lp_enable: 0,
            latched: 0,
            update: 0,
            date: 34_672_976,
        }
    }

    fn advance(&mut self, at: SimTime) {
        let now = at.ticks() & 0xffff_ffff_ffff;
        if self.enabled[0] && now >= self.targets[0] {
            self.raw |= 1 << 31;
            self.enabled[0] = false;
        }
        if self.enabled[1] && now >= self.targets[1] {
            self.raw |= 1 << 31;
            self.enabled[1] = false;
        }
    }
}

/// Scheduler-facing LP timer alarm state.
#[derive(Clone)]
pub struct EspC6LpTimerHandle {
    state: Rc<RefCell<EspC6LpTimerState>>,
}

impl EspC6LpTimerHandle {
    /// Returns whether the LP-core alarm is pending and enabled.
    pub fn lp_wakeup_pending(&self, at: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        state.advance(at);
        state.raw & state.lp_enable & (1 << 31) != 0
    }

    /// Returns whether the HP wake alarm is pending and enabled.
    pub fn hp_wakeup_pending(&self, at: SimTime) -> bool {
        let mut state = self.state.borrow_mut();
        state.advance(at);
        state.raw & state.hp_enable & (1 << 31) != 0
    }
}

/// Functional 48-bit ESP32-C6 low-power timer.
pub struct EspC6LpTimer {
    name: String,
    state: Rc<RefCell<EspC6LpTimerState>>,
}

impl EspC6LpTimer {
    /// Creates the timer and its wakeup handle.
    pub fn new(name: impl Into<String>) -> (Self, EspC6LpTimerHandle) {
        let state = Rc::new(RefCell::new(EspC6LpTimerState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            EspC6LpTimerHandle { state },
        )
    }
}

impl Device for EspC6LpTimer {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 LP timer requires an aligned word access",
            ));
        }
        let mut state = self.state.borrow_mut();
        state.advance(at);
        let value = match offset {
            0x00 => state.targets[0] as u32,
            0x04 => ((state.targets[0] >> 32) as u32) | (u32::from(state.enabled[0]) << 31),
            0x08 => state.targets[1] as u32,
            0x0c => ((state.targets[1] >> 32) as u32) | (u32::from(state.enabled[1]) << 31),
            0x10 => state.update,
            0x14 => state.latched as u32,
            0x18 => (state.latched >> 32) as u32,
            0x1c => at.ticks() as u32,
            0x20 => (at.ticks() >> 32) as u32 & 0xffff,
            0x24 | 0x34 | 0x44 => 0,
            0x28 | 0x38 => state.raw,
            0x2c => state.raw & state.hp_enable,
            0x30 => state.hp_enable,
            0x3c => state.raw & state.lp_enable,
            0x40 => state.lp_enable,
            0x3fc => state.date,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved read {offset:#x}",
                    self.name
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
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-C6 LP timer requires an aligned word access",
            ));
        }
        let value = value as u32;
        let mut state = self.state.borrow_mut();
        match offset {
            0x00 => state.targets[0] = (state.targets[0] & !0xffff_ffff) | u64::from(value),
            0x04 => {
                state.targets[0] =
                    (state.targets[0] & 0xffff_ffff) | (u64::from(value & 0xffff) << 32);
                state.enabled[0] |= value & (1 << 31) != 0;
            }
            0x08 => state.targets[1] = (state.targets[1] & !0xffff_ffff) | u64::from(value),
            0x0c => {
                state.targets[1] =
                    (state.targets[1] & 0xffff_ffff) | (u64::from(value & 0xffff) << 32);
                state.enabled[1] |= value & (1 << 31) != 0;
            }
            0x10 => {
                state.update = value & 0xe000_0000;
                if value & (1 << 28) != 0 {
                    state.latched = at.ticks();
                }
            }
            0x24 => {
                if value & (1 << 31) != 0 {
                    state.raw &= !(1 << 30);
                }
            }
            0x28 | 0x2c | 0x38 | 0x3c => {}
            0x30 => state.hp_enable = value & 0xc000_0000,
            0x34 | 0x44 => state.raw &= !(value & 0xc000_0000),
            0x40 => state.lp_enable = value & 0xc000_0000,
            0x3fc => state.date = value & 0x7fff_ffff,
            _ => {
                return Err(DeviceError::new(format!(
                    "{} reserved write {offset:#x}",
                    self.name
                )));
            }
        }
        state.advance(at);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = EspC6LpTimerState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmu_strobes_drive_both_core_transition_directions() {
        let (mut pmu, handle) = EspC6Pmu::new("pmu");
        pmu.write(0x184, AccessWidth::Word, 3 << 30, SimTime::ZERO)
            .unwrap();
        assert!(handle.take_lp_trigger_hp());
        assert!(handle.take_hp_trigger_lp());
        assert!(!handle.take_hp_trigger_lp());
        pmu.write(
            0x180,
            AccessWidth::Word,
            (1 << 31) | (1 << 4),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.lp_wakeup_mask(), 1 << 4);
        assert!(handle.take_lp_sleep());
    }

    #[test]
    fn lp_aon_store_registers_survive_non_power_resets() {
        let (mut aon, handle) = EspC6LpAon::new("aon");
        aon.write(0, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        aon.write(0x34, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        assert!(handle.take_system_reset());
        aon.reset(ResetKind::Watchdog);
        assert_eq!(
            aon.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x1234_5678
        );
        aon.reset(ResetKind::PowerOn);
        assert_eq!(aon.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
    }

    #[test]
    fn lp_timer_raises_hp_and_lp_wakeup_status() {
        let (mut timer, handle) = EspC6LpTimer::new("timer");
        timer
            .write(0, AccessWidth::Word, 10, SimTime::ZERO)
            .unwrap();
        timer
            .write(4, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        timer
            .write(0x30, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        timer
            .write(0x40, AccessWidth::Word, 1 << 31, SimTime::ZERO)
            .unwrap();
        assert!(!handle.lp_wakeup_pending(SimTime::from_ticks(9)));
        assert!(handle.lp_wakeup_pending(SimTime::from_ticks(10)));
        assert!(handle.hp_wakeup_pending(SimTime::from_ticks(10)));
        timer
            .write(0x44, AccessWidth::Word, 1 << 31, SimTime::from_ticks(10))
            .unwrap();
        assert!(!handle.lp_wakeup_pending(SimTime::from_ticks(10)));
    }
}
