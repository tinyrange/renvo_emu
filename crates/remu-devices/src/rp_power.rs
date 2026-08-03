use super::*;

const PSM_MASK: u32 = 0x0001_ffff;
const ROSC_CTRL_MASK: u32 = 0x00ff_ffff;
const ROSC_ENABLE_DISABLE: u32 = 0xd1e;
const ROSC_ENABLE_ENABLE: u32 = 0xfab;
const ROSC_RANGE_LOW: u32 = 0xfa4;
const ROSC_RANGE_MEDIUM: u32 = 0xfa5;
const ROSC_RANGE_HIGH: u32 = 0xfa7;
const ROSC_RANGE_TOO_HIGH: u32 = 0xfa6;
const ROSC_DIV_PASS: u32 = 0xaa0;
const ROSC_DIV_MAX: u32 = ROSC_DIV_PASS + 31;
const ROSC_WAKE: u32 = 0x7761_6b65;
const ROSC_DORMANT: u32 = 0x636f_6d61;
const ROSC_STATUS_STABLE: u32 = 1 << 31;
const ROSC_STATUS_BADWRITE: u32 = 1 << 24;
const ROSC_STATUS_DIV_RUNNING: u32 = 1 << 16;
const ROSC_STATUS_ENABLED: u32 = 1 << 12;
const VREG_MASK: u32 = 0x0000_00f3;
const VREG_ROK: u32 = 1 << 12;
const BOD_MASK: u32 = 0x0000_00f1;
const CHIP_RESET_PSM_RESTART: u32 = 1 << 24;

/// Register offsets in the RP2040 power state machine block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Rp2040PsmRegister {
    /// Force a power-managed block out of reset.
    ForceOn = 0x00,
    /// Force a power-managed block into reset.
    ForceOff = 0x04,
    /// Select blocks reset by the watchdog.
    WatchdogSelect = 0x08,
    /// Indicate that power-managed blocks are ready.
    Done = 0x0c,
}

impl TryFrom<u64> for Rp2040PsmRegister {
    type Error = DeviceError;

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x00 => Ok(Self::ForceOn),
            0x04 => Ok(Self::ForceOff),
            0x08 => Ok(Self::WatchdogSelect),
            0x0c => Ok(Self::Done),
            _ => Err(DeviceError::new(format!(
                "unmodeled RP2040 PSM register at offset {offset:#x}"
            ))),
        }
    }
}

/// Register offsets in the RP2040 ring-oscillator block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Rp2040RoscRegister {
    /// Protected enable and frequency-range control.
    Control = 0x00,
    /// Stage 0–3 drive-strength controls.
    FrequencyA = 0x04,
    /// Stage 4–7 drive-strength controls.
    FrequencyB = 0x08,
    /// Dormant and wake command register.
    Dormant = 0x0c,
    /// Protected ROSC output divider.
    Divider = 0x10,
    /// Protected phase-shifted output control.
    Phase = 0x14,
    /// Stable, error and divider status.
    Status = 0x18,
    /// Read-only oscillator output bit.
    RandomBit = 0x1c,
    /// Short countdown timer driven by ROSC.
    Count = 0x20,
}

impl TryFrom<u64> for Rp2040RoscRegister {
    type Error = DeviceError;

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x00 => Ok(Self::Control),
            0x04 => Ok(Self::FrequencyA),
            0x08 => Ok(Self::FrequencyB),
            0x0c => Ok(Self::Dormant),
            0x10 => Ok(Self::Divider),
            0x14 => Ok(Self::Phase),
            0x18 => Ok(Self::Status),
            0x1c => Ok(Self::RandomBit),
            0x20 => Ok(Self::Count),
            _ => Err(DeviceError::new(format!(
                "unmodeled RP2040 ROSC register at offset {offset:#x}"
            ))),
        }
    }
}

/// Register offsets in the RP2040 regulator and chip-reset block.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum Rp2040VregRegister {
    /// Core regulator controls and status.
    Vreg = 0x00,
    /// Brown-out detector controls.
    Bod = 0x04,
    /// Reset-cause and PSM restart status.
    ChipReset = 0x08,
}

impl TryFrom<u64> for Rp2040VregRegister {
    type Error = DeviceError;

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x00 => Ok(Self::Vreg),
            0x04 => Ok(Self::Bod),
            0x08 => Ok(Self::ChipReset),
            _ => Err(DeviceError::new(format!(
                "unmodeled RP2040 VREG_AND_CHIP_RESET register at offset {offset:#x}"
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
        _ => Err(DeviceError::new("invalid RP2040 atomic alias")),
    }
}

fn word_access(width: AccessWidth, name: &str) -> Result<(), DeviceError> {
    if width != AccessWidth::Word {
        Err(DeviceError::new(format!("{name} requires word access")))
    } else {
        Ok(())
    }
}

/// Functional RP2040 power-on state machine.
///
/// The model exposes the documented force-on, force-off, watchdog-select, and
/// done masks. It does not gate unrelated devices in the bus; callers can use
/// the masks to observe and test power sequencing without analogue timing.
pub struct Rp2040Psm {
    name: String,
    force_on: u32,
    force_off: u32,
    watchdog_select: u32,
}

impl Rp2040Psm {
    /// Creates a PSM in the post-startup state where every block is ready.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            force_on: 0,
            force_off: 0,
            watchdog_select: 0,
        }
    }

