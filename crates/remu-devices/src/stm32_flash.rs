//! Functional STM32L4 main-flash and FLASH register model.
//!
//! The model deliberately focuses on the software-visible programming path
//! used by STM32Cube and common bare-metal firmware: KEYR unlock, page/mass
//! erase, 64-bit double-word programming, NOR one-to-zero semantics, and the
//! status/control registers at `0x4002_2000`. Operation completion is
//! immediate in the functional simulator; cycle-level busy timing and ECC
//! correction remain outside this slice.

use super::*;

/// STM32L432KC main flash base address.
pub const STM32_FLASH_BASE: u32 = 0x0800_0000;
/// STM32L432KC main flash capacity (256 KiB).
pub const STM32_FLASH_SIZE: usize = 256 * 1024;
/// STM32L4 page size used by the L432KC.
pub const STM32_FLASH_PAGE_SIZE: usize = 0x800;

const FLASH_KEY1: u32 = 0x4567_0123;
const FLASH_KEY2: u32 = 0xcdef_89ab;
const FLASH_OPTKEY1: u32 = 0x0819_2a3b;
const FLASH_OPTKEY2: u32 = 0x4c5d_6e7f;

const STATUS_EOP: u32 = 1 << 0;
const STATUS_OPERR: u32 = 1 << 1;
const STATUS_PROGERR: u32 = 1 << 3;
const STATUS_WRPERR: u32 = 1 << 4;
const STATUS_PGAERR: u32 = 1 << 5;
const STATUS_SIZERR: u32 = 1 << 6;
const STATUS_PGSERR: u32 = 1 << 7;
const STATUS_MISERR: u32 = 1 << 8;
const STATUS_FASTERR: u32 = 1 << 9;
const STATUS_RDERR: u32 = 1 << 14;
const STATUS_OPTVERR: u32 = 1 << 15;
const STATUS_BSY: u32 = 1 << 16;
const STATUS_CLEARABLE: u32 = STATUS_EOP
    | STATUS_OPERR
    | STATUS_PROGERR
    | STATUS_WRPERR
    | STATUS_PGAERR
    | STATUS_SIZERR
    | STATUS_PGSERR
    | STATUS_MISERR
    | STATUS_FASTERR
    | STATUS_RDERR
    | STATUS_OPTVERR;

const CONTROL_PG: u32 = 1 << 0;
const CONTROL_PER: u32 = 1 << 1;
const CONTROL_MER1: u32 = 1 << 2;
const CONTROL_PNB_MASK: u32 = 0x7f << 3;
const CONTROL_STRT: u32 = 1 << 16;
const CONTROL_OPTSTRT: u32 = 1 << 17;
const CONTROL_FSTPG: u32 = 1 << 18;
const CONTROL_EOPIE: u32 = 1 << 24;
const CONTROL_ERRIE: u32 = 1 << 25;
const CONTROL_RDERRIE: u32 = 1 << 26;
const CONTROL_OBL_LAUNCH: u32 = 1 << 27;
const CONTROL_OPTLOCK: u32 = 1 << 30;
const CONTROL_LOCK: u32 = 1 << 31;
const CONTROL_SUPPORTED: u32 = CONTROL_PG
    | CONTROL_PER
    | CONTROL_MER1
    | CONTROL_PNB_MASK
    | CONTROL_STRT
    | CONTROL_OPTSTRT
    | CONTROL_FSTPG
    | CONTROL_EOPIE
    | CONTROL_ERRIE
    | CONTROL_RDERRIE
    | CONTROL_OBL_LAUNCH
    | CONTROL_OPTLOCK
    | CONTROL_LOCK;

