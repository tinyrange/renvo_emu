use super::{AccessWidth, Device, DeviceError, Rc, RefCell, ResetKind, SimTime};
use remu_core::AccessKind;
use std::collections::BTreeSet;

// Generated from ESP-IDF extmem_reg.h at f992ff36f68a783d786d83178e5f85e9a9c76ead.
// Header SHA-256: 1867236e60642887d3b96e8c364e25f404c9750b4b5ac87508a663ac3edf512b.

#[derive(Clone, Copy)]
struct RegisterSpec {
    offset: u16,
    reset: u32,
    read_mask: u32,
    write_mask: u32,
}

impl RegisterSpec {
    const fn new(offset: u16, reset: u32, read_mask: u32, write_mask: u32) -> Self {
        Self {
            offset,
            reset,
            read_mask,
            write_mask,
        }
    }
}

const REGISTER_SPECS: [RegisterSpec; 95] = [
    RegisterSpec::new(0x000, 0x00000000, 0x0000001d, 0x0000001d),
    RegisterSpec::new(0x004, 0x00000003, 0x00000003, 0x00000003),
    RegisterSpec::new(0x008, 0x00000005, 0x00000007, 0x00000007),
    RegisterSpec::new(0x00c, 0x00000000, 0x00000003, 0x00000003),
    RegisterSpec::new(0x010, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x014, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x018, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x01c, 0x00000004, 0x00000007, 0x00000003),
    RegisterSpec::new(0x020, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x024, 0x00000000, 0x0000ffff, 0x0000ffff),
    RegisterSpec::new(0x028, 0x00000001, 0x0000000f, 0x00000007),
    RegisterSpec::new(0x02c, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x030, 0x00000000, 0x007fffff, 0x007fffff),
    RegisterSpec::new(0x034, 0x00000002, 0x00000003, 0x00000001),
    RegisterSpec::new(0x038, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x03c, 0x00000000, 0x0000ffff, 0x0000ffff),
    RegisterSpec::new(0x040, 0x00000002, 0x00000007, 0x00000005),
    RegisterSpec::new(0x044, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x048, 0x00000000, 0x0000ffff, 0x0000ffff),
    RegisterSpec::new(0x04c, 0x00000008, 0x000003ff, 0x000003f7),
    RegisterSpec::new(0x050, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x054, 0x00000000, 0x07ffffff, 0x07ffffff),
    RegisterSpec::new(0x058, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x05c, 0x00000000, 0x07ffffff, 0x07ffffff),
    RegisterSpec::new(0x060, 0x00000000, 0x0000000f, 0x0000000f),
    RegisterSpec::new(0x064, 0x00000003, 0x00000003, 0x00000003),
    RegisterSpec::new(0x068, 0x00000005, 0x00000007, 0x00000007),
    RegisterSpec::new(0x06c, 0x00000000, 0x00000003, 0x00000003),
    RegisterSpec::new(0x070, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x074, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x078, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x07c, 0x00000004, 0x00000007, 0x00000003),
    RegisterSpec::new(0x080, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x084, 0x00000000, 0x0000ffff, 0x0000ffff),
    RegisterSpec::new(0x088, 0x00000001, 0x00000003, 0x00000001),
    RegisterSpec::new(0x08c, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x090, 0x00000000, 0x007fffff, 0x007fffff),
    RegisterSpec::new(0x094, 0x00000002, 0x00000007, 0x00000005),
    RegisterSpec::new(0x098, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x09c, 0x00000000, 0x0000ffff, 0x0000ffff),
    RegisterSpec::new(0x0a0, 0x00000008, 0x000003ff, 0x000003f7),
    RegisterSpec::new(0x0a4, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x0a8, 0x00000000, 0x07ffffff, 0x07ffffff),
    RegisterSpec::new(0x0ac, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x0b0, 0x00000000, 0x07ffffff, 0x07ffffff),
    RegisterSpec::new(0x0b4, 0x44000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x0b8, 0x47ffffff, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x0bc, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x0c0, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x0c4, 0x00000000, 0x00000003, 0x00000003),
    RegisterSpec::new(0x0c8, 0x00000000, 0xffffffff, 0x00000000),
    RegisterSpec::new(0x0cc, 0x00000000, 0xffffffff, 0x00000000),
    RegisterSpec::new(0x0d0, 0x00000000, 0xffffffff, 0x00000000),
    RegisterSpec::new(0x0d4, 0x00000000, 0xffffffff, 0x00000000),
    RegisterSpec::new(0x0d8, 0x00000000, 0xffffffff, 0x00000000),
    RegisterSpec::new(0x0dc, 0x00000000, 0x000001ff, 0x000001ff),
    RegisterSpec::new(0x0e0, 0x00000000, 0x000001ff, 0x000001ff),
    RegisterSpec::new(0x0e4, 0x00000000, 0x00000fff, 0x00000000),
    RegisterSpec::new(0x0e8, 0x00000000, 0x0000001f, 0x0000001f),
    RegisterSpec::new(0x0ec, 0x00000000, 0x0000001f, 0x0000001f),
    RegisterSpec::new(0x0f0, 0x00000000, 0x0000001f, 0x00000000),
    RegisterSpec::new(0x0f4, 0x00000000, 0x0000001f, 0x0000001f),
    RegisterSpec::new(0x0f8, 0x00000000, 0x0000001f, 0x0000001f),
    RegisterSpec::new(0x0fc, 0x00000000, 0x0000001f, 0x00000000),
    RegisterSpec::new(0x100, 0x00000000, 0x0000007f, 0x00000000),
    RegisterSpec::new(0x104, 0xffffffff, 0xffffffff, 0x00000000),
    RegisterSpec::new(0x108, 0x00000000, 0x0000007f, 0x00000000),
    RegisterSpec::new(0x10c, 0xffffffff, 0xffffffff, 0x00000000),
    RegisterSpec::new(0x110, 0x00000000, 0x0000007f, 0x00000000),
    RegisterSpec::new(0x114, 0xffffffff, 0xffffffff, 0x00000000),
    RegisterSpec::new(0x118, 0x00000000, 0x0000007f, 0x00000000),
    RegisterSpec::new(0x11c, 0xffffffff, 0xffffffff, 0x00000000),
    RegisterSpec::new(0x120, 0x00000000, 0x000fffff, 0x00000000),
    RegisterSpec::new(0x124, 0x00000000, 0xffffffff, 0x00000000),
    RegisterSpec::new(0x128, 0x00000000, 0x00000003, 0x00000003),
    RegisterSpec::new(0x12c, 0x00000005, 0x00000007, 0x00000007),
    RegisterSpec::new(0x130, 0x00001001, 0x00ffffff, 0x00000000),
    RegisterSpec::new(0x134, 0x00000000, 0x00000003, 0x00000003),
    RegisterSpec::new(0x138, 0x00000007, 0x00000007, 0x00000007),
    RegisterSpec::new(0x13c, 0x00000000, 0x00000001, 0x00000001),
    RegisterSpec::new(0x140, 0x00000000, 0x0000003f, 0x00000036),
    RegisterSpec::new(0x144, 0x00000000, 0x0000003f, 0x00000036),
    RegisterSpec::new(0x148, 0x00000000, 0x00ffffff, 0x00ffffff),
    RegisterSpec::new(0x14c, 0x00000007, 0x00000007, 0x00000007),
    RegisterSpec::new(0x150, 0x00000004, 0x00000007, 0x00000003),
    RegisterSpec::new(0x154, 0x00000004, 0x00000007, 0x00000003),
    RegisterSpec::new(0x158, 0x00000001, 0x00000001, 0x00000001),
    RegisterSpec::new(0x15c, 0x00000001, 0x00000001, 0x00000001),
    RegisterSpec::new(0x160, 0x00000000, 0x00000001, 0x00000001),
    RegisterSpec::new(0x164, 0x00000001, 0x00000001, 0x00000001),
    RegisterSpec::new(0x180, 0x00000000, 0x00000003, 0x00000003),
    RegisterSpec::new(0x184, 0x00000000, 0x00000007, 0x00000007),
    RegisterSpec::new(0x188, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x18c, 0x00000000, 0xffffffff, 0xffffffff),
    RegisterSpec::new(0x3fc, 0x02012310, 0x0fffffff, 0x0fffffff),
];

