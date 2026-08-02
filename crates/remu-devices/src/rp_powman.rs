use super::*;

const REGISTER_COUNT: usize = 0xec / 4 + 1;
const BADPASSWD: usize = 0x00 / 4;
const VREG_STS: usize = 0x08 / 4;
const CHIP_RESET: usize = 0x2c / 4;
const SET_TIME_63_48: usize = 0x60 / 4;
const SET_TIME_47_32: usize = 0x64 / 4;
const SET_TIME_31_16: usize = 0x68 / 4;
const SET_TIME_15_0: usize = 0x6c / 4;
const READ_TIME_UPPER: usize = 0x70 / 4;
const READ_TIME_LOWER: usize = 0x74 / 4;
const ALARM_TIME_63_48: usize = 0x78 / 4;
const ALARM_TIME_47_32: usize = 0x7c / 4;
const ALARM_TIME_31_16: usize = 0x80 / 4;
const ALARM_TIME_15_0: usize = 0x84 / 4;
const TIMER: usize = 0x88 / 4;
const CURRENT_PWRUP_REQ: usize = 0x9c / 4;
const LAST_SWCORE_PWRUP: usize = 0xa0 / 4;
const BOOTDIS: usize = 0xa8 / 4;
const SCRATCH0: usize = 0xb0 / 4;
const BOOT0: usize = 0xd0 / 4;
const INTR: usize = 0xe0 / 4;
const INTE: usize = 0xe4 / 4;
const INTF: usize = 0xe8 / 4;
const INTS: usize = 0xec / 4;

const TIMER_RUN: u32 = 1 << 1;
const TIMER_CLEAR: u32 = 1 << 2;
const TIMER_ALARM_ENABLE: u32 = 1 << 4;
const TIMER_ALARM: u32 = 1 << 6;
const TIMER_SOURCE_LPOSC: u32 = 1 << 8;
const TIMER_SOURCE_XOSC: u32 = 1 << 9;
const TIMER_SOURCE_GPIO_1KHZ: u32 = 1 << 10;
const TIMER_SOURCE_GPIO_1HZ: u32 = 1 << 13;
const TIMER_SOURCE_STATUS: u32 = 0x000f_0000;
const TIMER_WRITABLE: u32 = 0x0000_2777;
const CHIP_RESET_RO: u32 = 0x1fef_0000;

fn atomic_update(current: u32, alias: u64, value: u32) -> u32 {
    match alias {
        0 => value,
        1 => current ^ value,
        2 => current | value,
        3 => current & !value,
        _ => unreachable!("atomic alias is limited to two bits"),
    }
}

fn register_mask(offset: u64) -> Option<u32> {
    Some(match offset {
        0x00 => 0x0000_0001,
        0x04 => 0x0000_b170,
        0x08 => 0x0000_0011,
        0x0c => 0x0000_81f2,
        0x10 | 0x14 => 0x0000_01f6,
        0x18 => 0x0000_1000,
        0x1c | 0x20 | 0x24 => 0x0000_01f1,
        0x28 => 0x0000_03f3,
        0x2c => 0x1fef_0011,
        0x30 => 0x0000_1111,
        0x34 => 0x0013_11f3,
        0x38 => 0x0000_3fff,
        0x3c => 0x0000_07ff,
        0x40 => 0x0000_ffff,
        0x44 | 0x48 => 0x0000_713f,
        0x4c => 0x0000_0013,
        0x50 => 0x0000_003f,
        0x54 | 0x58 | 0x5c => 0x0000_ffff,
        0x60..=0x6c | 0x78..=0x84 => 0x0000_ffff,
        0x70 | 0x74 => u32::MAX,
        0x88 => 0x000f_2777,
        0x8c..=0x98 => 0x0000_07ff,
        0x9c..=0xa0 => 0x0000_007f,
        0xa4 => 1,
        0xa8 => 3,
        0xac => 0xf,
        0xb0..=0xdc => u32::MAX,
        0xe0..=0xec => 0xf,
        _ => return None,
    })
}