/// Named STM32L4 FLASH register identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stm32FlashRegister {
    /// Access control register (`0x00`).
    Acr,
    /// Power-down key register (`0x04`).
    Pdkeyr,
    /// Main flash key register (`0x08`).
    Keyr,
    /// Option-byte key register (`0x0c`).
    Optkeyr,
    /// Status register (`0x10`).
    Sr,
    /// Control register (`0x14`).
    Cr,
    /// ECC register (`0x18`).
    Eccr,
    /// Option register (`0x20`).
    Optr,
    /// Bank-one PCROP start (`0x24`).
    Pcrop1sr,
    /// Bank-one PCROP end (`0x28`).
    Pcrop1er,
    /// Bank-one write-protection area A (`0x2c`).
    Wrp1ar,
    /// Bank-one write-protection area B (`0x30`).
    Wrp1br,
}

impl Stm32FlashRegister {
    fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::Acr,
            0x04 => Self::Pdkeyr,
            0x08 => Self::Keyr,
            0x0c => Self::Optkeyr,
            0x10 => Self::Sr,
            0x14 => Self::Cr,
            0x18 => Self::Eccr,
            0x20 => Self::Optr,
            0x24 => Self::Pcrop1sr,
            0x28 => Self::Pcrop1er,
            0x2c => Self::Wrp1ar,
            0x30 => Self::Wrp1br,
            _ => return None,
        })
    }
}

struct Stm32FlashState {
    bytes: Vec<u8>,
    acr: u32,
    sr: u32,
    cr: u32,
    eccr: u32,
    optr: u32,
    pcrop1sr: u32,
    pcrop1er: u32,
    wrp1ar: u32,
    wrp1br: u32,
    key_stage: u8,
    option_key_stage: u8,
    pending_program: Option<(usize, u32)>,
}

impl Stm32FlashState {
    fn new(size: usize) -> Self {
        assert!(size > 0, "STM32 flash must contain bytes");
        assert_eq!(
            size % STM32_FLASH_PAGE_SIZE,
            0,
            "STM32 flash pages must fit exactly"
        );
        Self {
            bytes: vec![0xff; size],
            // L4 reset state used by the existing startup facade.
            acr: 0x0000_0600,
            sr: 0,
            cr: CONTROL_LOCK | CONTROL_OPTLOCK,
            eccr: 0,
            // Reset option bytes used by the STM32L432 CMSIS/HAL headers.
            optr: 0xffff_f8aa,
            pcrop1sr: 0,
            pcrop1er: 0,
            wrp1ar: 0xffff_00ff,
            wrp1br: 0xffff_00ff,
            key_stage: 0,
            option_key_stage: 0,
            pending_program: None,
        }
    }

    fn unlock_key(&mut self, value: u32) {
        match (self.key_stage, value) {
            (0, FLASH_KEY1) => self.key_stage = 1,
            (1, FLASH_KEY2) => {
                self.key_stage = 0;
                self.cr &= !CONTROL_LOCK;
            }
            _ => self.key_stage = 0,
        }
    }

    fn unlock_option_key(&mut self, value: u32) {
        match (self.option_key_stage, value) {
            (0, FLASH_OPTKEY1) => self.option_key_stage = 1,
            (1, FLASH_OPTKEY2) => {
                self.option_key_stage = 0;
                self.cr &= !CONTROL_OPTLOCK;
            }
            _ => self.option_key_stage = 0,
        }
    }

    fn clear_status(&mut self, value: u32) {
        self.sr &= !(value & STATUS_CLEARABLE);
    }

    fn finish_operation(&mut self) {
        self.sr &= !STATUS_BSY;
        self.sr |= STATUS_EOP;
        self.cr &= !CONTROL_STRT;
    }

    fn erase_page(&mut self) {
        let page =
            usize::try_from((self.cr & CONTROL_PNB_MASK) >> 3).expect("page number fits usize");
        let pages = self.bytes.len() / STM32_FLASH_PAGE_SIZE;
        let Some(start) = page.checked_mul(STM32_FLASH_PAGE_SIZE) else {
            self.sr |= STATUS_SIZERR;
            return;
        };
        if page >= pages {
            self.sr |= STATUS_SIZERR;
            return;
        }
        self.bytes[start..start + STM32_FLASH_PAGE_SIZE].fill(0xff);
    }

