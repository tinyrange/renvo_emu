use super::*;

const CONTROL_LIMIT: u64 = 0x1_0000;
const DATA_ECC: u64 = 0x1_0000;
const DATA_RAW: u64 = 0x1_4000;
const DATA_ECC_GUARDED: u64 = 0x1_8000;
const DATA_RAW_GUARDED: u64 = 0x1_c000;
const DATA_SIZE: usize = 8 * 1024;
const DATA_WORDS: usize = DATA_SIZE / 4;
const USR_DCTRL: u32 = 1;
const SBPI_STATUS_RDATA_VALID: u32 = 1;
const SBPI_STATUS_INSTR_DONE: u32 = 1 << 4;
const INTR_MASK: u32 = 0x1f;

fn atomic_update(current: u32, alias: u64, value: u32) -> Result<u32, DeviceError> {
    match alias {
        0 => Ok(value),
        1 => Ok(current ^ value),
        2 => Ok(current | value),
        3 => Ok(current & !value),
        _ => Err(DeviceError::new("invalid RP2350 OTP atomic alias")),
    }
}

fn aligned_word(width: AccessWidth, offset: u64) -> Result<(), DeviceError> {
    if width != AccessWidth::Word || !width.is_aligned(offset) {
        Err(DeviceError::new(
            "RP2350 OTP register requires aligned word access",
        ))
    } else {
        Ok(())
    }
}

/// Functional RP2350 OTP storage and control interface.
///
/// The reset image is blank and deterministic. The four documented read aliases,
/// page software locks, user data-read control, debug/architecture controls, and
/// interrupt windows are modeled. Fuse programming remains intentionally outside
/// the functional emulator; SBPI commands complete without changing the blank
/// image so tests cannot accidentally claim irreversible hardware behavior.
pub struct Rp2350Otp {
    name: String,
    data: [u32; DATA_WORDS],
    locks: [u32; 64],
    sbpi_instr: u32,
    sbpi_wdata: [u32; 4],
    sbpi_rdata: [u32; 4],
    sbpi_status: u32,
    usr: u32,
    dbg: u32,
    bist: u32,
    critical: u32,
    key_valid: u32,
    debugen: u32,
    debugen_lock: u32,
    archsel: u32,
    archsel_status: u32,
    bootdis: u32,
    intr: u32,
    inte: u32,
    intf: u32,
}

impl Rp2350Otp {
    /// Creates a blank RP2350 OTP image.
    pub fn new(name: impl Into<String>) -> Self {
        Self::with_words(name, &[])
    }

    /// Creates an OTP image from up to 8 KiB of little-endian words.
    pub fn with_words(name: impl Into<String>, words: &[u32]) -> Self {
        let mut data = [0; DATA_WORDS];
        let count = words.len().min(DATA_WORDS);
        data[..count].copy_from_slice(&words[..count]);
        Self {
            name: name.into(),
            data,
            locks: [0; 64],
            sbpi_instr: 0,
            sbpi_wdata: [0; 4],
            sbpi_rdata: [0; 4],
            sbpi_status: 0,
            usr: USR_DCTRL,
            dbg: 0,
            bist: 0x0fff_0000,
            critical: 0,
            key_valid: 0,
            debugen: 0,
            debugen_lock: 0,
            archsel: 0,
            archsel_status: 0,
            bootdis: 0,
            intr: 0,
            inte: 0,
            intf: 0,
        }
    }