/// Functional RP2350 power manager and always-on timer subset.
///
/// This model preserves the documented reset values, power-sequencer and
/// wake-source configuration, scratch/boot words, and the 64-bit AON timer.
/// It does not model analog voltage, actual power-domain transitions, or
/// electrical wake pins; those remain outside a functional firmware baseline.
pub struct Rp2350Powman {
    name: String,
    reset: [u32; REGISTER_COUNT],
    registers: [u32; REGISTER_COUNT],
    time_value: u64,
    time_epoch: SimTime,
}

impl Rp2350Powman {
    /// Creates the documented reset state.
    pub fn new(name: impl Into<String>) -> Self {
        let mut reset = [0; REGISTER_COUNT];
        for (offset, value) in [
            (0x04, 0x8050),
            (0x0c, 0x00b0),
            (0x10, 0x00b4),
            (0x14, 0x00b0),
            (0x1c, 0x00b1),
            (0x20, 0x00b0),
            (0x24, 0x00b1),
            (0x28, 0x0203),
            (0x34, 0x1011_f0),
            (0x38, 0x000f),
            (0x3c, 0x0040),
            (0x40, 0x2011),
            (0x44, 0x003f),
            (0x48, 0x003f),
            (0x50, 0x20),
            (0x54, 0xc49c),
            (0x58, 0x2ee0),
            (0x8c, 0x3f),
            (0x90, 0x3f),
            (0x94, 0x3f),
            (0x98, 0x3f),
        ] {
            reset[offset / 4] = value;
        }
        Self {
            name: name.into(),
            registers: reset,
            reset,
            time_value: 0,
            time_epoch: SimTime::ZERO,
        }
    }

    /// Returns the current AON timer value at a simulation timestamp.
    pub fn aon_time(&self, at: SimTime) -> u64 {
        if self.registers[TIMER] & TIMER_RUN != 0 {
            self.time_value
                .wrapping_add(at.ticks().saturating_sub(self.time_epoch.ticks()))
        } else {
            self.time_value
        }
    }

    /// Returns whether the AON alarm is currently pending.
    pub fn alarm_pending(&self, at: SimTime) -> bool {
        self.registers[TIMER] & TIMER_ALARM_ENABLE != 0 && self.aon_time(at) >= self.alarm_time()
    }

    fn alarm_time(&self) -> u64 {
        (u64::from(self.registers[ALARM_TIME_63_48] & 0xffff) << 48)
            | (u64::from(self.registers[ALARM_TIME_47_32] & 0xffff) << 32)
            | (u64::from(self.registers[ALARM_TIME_31_16] & 0xffff) << 16)
            | u64::from(self.registers[ALARM_TIME_15_0] & 0xffff)
    }

    fn set_time(&self) -> u64 {
        (u64::from(self.registers[SET_TIME_63_48] & 0xffff) << 48)
            | (u64::from(self.registers[SET_TIME_47_32] & 0xffff) << 32)
            | (u64::from(self.registers[SET_TIME_31_16] & 0xffff) << 16)
            | u64::from(self.registers[SET_TIME_15_0] & 0xffff)
    }

    fn update_time_from_set_registers(&mut self, at: SimTime) {
        self.time_value = self.set_time();
        self.time_epoch = at;
    }

    fn timer_readback(&self, at: SimTime) -> u32 {
        let mut value = self.registers[TIMER] & TIMER_WRITABLE;
        if self.alarm_pending(at) {
            value |= TIMER_ALARM;
        }
        let source = match value
            & (TIMER_SOURCE_LPOSC
                | TIMER_SOURCE_XOSC
                | TIMER_SOURCE_GPIO_1KHZ
                | TIMER_SOURCE_GPIO_1HZ)
        {
            TIMER_SOURCE_LPOSC => 1 << 16,
            TIMER_SOURCE_XOSC => 1 << 17,
            TIMER_SOURCE_GPIO_1KHZ => 1 << 18,
            TIMER_SOURCE_GPIO_1HZ => 1 << 19,
            _ => 0,
        };
        value | (source & TIMER_SOURCE_STATUS)
    }

