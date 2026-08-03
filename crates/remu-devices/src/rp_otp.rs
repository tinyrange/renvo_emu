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
const SBPI_STATUS_INSTR_MISS: u32 = 1 << 8;
const SBPI_STATUS_WC_MASK: u32 =
    SBPI_STATUS_RDATA_VALID | SBPI_STATUS_INSTR_DONE | SBPI_STATUS_INSTR_MISS;
const SBPI_INSTR_EXEC: u32 = 1 << 30;
const SBPI_INSTR_CONFIG_MASK: u32 = 0x3fff_ffff;
const BIST_CNT_CLR: u32 = 1 << 29;
const BIST_CNT_ENA: u32 = 1 << 28;
const BIST_CNT_MAX_MASK: u32 = 0x0fff_0000;
const BIST_CONFIG_MASK: u32 = BIST_CNT_ENA | BIST_CNT_MAX_MASK;
const BIST_COUNT_MASK: u32 = 0x0000_1fff;
const DBG_ROSC_UP_SEEN: u32 = 1 << 2;
const DEBUGEN_MASK: u32 = 0x10f;
const BOOTDIS_NEXT: u32 = 1 << 1;
const BOOTDIS_NOW: u32 = 1;
const INTR_MASK: u32 = 0x1f;
const INTR_WC_MASK: u32 = 0x1e;

/// RP2350 OTP control-register offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rp2350OtpRegister {
    /// Software lock register for one of the 64 OTP pages.
    SwLock(u8),
    /// SBPI instruction dispatch register.
    SbpiInstr,
    /// SBPI write-payload word.
    SbpiWdata(u8),
    /// SBPI read-payload word.
    SbpiRdata(u8),
    /// SBPI status register.
    SbpiStatus,
    /// User-interface power and data-read controls.
    Usr,
    /// OTP power-on state-machine debug register.
    Dbg,
    /// Built-in self-test control and counters.
    Bist,
    /// Certificate-key write-only word.
    CrtKeyW(u8),
    /// Read-only critical boot flags.
    Critical,
    /// Read-only enrolled-key status.
    KeyValid,
    /// Debug feature enables.
    DebugEn,
    /// Monotonic debug-enable locks.
    DebugEnLock,
    /// Architecture selection for the two cores.
    ArchSel,
    /// Sampled architecture-selection status.
    ArchSelStatus,
    /// Boot-vector disable controls.
    BootDis,
    /// Raw interrupt status.
    Intr,
    /// Interrupt enables.
    Inte,
    /// Interrupt force bits.
    Intf,
    /// Masked interrupt status.
    Ints,
}