    fn read_control(&mut self, register: u64) -> Result<u32, DeviceError> {
        let value = match register {
            0x000..=0x0fc if register % 4 == 0 => {
                self.locks[usize::try_from(register / 4).expect("OTP lock index fits")]
            }
            0x100 => self.sbpi_instr,
            0x104..=0x110 if (register - 0x104) % 4 == 0 => {
                self.sbpi_wdata[usize::try_from((register - 0x104) / 4).expect("OTP word fits")]
            }
            0x114..=0x120 if (register - 0x114) % 4 == 0 => {
                let index = usize::try_from((register - 0x114) / 4).expect("OTP word fits");
                let value = self.sbpi_rdata[index];
                self.sbpi_rdata[index] = 0;
                if self.sbpi_rdata.iter().all(|word| *word == 0) {
                    self.sbpi_status &= !SBPI_STATUS_RDATA_VALID;
                }
                value
            }
            0x124 => self.sbpi_status,
            0x128 => self.usr,
            0x12c => self.dbg,
            0x134 => self.bist,
            0x138..=0x144 if (register - 0x138) % 4 == 0 => 0,
            0x148 => self.critical,
            0x14c => self.key_valid,
            0x150 => self.debugen,
            0x154 => self.debugen_lock,
            0x158 => self.archsel,
            0x15c => self.archsel_status,
            0x160 => self.bootdis,
            0x164 => self.intr,
            0x168 => self.inte,
            0x16c => self.intf,
            0x170 => (self.intr | self.intf) & self.inte,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 OTP read at offset {register:#x}"
                )));
            }
        };
        Ok(value)
    }

    fn write_control(&mut self, register: u64, alias: u64, value: u32) -> Result<(), DeviceError> {
        match register {
            0x000..=0x0fc if register % 4 == 0 => {
                let index = usize::try_from(register / 4).expect("OTP lock index fits");
                // Lock states only advance until reset. Atomic clear/XOR aliases cannot
                // reopen a page, matching the fuse-backed lock shim.
                self.locks[index] |= value & 0x0f;
            }
            0x100 => {
                self.sbpi_instr = atomic_update(self.sbpi_instr, alias, value)? & 0x7fff_ffff;
                if value & (1 << 30) != 0 {
                    self.sbpi_status |= SBPI_STATUS_INSTR_DONE;
                    self.sbpi_status &= !SBPI_STATUS_RDATA_VALID;
                }
            }
            0x104..=0x110 if (register - 0x104) % 4 == 0 => {
                let index = usize::try_from((register - 0x104) / 4).expect("OTP word fits");
                self.sbpi_wdata[index] = atomic_update(self.sbpi_wdata[index], alias, value)?;
            }
            0x124 => {
                self.sbpi_status &= !(value & (SBPI_STATUS_INSTR_DONE | 0x100));
            }
            0x128 => {
                self.usr = atomic_update(self.usr, alias, value)? & 0x11;
            }
            0x12c => {
                self.dbg = atomic_update(self.dbg, alias, value)? & 0x10ff;
            }
            0x134 => {
                self.bist = atomic_update(self.bist, alias, value)? & 0x7fff_1fff;
            }
            0x138..=0x144 if (register - 0x138) % 4 == 0 => {
                // Certificate key registers are write-only. Retain no secret material.
            }
            0x150 => {
                let updated = atomic_update(self.debugen, alias, value)? & 0x10f;
                self.debugen = (self.debugen & self.debugen_lock) | (updated & !self.debugen_lock);
            }
            0x154 => {
                self.debugen_lock = atomic_update(self.debugen_lock, alias, value)? & 0x10f;
            }
            0x158 => self.archsel = atomic_update(self.archsel, alias, value)? & 3,
            0x15c => {}
            0x160 => self.bootdis = atomic_update(self.bootdis, alias, value)? & 3,
            0x164 => self.intr &= !(value & INTR_MASK),
            0x168 => self.inte = atomic_update(self.inte, alias, value)? & INTR_MASK,
            0x16c => self.intf = atomic_update(self.intf, alias, value)? & INTR_MASK,
            0x170 => self.intr &= !(value & INTR_MASK),
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 OTP write at offset {register:#x}"
                )));
            }
        }
        self.archsel_status = self.archsel;
        Ok(())
    }

    fn read_data(&self, offset: u64) -> Result<u32, DeviceError> {
        if self.usr & USR_DCTRL == 0 {
            return Err(DeviceError::new(
                "RP2350 OTP data reads disabled by USR.DCTRL",
            ));
        }
        let alias = match offset & 0x1_ffff {
            DATA_ECC => DATA_ECC,
            DATA_RAW => DATA_RAW,
            DATA_ECC_GUARDED => DATA_ECC_GUARDED,
            DATA_RAW_GUARDED => DATA_RAW_GUARDED,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP2350 OTP data read at offset {offset:#x}"
                )));
            }
        };
        let byte_offset = offset - alias;
        if byte_offset >= u64::try_from(DATA_SIZE).expect("OTP size fits") {
            return Ok(0);
        }
        let index = usize::try_from(byte_offset / 4).expect("OTP data index fits");
        let word = self.data[index] & 0x00ff_ffff;
        Ok(word)
    }
}

impl Device for Rp2350Otp {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        aligned_word(width, offset)?;
        let value = if offset < CONTROL_LIMIT {
            self.read_control(offset & 0x0fff)?
        } else {
            self.read_data(offset)?
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
        aligned_word(width, offset)?;
        if offset >= CONTROL_LIMIT {
            return Err(DeviceError::new("RP2350 OTP data aliases are read-only"));
        }
        self.write_control(offset & 0x0fff, (offset >> 12) & 3, value as u32)
    }

    fn reset(&mut self, _kind: ResetKind) {
        let name = self.name.clone();
        *self = Self::new(name);
    }
}