    fn done(&self) -> u32 {
        (PSM_MASK & !self.force_off) | self.force_on
    }

    fn write_force_on(&mut self, alias: u64, value: u32) -> Result<(), DeviceError> {
        self.force_on = atomic_update(self.force_on, alias, value)? & PSM_MASK;
        self.force_off &= !self.force_on;
        Ok(())
    }

    fn write_force_off(&mut self, alias: u64, value: u32) -> Result<(), DeviceError> {
        self.force_off = atomic_update(self.force_off, alias, value)? & PSM_MASK;
        self.force_on &= !self.force_off;
        Ok(())
    }
}

impl Device for Rp2040Psm {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        word_access(width, "RP2040 PSM")?;
        let register = Rp2040PsmRegister::try_from(offset & 0x0fff)?;
        let value = match register {
            Rp2040PsmRegister::ForceOn => self.force_on,
            Rp2040PsmRegister::ForceOff => self.force_off,
            Rp2040PsmRegister::WatchdogSelect => self.watchdog_select,
            Rp2040PsmRegister::Done => self.done(),
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
        word_access(width, "RP2040 PSM")?;
        let alias = (offset >> 12) & 3;
        let register = Rp2040PsmRegister::try_from(offset & 0x0fff)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked PSM value fits");
        match register {
            Rp2040PsmRegister::ForceOn => self.write_force_on(alias, value & PSM_MASK)?,
            Rp2040PsmRegister::ForceOff => self.write_force_off(alias, value & PSM_MASK)?,
            Rp2040PsmRegister::WatchdogSelect => {
                self.watchdog_select =
                    atomic_update(self.watchdog_select, alias, value)? & PSM_MASK;
            }
            Rp2040PsmRegister::Done => {
                return Err(DeviceError::new("RP2040 PSM DONE is read-only"));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.force_on = 0;
        self.force_off = 0;
        self.watchdog_select = 0;
    }
}

/// Functional RP2040 ring oscillator with deterministic status and countdown behavior.
pub struct Rp2040Rosc {
    name: String,
    control: u32,
    freqa: u32,
    freqb: u32,
    dormant: u32,
    div: u32,
    phase: u32,
    badwrite: bool,
    div_running: bool,
    dormant_mode: bool,
    count: u8,
    last_count_at: SimTime,
}

impl Rp2040Rosc {
    /// Creates a ROSC in its documented power-up configuration.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            control: 0xaa0,
            freqa: 0,
            freqb: 0,
            dormant: ROSC_WAKE,
            div: ROSC_DIV_PASS + 16,
            phase: 0x8,
            badwrite: false,
            div_running: false,
            dormant_mode: false,
            count: 0,
            last_count_at: SimTime::ZERO,
        }
    }

    fn enabled(&self) -> bool {
        (self.control >> 12) & 0xfff != ROSC_ENABLE_DISABLE
    }

    fn stable(&self) -> bool {
        self.enabled() && !self.dormant_mode
    }

    fn status(&self) -> u32 {
        let mut status = 0;
        if self.stable() {
            status |= ROSC_STATUS_STABLE;
        }
        if self.badwrite {
            status |= ROSC_STATUS_BADWRITE;
        }
        if self.div_running && self.stable() {
            status |= ROSC_STATUS_DIV_RUNNING;
        }
        if self.enabled() {
            status |= ROSC_STATUS_ENABLED;
        }
        status
    }

    fn update_count(&mut self, at: SimTime) {
        let elapsed = at.ticks().saturating_sub(self.last_count_at.ticks());
        if self.count != 0 {
            self.count = self
                .count
                .saturating_sub(u8::try_from(elapsed).unwrap_or(u8::MAX));
        }
        self.last_count_at = at;
    }

    fn mark_badwrite(&mut self) {
        self.badwrite = true;
    }

    fn write_control(&mut self, value: u32) {
        let mut enable = (value >> 12) & 0xfff;
        let mut range = value & 0xfff;
        let enable_valid = enable == ROSC_ENABLE_DISABLE || enable == ROSC_ENABLE_ENABLE;
        let range_valid = matches!(
            range,
            ROSC_RANGE_LOW | ROSC_RANGE_MEDIUM | ROSC_RANGE_HIGH | ROSC_RANGE_TOO_HIGH
        );
        if !enable_valid {
            self.mark_badwrite();
            enable = ROSC_ENABLE_ENABLE;
        }
        if !range_valid {
            self.mark_badwrite();
            range = ROSC_RANGE_LOW;
        }
        self.control = ((enable << 12) | range) & ROSC_CTRL_MASK;
        self.div_running = self.enabled() && !self.dormant_mode;
    }

    fn write_frequency(&mut self, value: u32, first: bool) {
        let mask = 0xffff_7777;
        let destination = if first {
            &mut self.freqa
        } else {
            &mut self.freqb
        };
        if value >> 16 != 0x9696 {
            *destination = 0;
            self.mark_badwrite();
        } else {
            *destination = value & mask;
        }
    }