    fn start_erase(&mut self) {
        self.pending_program = None;
        self.sr &= !STATUS_EOP;
        self.sr |= STATUS_BSY;
        if self.cr & CONTROL_MER1 != 0 {
            self.bytes.fill(0xff);
        } else if self.cr & CONTROL_PER != 0 {
            self.erase_page();
        } else {
            self.sr |= STATUS_OPERR;
        }
        self.finish_operation();
    }

    fn program_doubleword(&mut self, offset: usize, value: u64) {
        if offset % 8 != 0 {
            self.sr |= STATUS_PGAERR;
            return;
        }
        let Some(destination) = self.bytes.get_mut(offset..offset.saturating_add(8)) else {
            self.sr |= STATUS_SIZERR;
            return;
        };
        for (index, byte) in destination.iter_mut().enumerate() {
            let requested =
                u8::try_from((value >> (index * 8)) & 0xff).expect("masked flash byte fits u8");
            *byte &= requested;
        }
        self.sr |= STATUS_EOP;
    }

    fn program_word(&mut self, offset: usize, value: u32) {
        if offset % 4 != 0 {
            self.sr |= STATUS_PGAERR;
            return;
        }
        match self.pending_program {
            None if offset % 8 == 0 => self.pending_program = Some((offset, value)),
            Some((first_offset, first)) if offset == first_offset + 4 => {
                self.pending_program = None;
                self.program_doubleword(offset - 4, u64::from(first) | (u64::from(value) << 32));
            }
            _ => {
                self.pending_program = None;
                self.sr |= STATUS_PGSERR;
            }
        }
    }

    fn program(&mut self, offset: usize, width: AccessWidth, value: u64) {
        if self.cr & CONTROL_LOCK != 0 {
            return;
        }
        if self.cr & CONTROL_PG == 0 {
            self.sr |= STATUS_PROGERR;
            return;
        }
        self.sr &= !STATUS_EOP;
        self.sr |= STATUS_BSY;
        match width {
            AccessWidth::DoubleWord => self.program_doubleword(offset, value),
            AccessWidth::Word => self.program_word(offset, value as u32),
            AccessWidth::Byte | AccessWidth::HalfWord => self.sr |= STATUS_SIZERR,
        }
        self.sr &= !STATUS_BSY;
    }

    fn reset_controller(&mut self) {
        self.acr = 0x0000_0600;
        self.sr = 0;
        self.cr = CONTROL_LOCK | CONTROL_OPTLOCK;
        self.eccr = 0;
        self.optr = 0xffff_f8aa;
        self.pcrop1sr = 0;
        self.pcrop1er = 0;
        self.wrp1ar = 0xffff_00ff;
        self.wrp1br = 0xffff_00ff;
        self.key_stage = 0;
        self.option_key_stage = 0;
        self.pending_program = None;
    }
}

/// Device-backed STM32L432KC main flash memory.
pub struct Stm32FlashMemory {
    name: String,
    state: Rc<RefCell<Stm32FlashState>>,
}

impl Stm32FlashMemory {
    /// Creates the main flash and its matching FLASH controller device.
    pub fn new(name: impl Into<String>, size: usize) -> (Self, Stm32FlashController) {
        let state = Rc::new(RefCell::new(Stm32FlashState::new(size)));
        let name = name.into();
        (
            Self {
                name: name.clone(),
                state: state.clone(),
            },
            Stm32FlashController {
                name: format!("{name}.controller"),
                state,
            },
        )
    }

    /// Creates an executable alias sharing this flash's contents and control state.
    pub fn alias(&self, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: self.state.clone(),
        }
    }

    fn require_width(width: AccessWidth) -> Result<usize, DeviceError> {
        if !matches!(
            width,
            AccessWidth::Byte | AccessWidth::HalfWord | AccessWidth::Word | AccessWidth::DoubleWord
        ) {
            return Err(DeviceError::new("STM32 flash access width is unsupported"));
        }
        Ok(usize::from(width.bytes()))
    }
}

