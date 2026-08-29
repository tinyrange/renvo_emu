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
const CONTROL_BER32: u32 = 1 << 23;
const CONTROL_OBWRE: u32 = 1 << 9;
const CONTROL_ERRIE: u32 = 1 << 10;
const CONTROL_EOPIE: u32 = 1 << 12;
const CONTROL_FWAKEIE: u32 = 1 << 13;
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
    | CONTROL_BUF_RST
    | CONTROL_BER32
    | CONTROL_OBWRE
    | CONTROL_ERRIE
    | CONTROL_EOPIE
    | CONTROL_FWAKEIE;

/// WCH flash-controller generations covered by the functional model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WchFlashVariant {
    /// CH32V003 uses standard half-word programming and 64-byte fast pages.
    Ch32v003,
    /// CH32V006 uses 32-bit buffered programming and 256-byte fast pages.
    Ch32v006,
}

impl WchFlashVariant {
    fn status_reset(self) -> u32 {
        match self {
            Self::Ch32v003 => 1 << 15,
            Self::Ch32v006 => 0x0000_b000,
        }
    }

    fn fast_page_size(self) -> usize {
        match self {
            Self::Ch32v003 => 64,
            Self::Ch32v006 => 256,
        }
    }
}

struct WchFlashState {
    variant: WchFlashVariant,
    bytes: Vec<u8>,
    page_size: usize,
    actlr: u32,
    statr: u32,
    ctlr: u32,
    addr: u32,
    obr: u32,
    wpr: u32,
    key_stage: u8,
    option_key_stage: u8,
    mode_key_stage: u8,
}

impl WchFlashState {
    fn new(size: usize, page_size: usize, variant: WchFlashVariant) -> Self {
        Self {
            variant,
            bytes: vec![0xff; size],
            page_size,
            actlr: 0,
            statr: variant.status_reset(),
            ctlr: CONTROL_LOCK | CONTROL_FLOCK,
            addr: 0,
            obr: 0x03ff_fffe,
            wpr: u32::MAX,
            key_stage: 0,
            option_key_stage: 0,
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

    fn erase_page(&mut self, page_size: usize) {
        let Some(address) = self.normalize_address(self.addr) else {
            self.statr |= STATUS_WRPRTERR;
            return;
        };
        let start = address / page_size * page_size;
        let end = start.saturating_add(page_size).min(self.bytes.len());
        self.bytes[start..end].fill(0xff);
    }

    fn erase_all(&mut self) {
        self.bytes.fill(0xff);
    }

    fn erase_ber32(&mut self) {
        let Some(address) = self.normalize_address(self.addr) else {
            self.statr |= STATUS_WRPRTERR;
            return;
        };
        if address >= 32 * 1024 {
            self.statr |= STATUS_WRPRTERR;
            return;
        }
        let end = (32 * 1024).min(self.bytes.len());
        self.bytes[..end].fill(0xff);
    }

    fn start_operation(&mut self) {
        if self.ctlr & CONTROL_LOCK != 0 {
            return;
        }
        self.statr &= !(STATUS_EOP | STATUS_WRPRTERR);
        self.statr |= STATUS_BSY;
        if self.ctlr & CONTROL_MER != 0 {
            self.erase_all();
        } else if self.variant == WchFlashVariant::Ch32v006 && self.ctlr & CONTROL_BER32 != 0 {
            self.erase_ber32();
        } else if self.ctlr & CONTROL_PER != 0 {
            self.erase_page(self.page_size);
        } else if self.ctlr & CONTROL_PAGE_ER != 0 && self.ctlr & CONTROL_FLOCK == 0 {
            self.erase_page(self.variant.fast_page_size());
        } else if self.ctlr & CONTROL_PAGE_PG != 0 && self.ctlr & CONTROL_FLOCK == 0 {
            // Fast-page data is committed by the functional memory write path.
        } else if self.ctlr & (CONTROL_PAGE_PG | CONTROL_PAGE_ER) != 0 {
            self.statr |= STATUS_WRPRTERR;
        }
        self.complete();
    }

    fn reset_controller(&mut self) {
        self.actlr = 0;
        self.statr = self.variant.status_reset();
        self.ctlr = CONTROL_LOCK | CONTROL_FLOCK;
        self.addr = 0;
        self.key_stage = 0;
        self.option_key_stage = 0;
        self.mode_key_stage = 0;
    }
}

/// Device-backed WCH program flash memory.
///
/// Runtime writes are acknowledged only when the flash controller is unlocked
/// and the target's standard or fast programming mode is enabled. Programming
/// follows NOR semantics (`1` bits can become `0`, but not the reverse); image
/// loading bypasses that rule so ELF and raw firmware artifacts can initialize
/// the target.
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
        Self::new_for_variant(name, size, page_size, WchFlashVariant::Ch32v003)
    }

