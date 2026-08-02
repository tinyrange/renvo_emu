use super::*;

const FLASH_KEY1: u32 = 0x4567_0123;
const FLASH_KEY2: u32 = 0xcdef_89ab;

const STATUS_BSY: u32 = 1 << 0;
const STATUS_WRPRTERR: u32 = 1 << 4;
const STATUS_EOP: u32 = 1 << 5;

const CONTROL_PG: u32 = 1 << 0;
const CONTROL_PER: u32 = 1 << 1;
const CONTROL_MER: u32 = 1 << 2;
const CONTROL_OPTPG: u32 = 1 << 4;
const CONTROL_OPTER: u32 = 1 << 5;
const CONTROL_STRT: u32 = 1 << 6;
const CONTROL_LOCK: u32 = 1 << 7;
const CONTROL_FLOCK: u32 = 1 << 15;
const CONTROL_PAGE_PG: u32 = 1 << 16;
const CONTROL_PAGE_ER: u32 = 1 << 17;
const CONTROL_BUF_LOAD: u32 = 1 << 18;
const CONTROL_BUF_RST: u32 = 1 << 19;
const CONTROL_SUPPORTED: u32 = CONTROL_PG
    | CONTROL_PER
    | CONTROL_MER
    | CONTROL_OPTPG
    | CONTROL_OPTER
    | CONTROL_STRT
    | CONTROL_LOCK
    | CONTROL_FLOCK
    | CONTROL_PAGE_PG
    | CONTROL_PAGE_ER
    | CONTROL_BUF_LOAD
    | CONTROL_BUF_RST;

struct WchFlashState {
    bytes: Vec<u8>,
    page_size: usize,
    actlr: u32,
    statr: u32,
    ctlr: u32,
    addr: u32,
    obr: u32,
    wpr: u32,
    key_stage: u8,
    mode_key_stage: u8,
}

impl WchFlashState {
    fn new(size: usize, page_size: usize) -> Self {
        Self {
            bytes: vec![0xff; size],
            page_size,
            actlr: 0,
            statr: 0,
            ctlr: CONTROL_LOCK,
            addr: 0,
            obr: 0x03ff_fffe,
            wpr: u32::MAX,
            key_stage: 0,
            mode_key_stage: 0,
        }
    }

    fn normalize_address(&self, address: u32) -> Option<usize> {
        let address = if address >= 0x0800_0000 {
            address.checked_sub(0x0800_0000)?
        } else {
            address
        };
        let address = usize::try_from(address).ok()?;
        (address < self.bytes.len()).then_some(address)
    }

    fn complete(&mut self) {
        self.statr &= !STATUS_BSY;
        self.statr |= STATUS_EOP;
        self.ctlr &= !CONTROL_STRT;
    }

    fn erase_page(&mut self) {
        let Some(address) = self.normalize_address(self.addr) else {
            self.statr |= STATUS_WRPRTERR;
            return;
        };
        let start = address / self.page_size * self.page_size;
        let end = start.saturating_add(self.page_size).min(self.bytes.len());
        self.bytes[start..end].fill(0xff);
    }

    fn erase_all(&mut self) {
        self.bytes.fill(0xff);
    }

    fn start_operation(&mut self) {
        if self.ctlr & CONTROL_LOCK != 0 {
            return;
        }
        self.statr &= !(STATUS_EOP | STATUS_WRPRTERR);
        self.statr |= STATUS_BSY;
        if self.ctlr & CONTROL_MER != 0 {
            self.erase_all();
        } else if self.ctlr & (CONTROL_PER | CONTROL_PAGE_ER) != 0 {
            self.erase_page();
        }
        self.complete();
    }

    fn reset_controller(&mut self) {
        self.actlr = 0;
        self.statr = 0;
        self.ctlr = CONTROL_LOCK;
        self.addr = 0;
        self.key_stage = 0;
        self.mode_key_stage = 0;
    }
}