impl Device for Stm32FlashMemory {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let width = Self::require_width(width)?;
        let offset = usize::try_from(offset)
            .map_err(|_| DeviceError::new("STM32 flash address exceeds host size"))?;
        let state = self.state.borrow();
        let Some(bytes) = state.bytes.get(offset..offset.saturating_add(width)) else {
            return Err(DeviceError::new("STM32 flash read exceeds mapped capacity"));
        };
        Ok(bytes
            .iter()
            .enumerate()
            .fold(0_u64, |value, (index, byte)| {
                value | (u64::from(*byte) << (index * 8))
            }))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        Self::require_width(width)?;
        let offset = usize::try_from(offset)
            .map_err(|_| DeviceError::new("STM32 flash address exceeds host size"))?;
        self.state.borrow_mut().program(offset, width, value);
        Ok(())
    }

    fn load(&mut self, offset: u64, data: &[u8]) -> Result<(), DeviceError> {
        let offset = usize::try_from(offset)
            .map_err(|_| DeviceError::new("STM32 flash load address exceeds host size"))?;
        let mut state = self.state.borrow_mut();
        let end = offset
            .checked_add(data.len())
            .ok_or_else(|| DeviceError::new("STM32 flash load address overflows"))?;
        let Some(destination) = state.bytes.get_mut(offset..end) else {
            return Err(DeviceError::new(
                "STM32 flash image exceeds mapped capacity",
            ));
        };
        destination.copy_from_slice(data);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        // Main flash contents are non-volatile; the controller resets below.
    }
}

/// STM32L432KC FLASH register block at `0x4002_2000`.
pub struct Stm32FlashController {
    name: String,
    state: Rc<RefCell<Stm32FlashState>>,
}

impl Stm32FlashController {
    fn require_register_access(
        offset: u64,
        width: AccessWidth,
    ) -> Result<Stm32FlashRegister, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "STM32 FLASH registers require aligned word accesses",
            ));
        }
        Stm32FlashRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled STM32 FLASH register at offset {offset:#x}"
            ))
        })
    }
}