impl TryFrom<u64> for Rp2350OtpRegister {
    type Error = DeviceError;

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        let register = match offset {
            0x000..=0x0fc if offset % 4 == 0 => {
                Self::SwLock(u8::try_from(offset / 4).expect("OTP lock index fits"))
            }
            0x100 => Self::SbpiInstr,
            0x104..=0x110 if (offset - 0x104) % 4 == 0 => {
                Self::SbpiWdata(u8::try_from((offset - 0x104) / 4).expect("OTP word index fits"))
            }
            0x114..=0x120 if (offset - 0x114) % 4 == 0 => {
                Self::SbpiRdata(u8::try_from((offset - 0x114) / 4).expect("OTP word index fits"))
            }
            0x124 => Self::SbpiStatus,
            0x128 => Self::Usr,
            0x12c => Self::Dbg,
            0x134 => Self::Bist,
            0x138..=0x144 if (offset - 0x138) % 4 == 0 => {
                Self::CrtKeyW(u8::try_from((offset - 0x138) / 4).expect("OTP key index fits"))
            }
            0x148 => Self::Critical,
            0x14c => Self::KeyValid,
            0x150 => Self::DebugEn,
            0x154 => Self::DebugEnLock,
            0x158 => Self::ArchSel,
            0x15c => Self::ArchSelStatus,
            0x160 => Self::BootDis,
            0x164 => Self::Intr,
            0x168 => Self::Inte,
            0x16c => Self::Intf,
            0x170 => Self::Ints,
            _ => {
                return Err(DeviceError::new(format!(
                    "invalid RP2350 OTP register offset {offset:#x}"
                )));
            }
        };
        Ok(register)
    }
}

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
        let value = match Rp2350OtpRegister::try_from(register)? {
            Rp2350OtpRegister::SwLock(index) => self.locks[usize::from(index)],
            Rp2350OtpRegister::SbpiInstr => self.sbpi_instr,
            Rp2350OtpRegister::SbpiWdata(index) => self.sbpi_wdata[usize::from(index)],
            Rp2350OtpRegister::SbpiRdata(index) => {
                let index = usize::from(index);
                let value = self.sbpi_rdata[index];
                self.sbpi_rdata[index] = 0;
                if self.sbpi_rdata.iter().all(|word| *word == 0) {
                    self.sbpi_status &= !SBPI_STATUS_RDATA_VALID;
                }
                value
            }
            Rp2350OtpRegister::SbpiStatus => self.sbpi_status,
            Rp2350OtpRegister::Usr => self.usr,
            Rp2350OtpRegister::Dbg => self.dbg,
            Rp2350OtpRegister::Bist => self.bist,
            Rp2350OtpRegister::CrtKeyW(_) => 0,
            Rp2350OtpRegister::Critical => self.critical,
            Rp2350OtpRegister::KeyValid => self.key_valid,
            Rp2350OtpRegister::DebugEn => self.debugen,
            Rp2350OtpRegister::DebugEnLock => self.debugen_lock,
            Rp2350OtpRegister::ArchSel => self.archsel,
            Rp2350OtpRegister::ArchSelStatus => self.archsel_status,
            Rp2350OtpRegister::BootDis => self.bootdis,
            Rp2350OtpRegister::Intr => self.intr,
            Rp2350OtpRegister::Inte => self.inte,
            Rp2350OtpRegister::Intf => self.intf,
            Rp2350OtpRegister::Ints => (self.intr | self.intf) & self.inte,
        };
        Ok(value)
    }

    fn write_control(&mut self, register: u64, alias: u64, value: u32) -> Result<(), DeviceError> {
        match Rp2350OtpRegister::try_from(register)? {
            Rp2350OtpRegister::SwLock(index) => {
                let index = usize::from(index);
                // Lock states only advance until reset. Atomic clear/XOR aliases cannot
                // reopen a page, matching the fuse-backed lock shim.
                self.locks[index] |= value & 0x0f;
            }
            Rp2350OtpRegister::SbpiInstr => {
                self.sbpi_instr =
                    atomic_update(self.sbpi_instr, alias, value)? & SBPI_INSTR_CONFIG_MASK;
                if value & SBPI_INSTR_EXEC != 0 {
                    self.sbpi_status |= SBPI_STATUS_INSTR_DONE;
                    self.sbpi_status &= !SBPI_STATUS_RDATA_VALID;
                }
            }
            Rp2350OtpRegister::SbpiWdata(index) => {
                let index = usize::from(index);
                self.sbpi_wdata[index] = atomic_update(self.sbpi_wdata[index], alias, value)?;
            }
            Rp2350OtpRegister::SbpiRdata(_) => {
                return Err(DeviceError::new("RP2350 OTP SBPI read data is read-only"));
            }
            Rp2350OtpRegister::SbpiStatus => {
                self.sbpi_status &= !(value & SBPI_STATUS_WC_MASK);
            }
            Rp2350OtpRegister::Usr => {
                self.usr = atomic_update(self.usr, alias, value)? & 0x11;
            }
            Rp2350OtpRegister::Dbg => {
                self.dbg &= !(value & DBG_ROSC_UP_SEEN);
            }
            Rp2350OtpRegister::Bist => {
                let config = atomic_update(
                    self.bist & BIST_CONFIG_MASK,
                    alias,
                    value & BIST_CONFIG_MASK,
                )?;
                self.bist = (self.bist & !BIST_CONFIG_MASK) | (config & BIST_CONFIG_MASK);
                if value & BIST_CNT_CLR != 0 {
                    self.bist &= !BIST_COUNT_MASK;
                }
            }
            Rp2350OtpRegister::CrtKeyW(_) => {
                // Certificate key registers are write-only. Retain no secret material.
            }
            Rp2350OtpRegister::Critical | Rp2350OtpRegister::KeyValid => {
                return Err(DeviceError::new("RP2350 OTP register is read-only"));
            }
            Rp2350OtpRegister::DebugEn => {
                let updated = atomic_update(self.debugen, alias, value)? & DEBUGEN_MASK;
                self.debugen = (self.debugen & self.debugen_lock) | (updated & !self.debugen_lock);
            }
            Rp2350OtpRegister::DebugEnLock => {
                self.debugen_lock |= value & DEBUGEN_MASK;
            }
            Rp2350OtpRegister::ArchSel => {
                self.archsel = atomic_update(self.archsel, alias, value)? & 3;
            }
            Rp2350OtpRegister::ArchSelStatus | Rp2350OtpRegister::Ints => {
                return Err(DeviceError::new("RP2350 OTP register is read-only"));
            }
            Rp2350OtpRegister::BootDis => {
                self.bootdis |= value & BOOTDIS_NEXT;
                if value & BOOTDIS_NOW != 0 {
                    self.bootdis &= !BOOTDIS_NOW;
                }
            }
            Rp2350OtpRegister::Intr => self.intr &= !(value & INTR_WC_MASK),
            Rp2350OtpRegister::Inte => {
                self.inte = atomic_update(self.inte, alias, value)? & INTR_MASK
            }
            Rp2350OtpRegister::Intf => {
                self.intf = atomic_update(self.intf, alias, value)? & INTR_MASK
            }
        }
        self.archsel_status = self.archsel;
        Ok(())
    }

    fn read_data(&self, offset: u64) -> Result<u32, DeviceError> {
        if self.usr & (USR_DCTRL | (1 << 4)) != USR_DCTRL {
            return Err(DeviceError::new(
                "RP2350 OTP data reads disabled by USR.DCTRL or USR.PD",
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