/// Device-backed WCH program flash memory.
///
/// Runtime writes are acknowledged only when the flash controller is unlocked
/// and `PG` is set. Programming follows NOR semantics (`1` bits can become
/// `0`, but not the reverse); image loading bypasses that rule so ELF and raw
/// firmware artifacts can initialize the target.
pub struct WchFlashMemory {
    name: String,
    state: Rc<RefCell<WchFlashState>>,
}

impl WchFlashMemory {
    /// Creates a WCH flash memory and its controller sharing one backing store.
    pub fn new(
        name: impl Into<String>,
        size: usize,
        page_size: usize,
    ) -> (Self, WchFlashController) {
        assert!(size > 0, "WCH flash must contain bytes");
        assert!(page_size > 0, "WCH flash pages must contain bytes");
        let state = Rc::new(RefCell::new(WchFlashState::new(size, page_size)));
        let name = name.into();
        (
            Self {
                name: name.clone(),
                state: state.clone(),
            },
            WchFlashController {
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
        let bytes = usize::from(width.bytes());
        if !matches!(
            width,
            AccessWidth::Byte | AccessWidth::HalfWord | AccessWidth::Word
        ) {
            return Err(DeviceError::new(
                "WCH flash supports byte, halfword, and word accesses",
            ));
        }
        Ok(bytes)
    }
}

impl Device for WchFlashMemory {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let width = Self::require_width(width)?;
        let offset = usize::try_from(offset)
            .map_err(|_| DeviceError::new("WCH flash address exceeds host size"))?;
        let state = self.state.borrow();
        let Some(bytes) = state.bytes.get(offset..offset.saturating_add(width)) else {
            return Err(DeviceError::new("WCH flash read exceeds the mapped image"));
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
        let width = Self::require_width(width)?;
        if width == 1 || offset % u64::try_from(width).expect("flash width fits u64") != 0 {
            return Err(DeviceError::new(
                "WCH flash programming requires aligned halfword or word writes",
            ));
        }
        let offset = usize::try_from(offset)
            .map_err(|_| DeviceError::new("WCH flash address exceeds host size"))?;
        let mut state = self.state.borrow_mut();
        if state.ctlr & (CONTROL_LOCK | CONTROL_PG) != CONTROL_PG {
            return Ok(());
        }
        let Some(bytes) = state.bytes.get_mut(offset..offset.saturating_add(width)) else {
            state.statr |= STATUS_WRPRTERR;
            return Ok(());
        };
        for (index, byte) in bytes.iter_mut().enumerate() {
            let requested =
                u8::try_from((value >> (index * 8)) & 0xff).expect("masked flash byte fits u8");
            *byte &= requested;
        }
        state.statr |= STATUS_EOP;
        Ok(())
    }

    fn load(&mut self, offset: u64, data: &[u8]) -> Result<(), DeviceError> {
        let offset = usize::try_from(offset)
            .map_err(|_| DeviceError::new("WCH flash load address exceeds host size"))?;
        let mut state = self.state.borrow_mut();
        let end = offset
            .checked_add(data.len())
            .ok_or_else(|| DeviceError::new("WCH flash load address overflows"))?;
        let Some(destination) = state.bytes.get_mut(offset..end) else {
            return Err(DeviceError::new("WCH flash image exceeds the mapped image"));
        };
        destination.copy_from_slice(data);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        // Program contents survive a device reset; the controller resets below.
    }
}

/// WCH flash controller register block at `0x4002_2000`.
pub struct WchFlashController {
    name: String,
    state: Rc<RefCell<WchFlashState>>,
}

impl WchFlashController {
    fn require_register_access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "WCH flash registers require aligned word accesses",
            ));
        }
        Ok(())
    }

    fn unlock_key(state: &mut WchFlashState, value: u32, mode: bool) {
        let stage = if mode {
            &mut state.mode_key_stage
        } else {
            &mut state.key_stage
        };
        match (*stage, value) {
            (0, FLASH_KEY1) => *stage = 1,
            (1, FLASH_KEY2) => {
                *stage = 0;
                if mode {
                    state.ctlr &= !CONTROL_FLOCK;
                } else {
                    state.ctlr &= !CONTROL_LOCK;
                }
            }
            _ => *stage = 0,
        }
    }
}