impl Device for Stm32FlashController {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let register = Self::require_register_access(offset, width)?;
        let state = self.state.borrow();
        let value = match register {
            Stm32FlashRegister::Acr => state.acr,
            Stm32FlashRegister::Pdkeyr | Stm32FlashRegister::Keyr | Stm32FlashRegister::Optkeyr => {
                0
            }
            Stm32FlashRegister::Sr => state.sr,
            Stm32FlashRegister::Cr => state.cr,
            Stm32FlashRegister::Eccr => state.eccr,
            Stm32FlashRegister::Optr => state.optr,
            Stm32FlashRegister::Pcrop1sr => state.pcrop1sr,
            Stm32FlashRegister::Pcrop1er => state.pcrop1er,
            Stm32FlashRegister::Wrp1ar => state.wrp1ar,
            Stm32FlashRegister::Wrp1br => state.wrp1br,
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
        let register = Self::require_register_access(offset, width)?;
        let value = u32::try_from(value).expect("word access value fits u32");
        let mut state = self.state.borrow_mut();
        match register {
            Stm32FlashRegister::Acr => state.acr = value,
            Stm32FlashRegister::Pdkeyr => {}
            Stm32FlashRegister::Keyr => state.unlock_key(value),
            Stm32FlashRegister::Optkeyr => state.unlock_option_key(value),
            Stm32FlashRegister::Sr => state.clear_status(value),
            Stm32FlashRegister::Cr => {
                if state.cr & CONTROL_LOCK == 0 {
                    state.cr = value & CONTROL_SUPPORTED;
                    if state.cr & CONTROL_STRT != 0 {
                        state.start_erase();
                    }
                }
            }
            Stm32FlashRegister::Eccr => state.eccr = value,
            Stm32FlashRegister::Optr => {
                if state.cr & CONTROL_OPTLOCK == 0 {
                    state.optr = value;
                }
            }
            Stm32FlashRegister::Pcrop1sr => {
                if state.cr & CONTROL_OPTLOCK == 0 {
                    state.pcrop1sr = value;
                }
            }
            Stm32FlashRegister::Pcrop1er => {
                if state.cr & CONTROL_OPTLOCK == 0 {
                    state.pcrop1er = value;
                }
            }
            Stm32FlashRegister::Wrp1ar => {
                if state.cr & CONTROL_OPTLOCK == 0 {
                    state.wrp1ar = value;
                }
            }
            Stm32FlashRegister::Wrp1br => {
                if state.cr & CONTROL_OPTLOCK == 0 {
                    state.wrp1br = value;
                }
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset_controller();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unlock(controller: &mut Stm32FlashController) {
        controller
            .write(
                0x08,
                AccessWidth::Word,
                u64::from(FLASH_KEY1),
                SimTime::ZERO,
            )
            .unwrap();
        controller
            .write(
                0x08,
                AccessWidth::Word,
                u64::from(FLASH_KEY2),
                SimTime::ZERO,
            )
            .unwrap();
    }

    #[test]
    fn reset_is_locked_and_doubleword_programming_uses_nor_semantics() {
        let (mut flash, mut controller) = Stm32FlashMemory::new("flash", 0x1000);
        flash.load(0, &[0xff; 8]).unwrap();
        flash
            .write(
                0,
                AccessWidth::DoubleWord,
                0x1122_3344_5566_7788,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            flash
                .read(0, AccessWidth::DoubleWord, SimTime::ZERO)
                .unwrap(),
            u64::MAX
        );

        unlock(&mut controller);
        controller
            .write(
                0x14,
                AccessWidth::Word,
                u64::from(CONTROL_PG),
                SimTime::ZERO,
            )
            .unwrap();
        flash
            .write(
                0,
                AccessWidth::DoubleWord,
                0x1122_3344_5566_7788,
                SimTime::ZERO,
            )
            .unwrap();
        flash
            .write(0, AccessWidth::DoubleWord, u64::MAX, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            flash
                .read(0, AccessWidth::DoubleWord, SimTime::ZERO)
                .unwrap(),
            0x1122_3344_5566_7788
        );
        assert_ne!(
            controller
                .read(0x10, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(STATUS_EOP),
            0
        );
    }

    #[test]
    fn paired_word_programming_and_page_erase_follow_l432_page_size() {
        let (mut flash, mut controller) = Stm32FlashMemory::new("flash", 0x1000);
        flash.load(0, &[0xff; 8]).unwrap();
        unlock(&mut controller);
        controller
            .write(
                0x14,
                AccessWidth::Word,
                u64::from(CONTROL_PG),
                SimTime::ZERO,
            )
            .unwrap();
        flash
            .write(0, AccessWidth::Word, 0xdead_beef, SimTime::ZERO)
            .unwrap();
        flash
            .write(4, AccessWidth::Word, 0xcafe_babe, SimTime::ZERO)
            .unwrap();
        flash.load(STM32_FLASH_PAGE_SIZE as u64, &[0; 4]).unwrap();
        assert_eq!(
            flash
                .read(0, AccessWidth::DoubleWord, SimTime::ZERO)
                .unwrap(),
            0xcafe_babe_dead_beef
        );

        controller
            .write(
                0x14,
                AccessWidth::Word,
                u64::from(CONTROL_PER | (1 << 3) | CONTROL_STRT),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            flash.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xdead_beef
        );
        assert_eq!(
            flash
                .read(
                    STM32_FLASH_PAGE_SIZE as u64,
                    AccessWidth::Word,
                    SimTime::ZERO
                )
                .unwrap(),
            u32::MAX as u64
        );
        assert_ne!(
            controller
                .read(0x10, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(STATUS_EOP),
            0
        );
    }

    #[test]
    fn firmware_load_bypasses_runtime_lock() {
        let (mut flash, _controller) = Stm32FlashMemory::new("flash", 0x1000);
        flash.load(0, &[1, 2, 3, 4]).unwrap();
        assert_eq!(
            flash.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x0403_0201
        );
    }
}