    fn write_div(&mut self, value: u32) {
        let value = value & 0xfff;
        if (ROSC_DIV_PASS..=ROSC_DIV_MAX).contains(&value) {
            self.div = value;
        } else {
            self.div = ROSC_DIV_MAX;
            self.mark_badwrite();
        }
        self.div_running = self.enabled() && !self.dormant_mode;
    }

    fn write_phase(&mut self, value: u32) {
        if ((value >> 4) & 0xff) == 0xaa {
            self.phase = value & 0xfff;
        } else {
            self.phase = 0x8;
            self.mark_badwrite();
        }
    }
}

impl Device for Rp2040Rosc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        word_access(width, "RP2040 ROSC")?;
        self.update_count(at);
        let register = Rp2040RoscRegister::try_from(offset & 0x0fff)?;
        let value = match register {
            Rp2040RoscRegister::Control => self.control,
            Rp2040RoscRegister::FrequencyA => self.freqa,
            Rp2040RoscRegister::FrequencyB => self.freqb,
            Rp2040RoscRegister::Dormant => self.dormant,
            Rp2040RoscRegister::Divider => self.div,
            Rp2040RoscRegister::Phase => self.phase,
            Rp2040RoscRegister::Status => self.status(),
            // The hardware exposes the oscillator output, whose reset value is 1.
            // Keep this functional model deterministic while preserving that value.
            Rp2040RoscRegister::RandomBit => 1,
            Rp2040RoscRegister::Count => u32::from(self.count),
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
        word_access(width, "RP2040 ROSC")?;
        self.update_count(at);
        let register = Rp2040RoscRegister::try_from(offset & 0x0fff)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked ROSC value fits");
        match register {
            Rp2040RoscRegister::Control => self.write_control(value & ROSC_CTRL_MASK),
            Rp2040RoscRegister::FrequencyA => self.write_frequency(value, true),
            Rp2040RoscRegister::FrequencyB => self.write_frequency(value, false),
            Rp2040RoscRegister::Dormant => {
                if value == ROSC_DORMANT {
                    self.dormant = ROSC_DORMANT;
                    self.dormant_mode = true;
                } else {
                    if value != ROSC_WAKE {
                        self.mark_badwrite();
                    }
                    self.dormant = ROSC_WAKE;
                    self.dormant_mode = false;
                }
                self.div_running = self.enabled() && !self.dormant_mode;
            }
            Rp2040RoscRegister::Divider => self.write_div(value),
            Rp2040RoscRegister::Phase => self.write_phase(value),
            Rp2040RoscRegister::Status => {
                if value & ROSC_STATUS_BADWRITE != 0 {
                    self.badwrite = false;
                }
            }
            Rp2040RoscRegister::RandomBit => {
                return Err(DeviceError::new("RP2040 ROSC RANDOMBIT is read-only"));
            }
            Rp2040RoscRegister::Count => {
                self.count = u8::try_from(value & 0xff).expect("masked ROSC count fits");
                self.last_count_at = at;
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}

/// Functional RP2040 voltage regulator and chip-reset status block.
pub struct Rp2040VregAndChipReset {
    name: String,
    vreg: u32,
    bod: u32,
    chip_reset: u32,
}

impl Rp2040VregAndChipReset {
    /// Creates the post-power-on register state.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            vreg: 0x0000_00b1,
            bod: 0x0000_0091,
            chip_reset: 0,
        }
    }

    fn regulator_ok(&self) -> bool {
        self.vreg & 1 != 0 && self.vreg & 2 == 0
    }
}

impl Device for Rp2040VregAndChipReset {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        word_access(width, "RP2040 VREG_AND_CHIP_RESET")?;
        let register = Rp2040VregRegister::try_from(offset & 0x0fff)?;
        let value = match register {
            Rp2040VregRegister::Vreg => {
                self.vreg | self.regulator_ok().then_some(VREG_ROK).unwrap_or(0)
            }
            Rp2040VregRegister::Bod => self.bod,
            Rp2040VregRegister::ChipReset => self.chip_reset,
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
        word_access(width, "RP2040 VREG_AND_CHIP_RESET")?;
        let alias = (offset >> 12) & 3;
        let register = Rp2040VregRegister::try_from(offset & 0x0fff)?;
        let value = u32::try_from(value & u64::from(u32::MAX))
            .expect("masked VREG_AND_CHIP_RESET value fits");
        match register {
            Rp2040VregRegister::Vreg => {
                self.vreg = atomic_update(self.vreg, alias, value)? & VREG_MASK
            }
            Rp2040VregRegister::Bod => self.bod = atomic_update(self.bod, alias, value)? & BOD_MASK,
            Rp2040VregRegister::ChipReset => {
                // PSM_RESTART_FLAG is write-one-clear; the read-only reset-cause bits
                // remain host-controlled and are not changed by firmware writes.
                if value & CHIP_RESET_PSM_RESTART != 0 {
                    self.chip_reset &= !CHIP_RESET_PSM_RESTART;
                }
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self = Self::new(self.name.clone());
    }
}