impl Device for WchFlashController {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::require_register_access(offset, width)?;
        let state = self.state.borrow();
        let value = match offset {
            0x00 => state.actlr,
            0x04 | 0x08 | 0x24 => 0,
            0x0c => state.statr,
            0x10 => state.ctlr,
            0x14 => state.addr,
            0x18 => 0,
            0x1c => state.obr,
            0x20 => state.wpr,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH flash read at offset {offset:#x}"
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
        Self::require_register_access(offset, width)?;
        let value = u32::try_from(value).expect("word access value fits u32");
        let mut state = self.state.borrow_mut();
        match offset {
            0x00 => state.actlr = value,
            0x04 => Self::unlock_key(&mut state, value, false),
            0x08 => Self::unlock_key(&mut state, value, false),
            0x0c => state.statr &= !(value & (STATUS_EOP | STATUS_WRPRTERR)),
            0x10 => {
                if state.ctlr & CONTROL_LOCK == 0 {
                    state.ctlr = value & CONTROL_SUPPORTED;
                    if state.ctlr & CONTROL_STRT != 0 {
                        state.start_operation();
                    }
                }
            }
            0x14 => state.addr = value,
            0x1c => state.obr = value,
            0x20 => state.wpr = value,
            0x24 => Self::unlock_key(&mut state, value, true),
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled WCH flash write at offset {offset:#x}"
                )));
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

    fn unlock(controller: &mut WchFlashController) {
        controller
            .write(
                0x04,
                AccessWidth::Word,
                u64::from(FLASH_KEY1),
                SimTime::ZERO,
            )
            .unwrap();
        controller
            .write(
                0x04,
                AccessWidth::Word,
                u64::from(FLASH_KEY2),
                SimTime::ZERO,
            )
            .unwrap();
    }

    #[test]
    fn programming_requires_unlock_and_preserves_nor_one_to_zero_rules() {
        let (mut flash, mut controller) = WchFlashMemory::new("flash", 2048, 1024);
        flash.load(0, &[0xff, 0xff, 0xff, 0xff]).unwrap();
        flash
            .write(0, AccessWidth::HalfWord, 0x1234, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            flash.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(u32::MAX)
        );

        unlock(&mut controller);
        controller
            .write(
                0x10,
                AccessWidth::Word,
                u64::from(CONTROL_PG),
                SimTime::ZERO,
            )
            .unwrap();
        flash
            .write(0, AccessWidth::HalfWord, 0x1234, SimTime::ZERO)
            .unwrap();
        flash
            .write(0, AccessWidth::HalfWord, 0xffff, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            flash.read(0, AccessWidth::HalfWord, SimTime::ZERO).unwrap(),
            0x1234
        );
        assert_ne!(
            controller
                .read(0x0c, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(STATUS_EOP),
            0
        );
    }

    #[test]
    fn page_erase_uses_the_one_kilobyte_boundary_and_clears_status() {
        let (mut flash, mut controller) = WchFlashMemory::new("flash", 2048, 1024);
        flash.load(0, &[0, 0]).unwrap();
        flash.load(1024, &[0, 0]).unwrap();
        unlock(&mut controller);
        controller
            .write(0x14, AccessWidth::Word, 1020, SimTime::ZERO)
            .unwrap();
        controller
            .write(
                0x10,
                AccessWidth::Word,
                u64::from(CONTROL_PER | CONTROL_STRT),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            flash.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from(u32::MAX)
        );
        assert_eq!(
            flash
                .read(1024, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0
        );
        controller
            .write(
                0x0c,
                AccessWidth::Word,
                u64::from(STATUS_EOP),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            controller
                .read(0x0c, AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(STATUS_EOP),
            0
        );
    }
}