    fn reset_registers(&mut self, kind: ResetKind) {
        let mut persistent = [0; 12];
        persistent.copy_from_slice(&self.registers[SCRATCH0..BOOT0 + 4]);
        self.registers = self.reset;
        if kind == ResetKind::Watchdog {
            self.registers[SCRATCH0..BOOT0 + 4].copy_from_slice(&persistent);
        }
        self.time_value = 0;
        self.time_epoch = SimTime::ZERO;
    }
}

impl Device for Rp2350Powman {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2350 POWMAN requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        let mask = register_mask(register_offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2350 POWMAN read at offset {register_offset:#x}"
            ))
        })?;
        let index = usize::try_from(register_offset / 4).expect("POWMAN index fits");
        let value = match index {
            READ_TIME_UPPER => (self.aon_time(at) >> 32) as u32,
            READ_TIME_LOWER => self.aon_time(at) as u32,
            TIMER => self.timer_readback(at),
            INTR => self.registers[INTR] | u32::from(self.alarm_pending(at)) << 1,
            INTS => {
                let raw = self.registers[INTR] | u32::from(self.alarm_pending(at)) << 1;
                raw & self.registers[INTE] | self.registers[INTF]
            }
            _ => self.registers[index] & mask,
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
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2350 POWMAN requires aligned word access",
            ));
        }
        let register_offset = offset & 0x0fff;
        let mask = register_mask(register_offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2350 POWMAN write at offset {register_offset:#x}"
            ))
        })?;
        let index = usize::try_from(register_offset / 4).expect("POWMAN index fits");
        let alias = (offset >> 12) & 3;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("POWMAN value fits");
        if matches!(
            index,
            VREG_STS
                | CURRENT_PWRUP_REQ
                | LAST_SWCORE_PWRUP
                | READ_TIME_UPPER
                | READ_TIME_LOWER
                | INTS
        ) {
            return Err(DeviceError::new("RP2350 POWMAN register is read-only"));
        }
        match index {
            BADPASSWD => self.registers[index] &= !value,
            CHIP_RESET => {
                let current = self.registers[index];
                let updated = atomic_update(current, alias, value & !CHIP_RESET_RO);
                self.registers[index] = (current & CHIP_RESET_RO) | (updated & !CHIP_RESET_RO);
                if value & (1 << 4) != 0 {
                    self.registers[index] &= !(1 << 4);
                }
            }
            TIMER => {
                let was_running = self.registers[TIMER] & TIMER_RUN != 0;
                let now = self.aon_time(at);
                let updated = atomic_update(self.registers[TIMER], alias, value & TIMER_WRITABLE);
                if value & TIMER_CLEAR != 0 {
                    self.time_value = 0;
                    self.time_epoch = at;
                } else if was_running && updated & TIMER_RUN == 0 {
                    self.time_value = now;
                    self.time_epoch = at;
                } else if !was_running && updated & TIMER_RUN != 0 {
                    self.time_epoch = at;
                }
                self.registers[TIMER] = updated & TIMER_WRITABLE;
            }
            INTR => self.registers[index] &= !(value & mask),
            BOOTDIS => self.registers[index] |= value & mask,
            i if (SET_TIME_63_48..=SET_TIME_15_0).contains(&i) => {
                self.registers[i] = atomic_update(self.registers[i], alias, value) & mask;
                self.update_time_from_set_registers(at);
            }
            _ => self.registers[index] = atomic_update(self.registers[index], alias, value) & mask,
        }
        Ok(())
    }

    fn reset(&mut self, kind: ResetKind) {
        self.reset_registers(kind);
    }
}