fn spec_for_offset(offset: u64) -> Option<(usize, RegisterSpec)> {
    let offset = u16::try_from(offset).ok()?;
    REGISTER_SPECS
        .binary_search_by_key(&offset, |spec| spec.offset)
        .ok()
        .map(|index| (index, REGISTER_SPECS[index]))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CacheKind {
    Data,
    Instruction,
}

impl CacheKind {
    const fn control(self) -> u64 {
        match self {
            Self::Data => 0x000,
            Self::Instruction => 0x060,
        }
    }

    const fn bus_control(self) -> u64 {
        match self {
            Self::Data => 0x004,
            Self::Instruction => 0x064,
        }
    }

    const fn block_size(self, control: u32) -> u32 {
        match self {
            Self::Data => 16 << ((control >> 3) & 3),
            Self::Instruction => 16 << ((control >> 2) & 3),
        }
    }
}

struct Esp32S3ExtmemState {
    registers: [u32; REGISTER_SPECS.len()],
    tags: [BTreeSet<u32>; 2],
    locked: [BTreeSet<u32>; 2],
}

impl Esp32S3ExtmemState {
    fn new() -> Self {
        let mut state = Self {
            registers: [0; REGISTER_SPECS.len()],
            tags: [BTreeSet::new(), BTreeSet::new()],
            locked: [BTreeSet::new(), BTreeSet::new()],
        };
        for (index, spec) in REGISTER_SPECS.iter().enumerate() {
            state.registers[index] = spec.reset;
        }
        state
    }

    fn register(&self, offset: u64) -> u32 {
        spec_for_offset(offset)
            .map(|(index, _)| self.registers[index])
            .unwrap_or(0)
    }

    fn set_register(&mut self, offset: u64, value: u32) {
        if let Some((index, _)) = spec_for_offset(offset) {
            self.registers[index] = value;
        }
    }

    fn cache_index(kind: CacheKind) -> usize {
        usize::from(kind == CacheKind::Instruction)
    }

    fn enabled(&self, kind: CacheKind) -> bool {
        self.register(0x164) & 1 != 0
            && self.register(kind.control()) & 1 != 0
            && self.register(kind.bus_control()) != 3
    }

    fn block_address(&self, kind: CacheKind, address: u32) -> u32 {
        let size = kind.block_size(self.register(kind.control()));
        address & !(size - 1)
    }

    fn cache_range(&mut self, kind: CacheKind, address: u32, blocks: u32, present: bool) {
        let size = kind.block_size(self.register(kind.control()));
        let index = Self::cache_index(kind);
        for block in 0..blocks.min(1 << 20) {
            let address = address.wrapping_add(block.saturating_mul(size)) & !(size - 1);
            if present {
                self.tags[index].insert(address);
            } else if !self.locked[index].contains(&address) {
                self.tags[index].remove(&address);
            }
        }
    }

    fn lock_range(&mut self, kind: CacheKind, address: u32, blocks: u32, lock: bool) {
        let size = kind.block_size(self.register(kind.control()));
        let index = Self::cache_index(kind);
        for block in 0..blocks.min(1 << 20) {
            let address = address.wrapping_add(block.saturating_mul(size)) & !(size - 1);
            if lock {
                self.tags[index].insert(address);
                self.locked[index].insert(address);
            } else {
                self.locked[index].remove(&address);
            }
        }
    }

    fn illegal(&mut self, bit: u8) {
        self.set_register(0x0e4, self.register(0x0e4) | (1 << bit));
    }

    fn complete_lock(&mut self, kind: CacheKind, value: u32) {
        let (control, address, size) = match kind {
            CacheKind::Data => (0x01c, 0x020, 0x024),
            CacheKind::Instruction => (0x07c, 0x080, 0x084),
        };
        if value & 3 != 0 {
            self.lock_range(
                kind,
                self.register(address),
                self.register(size),
                value & 1 != 0,
            );
            self.set_register(control, 4);
        }
    }

    fn complete_sync(&mut self, kind: CacheKind, value: u32) {
        let (control, address, size, command, illegal_bit, status_bit) = match kind {
            CacheKind::Data => (0x028, 0x02c, 0x030, 7, 2, 3),
            CacheKind::Instruction => (0x088, 0x08c, 0x090, 1, 0, 0),
        };
        if value & command == 0 {
            return;
        }
        let start = self.register(address);
        let blocks = self.register(size);
        if !self.enabled(kind) || blocks == 0 {
            self.illegal(illegal_bit);
        } else if kind == CacheKind::Instruction || value & 1 != 0 {
            self.cache_range(kind, start, blocks, false);
        }
        self.set_register(control, if kind == CacheKind::Data { 8 } else { 2 });
        let status = self.register(0x144);
        self.set_register(0x144, status | (1 << status_bit));
    }

    fn complete_preload(&mut self, kind: CacheKind, value: u32) {
        if value & 1 == 0 {
            return;
        }
        let (control, address, size, illegal_bit, status_bit) = match kind {
            CacheKind::Data => (0x040, 0x044, 0x048, 3, 3),
            CacheKind::Instruction => (0x094, 0x098, 0x09c, 1, 0),
        };
        let start = self.register(address);
        let blocks = self.register(size);
        if !self.enabled(kind) || blocks == 0 {
            self.illegal(illegal_bit);
        } else {
            self.cache_range(kind, start, blocks, true);
        }
        self.set_register(control, (value & 4) | 2);
        let status = self.register(0x140);
        self.set_register(0x140, status | (1 << status_bit));
    }

    fn complete_occupy(&mut self, value: u32) {
        if value & 1 != 0 {
            let address = self.register(0x038);
            let blocks = self.register(0x03c);
            if !self.enabled(CacheKind::Data) || blocks == 0 {
                self.illegal(6);
            } else {
                self.cache_range(CacheKind::Data, address, blocks, true);
            }
            self.set_register(0x034, 2);
        }
    }

    fn observe_access(&mut self, core: u8, address: u32, access: AccessKind) -> bool {
        let kind = if access == AccessKind::Execute {
            CacheKind::Instruction
        } else {
            CacheKind::Data
        };
        let cached = matches!(address, 0x3c00_0000..=0x3dff_ffff | 0x4200_0000..=0x43ff_ffff);
        if !cached {
            return true;
        }
        let core = usize::from(core.min(1));
        let shut = self.register(kind.bus_control()) & (1 << core) != 0;
        if !self.enabled(kind) || shut {
            let status_offset = if core == 0 { 0x0f0 } else { 0x0fc };
            let status_bit = if kind == CacheKind::Instruction { 2 } else { 4 };
            self.set_register(
                status_offset,
                self.register(status_offset) | (1 << status_bit),
            );
            let reject_offset = match (core, kind) {
                (0, CacheKind::Data) => 0x100,
                (0, CacheKind::Instruction) => 0x108,
                (1, CacheKind::Data) => 0x110,
                (1, CacheKind::Instruction) => 0x118,
                _ => unreachable!(),
            };
            self.set_register(reject_offset + 4, address);
            return false;
        }

        let (count_offset, miss_offset) = match kind {
            CacheKind::Instruction => (0x0cc, 0x0c8),
            CacheKind::Data => {
                let miss = if address < 0x3d00_0000 { 0x0d0 } else { 0x0d4 };
                (0x0d8, miss)
            }
        };
        self.increment_counter(count_offset);
        let index = Self::cache_index(kind);
        let block = self.block_address(kind, address);
        if !self.tags[index].contains(&block) {
            self.increment_counter(miss_offset);
            self.tags[index].insert(block);
        }
        true
    }

    fn increment_counter(&mut self, offset: u64) {
        let old = self.register(offset);
        let new = old.wrapping_add(1);
        self.set_register(offset, new);
        if new == 0 {
            let bit = match offset {
                0x0c8 => 8,
                0x0cc => 7,
                0x0d0 => 10,
                0x0d4 => 11,
                0x0d8 => 9,
                _ => return,
            };
            self.illegal(bit);
        }
    }

    fn configure_boot_caches(&mut self) {
        self.set_register(0x000, self.register(0x000) | 1);
        self.set_register(0x004, 0);
        self.set_register(0x060, self.register(0x060) | 1);
        self.set_register(0x064, 0);
    }

    fn interrupt_pending(&self, source: usize) -> bool {
        match source {
            56 => {
                self.register(0x0e4) & self.register(0x0dc) != 0
                    || self.register(0x0f0) & self.register(0x0e8) != 0
                    || self.register(0x0fc) & self.register(0x0f4) != 0
            }
            61 => self.register(0x140) & 0x18 == 0x18,
            62 => self.register(0x140) & 0x03 == 0x03,
            63 => self.register(0x144) & 0x18 == 0x18,
            64 => self.register(0x144) & 0x03 == 0x03,
            _ => false,
        }
    }
}

/// Scheduler-facing view of the ESP32-S3 external-memory/cache controller.
#[derive(Clone)]
pub struct Esp32S3ExtmemHandle {
    state: Rc<RefCell<Esp32S3ExtmemState>>,
}

impl Esp32S3ExtmemHandle {
    /// Establishes the cache state normally configured by the second-stage bootloader.
    pub fn configure_boot_caches(&self) {
        self.state.borrow_mut().configure_boot_caches();
    }

    /// Records and validates a CPU access through one of the cached external-memory aliases.
    pub fn observe_access(&self, core: u8, address: u32, access: AccessKind) -> bool {
        self.state
            .borrow_mut()
            .observe_access(core, address, access)
    }

    /// Returns the level of one native cache interrupt-matrix source.
    pub fn interrupt_pending(&self, source: usize) -> bool {
        self.state.borrow().interrupt_pending(source)
    }
}

/// Functional ESP32-S3 external-memory/cache controller.
pub struct Esp32S3Extmem {
    name: String,
    state: Rc<RefCell<Esp32S3ExtmemState>>,
}

impl Esp32S3Extmem {
    /// Creates reset controller state and its scheduler-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, Esp32S3ExtmemHandle) {
        let state = Rc::new(RefCell::new(Esp32S3ExtmemState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3ExtmemHandle { state },
        )
    }
}

impl Device for Esp32S3Extmem {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-S3 EXTMEM requires aligned word access",
            ));
        }
        let (_, spec) = spec_for_offset(offset).ok_or_else(|| {
            DeviceError::new(format!("{} read at reserved offset {offset:#x}", self.name))
        })?;
        Ok(u64::from(
            self.state.borrow().register(offset) & spec.read_mask,
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-S3 EXTMEM requires aligned word access",
            ));
        }
        let (_, spec) = spec_for_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write at reserved offset {offset:#x}",
                self.name
            ))
        })?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new(format!("{} word write exceeds 32 bits", self.name)))?;
        let mut state = self.state.borrow_mut();

        match offset {
            0x0c4 => {
                if value & 1 != 0 {
                    for offset in [0x0d0, 0x0d4, 0x0d8] {
                        state.set_register(offset, 0);
                    }
                }
                if value & 2 != 0 {
                    for offset in [0x0c8, 0x0cc] {
                        state.set_register(offset, 0);
                    }
                }
                return Ok(());
            }
            0x0e0 => {
                let status = state.register(0x0e4) & !(value & 0x1ff);
                state.set_register(0x0e4, status);
                return Ok(());
            }
            0x0ec | 0x0f8 => {
                let status = offset + 4;
                let cleared = state.register(status) & !(value & 0x1f);
                state.set_register(status, cleared);
                return Ok(());
            }
            0x140 | 0x144 => {
                let old = state.register(offset);
                let clear = ((value >> 2) & 1) | ((value >> 2) & 8);
                let status = (old & 0x09) & !clear;
                let enables = value & 0x12;
                state.set_register(offset, status | enables);
                return Ok(());
            }
            _ => {}
        }

        let old = state.register(offset);
        let stored = (old & !spec.write_mask) | (value & spec.write_mask);
        state.set_register(offset, stored);
        match offset {
            0x01c => state.complete_lock(CacheKind::Data, stored),
            0x028 => state.complete_sync(CacheKind::Data, stored),
            0x034 => state.complete_occupy(stored),
            0x040 => state.complete_preload(CacheKind::Data, stored),
            0x07c => state.complete_lock(CacheKind::Instruction, stored),
            0x088 => state.complete_sync(CacheKind::Instruction, stored),
            0x094 => state.complete_preload(CacheKind::Instruction, stored),
            0x04c | 0x0a0 if stored & (0x260) != 0 => {
                state.set_register(offset, (stored & !0x260) | 8);
            }
            0x150 | 0x154 => state.set_register(offset, (stored & 3) | 4),
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = Esp32S3ExtmemState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(device: &mut Esp32S3Extmem, offset: u64) -> u32 {
        device
            .read(offset, AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32
    }

    fn write(device: &mut Esp32S3Extmem, offset: u64, value: u32) {
        device
            .write(offset, AccessWidth::Word, u64::from(value), SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn all_vendor_registers_have_exact_resets_masks_and_holes() {
        let (mut device, _) = Esp32S3Extmem::new("extmem");
        let mut count = 0;
        for offset in (0..0x400).step_by(4) {
            if let Some((_, spec)) = spec_for_offset(offset) {
                count += 1;
                assert_eq!(read(&mut device, offset), spec.reset & spec.read_mask);
                let (mut isolated, _) = Esp32S3Extmem::new("isolated");
                write(&mut isolated, offset, u32::MAX);
                if !matches!(offset, 0x0c4 | 0x0e0 | 0x0ec | 0x0f8 | 0x140 | 0x144)
                    && !matches!(
                        offset,
                        0x01c
                            | 0x028
                            | 0x034
                            | 0x040
                            | 0x04c
                            | 0x07c
                            | 0x088
                            | 0x094
                            | 0x0a0
                            | 0x150
                            | 0x154
                    )
                {
                    assert_eq!(
                        read(&mut isolated, offset),
                        ((spec.reset & !spec.write_mask) | spec.write_mask) & spec.read_mask,
                        "write mask mismatch at {offset:#x}"
                    );
                }
            } else {
                assert!(
                    device
                        .read(offset, AccessWidth::Word, SimTime::ZERO)
                        .is_err()
                );
            }
        }
        assert_eq!(count, 95);
        assert!(device.read(1, AccessWidth::Byte, SimTime::ZERO).is_err());
    }

    #[test]
    fn preload_sync_lock_and_counter_paths_are_functional() {
        let (mut device, handle) = Esp32S3Extmem::new("extmem");
        handle.configure_boot_caches();
        write(&mut device, 0x140, 0x12);
        write(&mut device, 0x144, 0x12);
        write(&mut device, 0x044, 0x3c00_1000);
        write(&mut device, 0x048, 2);
        write(&mut device, 0x040, 1);
        assert_eq!(read(&mut device, 0x040), 2);
        assert!(handle.interrupt_pending(61));
        assert!(handle.observe_access(0, 0x3c00_1004, AccessKind::Read));
        assert_eq!(read(&mut device, 0x0d8), 1);
        assert_eq!(read(&mut device, 0x0d0), 0);

        write(&mut device, 0x020, 0x3c00_1000);
        write(&mut device, 0x024, 1);
        write(&mut device, 0x01c, 1);
        write(&mut device, 0x02c, 0x3c00_1000);
        write(&mut device, 0x030, 2);
        write(&mut device, 0x028, 1);
        assert_eq!(read(&mut device, 0x028), 8);
        assert!(handle.interrupt_pending(63));
        assert!(handle.observe_access(0, 0x3c00_1004, AccessKind::Read));
        assert_eq!(
            read(&mut device, 0x0d0),
            0,
            "locked line survives invalidation"
        );
        assert!(handle.observe_access(0, 0x3c00_1044, AccessKind::Read));
        assert_eq!(read(&mut device, 0x0d0), 1);
        write(&mut device, 0x0c4, 1);
        assert_eq!(read(&mut device, 0x0d8), 0);
        assert_eq!(read(&mut device, 0x0d0), 0);
    }

    #[test]
    fn disabled_bus_rejection_and_illegal_operation_interrupts_clear() {
        let (mut device, handle) = Esp32S3Extmem::new("extmem");
        write(&mut device, 0x0e8, 1 << 4);
        assert!(!handle.observe_access(0, 0x3c00_0040, AccessKind::Read));
        assert_eq!(read(&mut device, 0x104), 0x3c00_0040);
        assert!(handle.interrupt_pending(56));
        write(&mut device, 0x0ec, 1 << 4);
        assert!(!handle.interrupt_pending(56));

        write(&mut device, 0x0dc, 1 << 1);
        write(&mut device, 0x098, 0x4200_0000);
        write(&mut device, 0x09c, 1);
        write(&mut device, 0x094, 1);
        assert_eq!(read(&mut device, 0x0e4) & (1 << 1), 1 << 1);
        assert!(handle.interrupt_pending(56));
        write(&mut device, 0x0e0, 1 << 1);
        assert!(!handle.interrupt_pending(56));
    }
}