    /// Creates a target-specific WCH flash memory and controller pair.
    pub fn new_for_variant(
        name: impl Into<String>,
        size: usize,
        page_size: usize,
        variant: WchFlashVariant,
    ) -> (Self, WchFlashController) {
        assert!(size > 0, "WCH flash must contain bytes");
        assert!(page_size > 0, "WCH flash pages must contain bytes");
        let state = Rc::new(RefCell::new(WchFlashState::new(size, page_size, variant)));
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

    fn program_nor(state: &mut WchFlashState, offset: usize, width: usize, value: u64) {
        let Some(bytes) = state.bytes.get_mut(offset..offset.saturating_add(width)) else {
            state.statr |= STATUS_WRPRTERR;
            return;
        };
        for (index, byte) in bytes.iter_mut().enumerate() {
            let requested =
                u8::try_from((value >> (index * 8)) & 0xff).expect("masked flash byte fits u8");
            *byte &= requested;
        }
        state.statr |= STATUS_EOP;
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
        let offset = usize::try_from(offset)
            .map_err(|_| DeviceError::new("WCH flash address exceeds host size"))?;
        let mut state = self.state.borrow_mut();
        let standard = state.variant == WchFlashVariant::Ch32v003
            && state.ctlr & (CONTROL_LOCK | CONTROL_PG) == CONTROL_PG;
        let fast = state.ctlr & (CONTROL_LOCK | CONTROL_FLOCK | CONTROL_PAGE_PG) == CONTROL_PAGE_PG;
        if !standard && !fast {
            return Ok(());
        }
        let expected_width = if standard { 2 } else { 4 };
        if width != expected_width || offset % expected_width != 0 {
            return Err(DeviceError::new(if standard {
                "CH32V003 standard flash programming requires aligned halfword writes"
            } else {
                "WCH fast flash programming requires aligned word writes"
            }));
        };
        Self::program_nor(&mut state, offset, width, value);
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

    fn unlock_key(stage: &mut u8, value: u32) -> bool {
        match (*stage, value) {
            (0, FLASH_KEY1) => {
                *stage = 1;
                false
            }
            (1, FLASH_KEY2) => {
                *stage = 0;
                true
            }
            _ => {
                *stage = 0;
                false
            }
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
            0x28 => 0,
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
            0x04 => {
                if Self::unlock_key(&mut state.key_stage, value) {
                    state.ctlr &= !CONTROL_LOCK;
                }
            }
            0x08 => {
                if state.ctlr & CONTROL_LOCK == 0
                    && Self::unlock_key(&mut state.option_key_stage, value)
                {
                    state.ctlr |= CONTROL_OBWRE;
                }
            }
            0x0c => state.statr &= !(value & (STATUS_EOP | STATUS_WRPRTERR)),
            0x10 => {
                if state.ctlr & CONTROL_LOCK == 0 {
                    let mut supported = CONTROL_SUPPORTED;
                    match state.variant {
                        WchFlashVariant::Ch32v003 => supported &= !CONTROL_BER32,
                        WchFlashVariant::Ch32v006 => supported &= !CONTROL_PG,
                    }
                    state.ctlr = value & supported;
                    if state.ctlr & CONTROL_STRT != 0 {
                        state.start_operation();
                    }
                }
            }
            0x14 => state.addr = value,
            0x1c => state.obr = value,
            0x20 => state.wpr = value,
            0x24 => {
                if state.ctlr & CONTROL_LOCK == 0
                    && Self::unlock_key(&mut state.mode_key_stage, value)
                {
                    state.ctlr &= !CONTROL_FLOCK;
                }
            }
            0x28 => {}
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

    #[test]
    fn ch32v003_standard_programming_rejects_word_accesses() {
        let (mut flash, mut controller) =
            WchFlashMemory::new_for_variant("flash", 2048, 1024, WchFlashVariant::Ch32v003);
        unlock(&mut controller);
        controller
            .write(
                0x10,
                AccessWidth::Word,
                u64::from(CONTROL_PG),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(
            flash
                .write(0, AccessWidth::Word, 0x1234, SimTime::ZERO)
                .is_err()
        );
    }

    #[test]
    fn ch32v006_uses_fast_word_programming_and_256_byte_erase() {
        let (mut flash, mut controller) =
            WchFlashMemory::new_for_variant("flash", 1024, 1024, WchFlashVariant::Ch32v006);
        flash.load(0, &[0xff, 0xff, 0xff, 0xff]).unwrap();
        flash.load(256, &[0, 0]).unwrap();
        unlock(&mut controller);
        controller
            .write(
                0x24,
                AccessWidth::Word,
                u64::from(FLASH_KEY1),
                SimTime::ZERO,
            )
            .unwrap();
        controller
            .write(
                0x24,
                AccessWidth::Word,
                u64::from(FLASH_KEY2),
                SimTime::ZERO,
            )
            .unwrap();
        controller
            .write(
                0x10,
                AccessWidth::Word,
                u64::from(CONTROL_PAGE_PG | CONTROL_BUF_RST),
                SimTime::ZERO,
            )
            .unwrap();
        flash
            .write(0, AccessWidth::Word, 0x1234_5678, SimTime::ZERO)
            .unwrap();
        controller
            .write(
                0x10,
                AccessWidth::Word,
                u64::from(CONTROL_PAGE_PG | CONTROL_BUF_LOAD | CONTROL_STRT),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            flash.read(0, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x1234_5678
        );

        controller
            .write(0x14, AccessWidth::Word, 256, SimTime::ZERO)
            .unwrap();
        controller
            .write(
                0x10,
                AccessWidth::Word,
                u64::from(CONTROL_PAGE_ER | CONTROL_STRT),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            flash
                .read(256, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0xffff
        );
        assert_eq!(
            flash
                .read(512, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0xffff
        );
    }
}
