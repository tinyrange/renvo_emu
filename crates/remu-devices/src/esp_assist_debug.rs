//! ESP32-S3 CPU assist-debug monitors and trace logger.

use super::*;
use remu_core::AccessKind;

const REGISTER_WORDS: usize = 0x200 / 4;
const CORE_STRIDE: u64 = 0x90;
const INTERRUPT_MASK: u32 = 0x0fff;
const DATE_RESET: u32 = 0x0200_3040;

fn index(offset: u64) -> usize {
    (offset / 4) as usize
}

fn core_base(core: usize) -> u64 {
    core as u64 * CORE_STRIDE
}

fn documented(offset: u64) -> bool {
    offset.is_multiple_of(4)
        && (offset <= 0x11c || (0x120..=0x154).contains(&offset) || offset == 0x1fc)
}

fn read_mask(offset: u64) -> u32 {
    let local = if offset < 0x90 {
        Some(offset)
    } else if offset <= 0x11c {
        Some(offset - 0x90)
    } else {
        None
    };
    match local {
        Some(0x00..=0x0c) => INTERRUPT_MASK,
        Some(0x38 | 0x54) => 0xff,
        Some(0x48 | 0x4c) => 1,
        Some(0x70 | 0x74) => 0x03ff_ffff,
        Some(0x78 | 0x84) => 0x007f_ffff,
        Some(0x7c | 0x88) => 0xffff,
        Some(_) => u32::MAX,
        None => match offset {
            0x120 | 0x124 => 0x000f_ffff,
            0x128 => 0x7f,
            0x13c => 0xffff,
            0x154 => 1,
            0x1fc => 0x0fff_ffff,
            _ => u32::MAX,
        },
    }
}

fn write_mask(offset: u64) -> u32 {
    let local = if offset < 0x90 {
        Some((0, offset))
    } else if offset <= 0x11c {
        Some((1, offset - 0x90))
    } else {
        None
    };
    match local {
        Some((_, 0x00 | 0x08 | 0x0c)) => INTERRUPT_MASK,
        Some((_, 0x04 | 0x44 | 0x50..=0x8c)) => 0,
        Some((1, 0x30 | 0x34)) => 0,
        Some((_, 0x38)) => 0xff,
        Some((_, 0x48 | 0x4c)) => 1,
        Some(_) => u32::MAX,
        None => match offset {
            0x120 | 0x124 => 0x000f_ffff,
            0x128 => 0x7f,
            0x13c => 0xffff,
            0x150 => 0,
            0x154 => 1,
            0x1fc => 0x0fff_ffff,
            _ => u32::MAX,
        },
    }
}

fn reset_value(offset: u64) -> u32 {
    let local = if offset < 0x90 {
        Some(offset)
    } else if offset <= 0x11c {
        Some(offset - 0x90)
    } else {
        None
    };
    match local {
        Some(0x10 | 0x18 | 0x20 | 0x28 | 0x40 | 0x80 | 0x8c) => u32::MAX,
        Some(0x30) if offset < 0x90 => u32::MAX,
        None if matches!(offset, 0x120 | 0x124) => 0x000f_ffff,
        None if offset == 0x128 => 0x40,
        None if offset == 0x1fc => DATE_RESET,
        _ => 0,
    }
}

/// One four-word trace record requested by the debug logger.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Esp32S3AssistDebugLogWrite {
    /// Destination address in the configured trace ring.
    pub address: u32,
    /// PC, accessed address, access metadata, and transferred value.
    pub words: [u32; 4],
}

struct AssistDebugState {
    registers: [u32; REGISTER_WORDS],
    pending_logs: VecDeque<Esp32S3AssistDebugLogWrite>,
}

impl AssistDebugState {
    fn reset() -> Self {
        let mut registers = [0; REGISTER_WORDS];
        for offset in (0..=0x1fc).step_by(4).filter(|offset| documented(*offset)) {
            registers[index(offset)] = reset_value(offset);
        }
        Self {
            registers,
            pending_logs: VecDeque::new(),
        }
    }

    fn get(&self, offset: u64) -> u32 {
        self.registers[index(offset)]
    }

    fn set(&mut self, offset: u64, value: u32) {
        self.registers[index(offset)] = value;
    }

    fn latch(&mut self, core: usize, mask: u32, pc: u32, sp: u32) {
        let base = core_base(core);
        self.set(base + 0x04, self.get(base + 0x04) | mask);
        let (area_pc, area_sp) = if core == 0 {
            (base + 0x34, base + 0x30)
        } else {
            (base + 0x30, base + 0x34)
        };
        self.set(area_pc, pc);
        self.set(area_sp, sp);
    }

    fn observe_cpu(&mut self, core: usize, pc: u32, sp: u32) {
        let base = core_base(core);
        let enabled = self.get(base);
        if enabled & (1 << 8) != 0 && sp < self.get(base + 0x3c) {
            self.latch(core, 1 << 8, pc, sp);
            self.set(base + 0x44, pc);
        }
        if enabled & (1 << 9) != 0 && sp > self.get(base + 0x40) {
            self.latch(core, 1 << 9, pc, sp);
            self.set(base + 0x44, pc);
        }
        if self.get(base + 0x48) & self.get(base + 0x4c) & 1 != 0 {
            self.set(base + 0x50, pc);
            self.set(base + 0x54, 1);
            self.set(base + 0x5c, pc);
            self.set(base + 0x6c, sp);
        }
    }

    fn observe_area(&mut self, core: usize, address: u32, kind: AccessKind, pc: u32, sp: u32) {
        if kind == AccessKind::Execute {
            return;
        }
        let base = core_base(core);
        let class_base = if (0x3fc8_8000..0x3fce_0000).contains(&address) {
            0_u32
        } else {
            4_u32
        };
        let range_base = if class_base == 0 { 0x10 } else { 0x20 };
        for range in 0..2_u64 {
            let minimum = self.get(base + range_base + range * 8);
            let maximum = self.get(base + range_base + range * 8 + 4);
            if minimum <= address && address <= maximum {
                let write = u32::from(kind == AccessKind::Write);
                let mask = 1 << (class_base + range as u32 * 2 + write);
                if self.get(base) & mask != 0 {
                    self.latch(core, mask, pc, sp);
                }
            }
        }
    }

    fn record_debug(
        &mut self,
        core: usize,
        address: u32,
        width: AccessWidth,
        kind: AccessKind,
        pc: u32,
        sp: u32,
        value: u32,
    ) {
        let base = core_base(core);
        if self.get(base + 0x48) & self.get(base + 0x4c) & 1 == 0 {
            return;
        }
        if kind == AccessKind::Execute {
            return;
        }
        let status = match kind {
            AccessKind::Read => 2,
            AccessKind::Write => 3,
            AccessKind::Execute => unreachable!(),
        } | ((width.bytes() as u32) << 8);
        self.set(base + 0x54, 1);
        self.set(base + 0x58, value);
        self.set(base + 0x5c, pc);
        self.set(base + 0x60, status);
        self.set(base + 0x64, address);
        self.set(base + 0x68, value);
        self.set(base + 0x6c, sp);
    }

    fn record_log(
        &mut self,
        address: u32,
        width: AccessWidth,
        kind: AccessKind,
        pc: u32,
        value: u32,
    ) {
        let setting = self.get(0x128);
        let enable = match kind {
            AccessKind::Execute => 1,
            AccessKind::Read => 2,
            AccessKind::Write => 4,
        };
        if setting & enable == 0 {
            return;
        }
        let minimum = self.get(0x140);
        let maximum = self.get(0x144);
        if minimum > maximum || address < minimum || address > maximum {
            return;
        }
        let start = self.get(0x148);
        let end = self.get(0x14c);
        if start & 3 != 0 || end < start.saturating_add(15) {
            self.set(0x154, 1);
            return;
        }
        let mut destination = self.get(0x150);
        if destination < start || destination > end.saturating_sub(15) {
            destination = start;
        }
        let metadata = enable | ((width.bytes() as u32) << 8);
        let words = [pc, address, metadata, value];
        for (word, value) in words.iter().copied().enumerate() {
            self.set(0x12c + word as u64 * 4, value);
        }
        self.pending_logs.push_back(Esp32S3AssistDebugLogWrite {
            address: destination,
            words,
        });
        let next = destination.saturating_add(16);
        if next > end.saturating_sub(15) {
            self.set(0x154, 1);
            self.set(
                0x150,
                if setting & 0x40 != 0 {
                    start
                } else {
                    destination
                },
            );
        } else {
            self.set(0x150, next);
        }
    }
}

/// Machine-facing monitor, interrupt, and trace-log handle.
#[derive(Clone)]
pub struct Esp32S3AssistDebugHandle {
    state: Rc<RefCell<AssistDebugState>>,
}

impl Esp32S3AssistDebugHandle {
    /// Observes one architectural CPU access.
    pub fn observe_access(
        &self,
        core: u8,
        address: u32,
        width: AccessWidth,
        kind: AccessKind,
        pc: u32,
        sp: u32,
        value: u32,
    ) {
        let core = usize::from(core.min(1));
        let mut state = self.state.borrow_mut();
        state.observe_cpu(core, pc, sp);
        state.observe_area(core, address, kind, pc, sp);
        state.record_debug(core, address, width, kind, pc, sp, value);
        state.record_log(address, width, kind, pc, value);
    }

    /// Latches an IRAM0 or DRAM0 exception-monitor record.
    pub fn record_memory_exception(
        &self,
        core: u8,
        iram: bool,
        address: u32,
        width: AccessWidth,
        kind: AccessKind,
        pc: u32,
    ) {
        let core = usize::from(core.min(1));
        let base = core_base(core);
        let mut state = self.state.borrow_mut();
        let write = u32::from(kind == AccessKind::Write);
        if iram {
            let record = (u32::from(kind != AccessKind::Execute) << 25)
                | (write << 24)
                | (address & 0x00ff_ffff);
            state.set(base + 0x70, record);
            state.set(base + 0x74, record);
            let raw = state.get(base + 0x04) | (1 << 10);
            state.set(base + 0x04, raw);
        } else {
            let record = (write << 22) | (address & 0x003f_ffff);
            let byte_enable = ((1_u32 << width.bytes()) - 1) << (address & 3);
            state.set(base + 0x78, record);
            state.set(base + 0x7c, byte_enable);
            state.set(base + 0x80, pc);
            state.set(base + 0x84, record);
            state.set(base + 0x88, byte_enable);
            state.set(base + 0x8c, pc);
            let raw = state.get(base + 0x04) | (1 << 11);
            state.set(base + 0x04, raw);
        }
    }

    /// Returns whether the selected core has an enabled, unreleased monitor interrupt.
    pub fn interrupt_pending(&self, core: u8) -> bool {
        let state = self.state.borrow();
        let base = core_base(usize::from(core.min(1)));
        state.get(base + 0x04) & state.get(base) & !state.get(base + 0x08) & INTERRUPT_MASK != 0
    }

    /// Takes the next trace-ring write requested by observed CPU activity.
    pub fn take_log_write(&self) -> Option<Esp32S3AssistDebugLogWrite> {
        self.state.borrow_mut().pending_logs.pop_front()
    }
}

/// Functional ESP32-S3 ASSIST_DEBUG register block.
pub struct Esp32S3AssistDebug {
    name: String,
    state: Rc<RefCell<AssistDebugState>>,
}

impl Esp32S3AssistDebug {
    /// Creates the debug block and its machine-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, Esp32S3AssistDebugHandle) {
        let state = Rc::new(RefCell::new(AssistDebugState::reset()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3AssistDebugHandle { state },
        )
    }

    fn validate(&self, offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !documented(offset) {
            return Err(DeviceError::new(format!(
                "{} invalid access at {offset:#x}",
                self.name
            )));
        }
        Ok(())
    }
}

impl Device for Esp32S3AssistDebug {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        self.validate(offset, width)?;
        Ok(u64::from(
            self.state.borrow().get(offset) & read_mask(offset),
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        self.validate(offset, width)?;
        let mask = write_mask(offset);
        if mask == 0 {
            return Err(DeviceError::new(format!(
                "{} write to read-only register {offset:#x}",
                self.name
            )));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ASSIST_DEBUG word exceeds 32 bits"))?;
        let mut state = self.state.borrow_mut();
        let old = state.get(offset);
        state.set(offset, (old & !mask) | (value & mask));
        let local = if offset < 0x90 {
            offset
        } else {
            offset.saturating_sub(0x90)
        };
        if offset <= 0x11c && local == 0x0c {
            let base = offset - local;
            let raw = state.get(base + 0x04) & !(value & INTERRUPT_MASK);
            state.set(base + 0x04, raw);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = AssistDebugState::reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_87_vendor_registers_have_exact_resets_masks_and_holes() {
        let (mut device, _) = Esp32S3AssistDebug::new("assist");
        let offsets = (0..=0x1fc)
            .step_by(4)
            .filter(|offset| documented(*offset))
            .collect::<Vec<_>>();
        assert_eq!(offsets.len(), 87);
        for offset in offsets {
            assert_eq!(
                device.read(offset, AccessWidth::Word, SimTime::ZERO),
                Ok(u64::from(reset_value(offset) & read_mask(offset))),
                "reset at {offset:#x}"
            );
            if write_mask(offset) != 0 {
                device
                    .write(
                        offset,
                        AccessWidth::Word,
                        u64::from(u32::MAX),
                        SimTime::ZERO,
                    )
                    .unwrap();
                let expected = if matches!(offset, 0x0c | 0x9c) {
                    write_mask(offset)
                } else {
                    (reset_value(offset) & !write_mask(offset)) | write_mask(offset)
                };
                assert_eq!(
                    device
                        .read(offset, AccessWidth::Word, SimTime::ZERO)
                        .unwrap(),
                    u64::from(expected & read_mask(offset)),
                    "mask at {offset:#x}"
                );
            }
        }
        assert!(
            device
                .read(0x158, AccessWidth::Word, SimTime::ZERO)
                .is_err()
        );
        assert!(device.read(0, AccessWidth::Byte, SimTime::ZERO).is_err());
    }

    #[test]
    fn area_stack_recording_exception_and_interrupt_paths_are_functional() {
        let (mut device, handle) = Esp32S3AssistDebug::new("assist");
        device
            .write(0x10, AccessWidth::Word, 0x3fc8_8000, SimTime::ZERO)
            .unwrap();
        device
            .write(0x14, AccessWidth::Word, 0x3fc8_8fff, SimTime::ZERO)
            .unwrap();
        device
            .write(0x3c, AccessWidth::Word, 0x3fc8_9000, SimTime::ZERO)
            .unwrap();
        device
            .write(0x48, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(0x4c, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        device
            .write(
                0x00,
                AccessWidth::Word,
                (1 << 0) | (1 << 8) | (1 << 11),
                SimTime::ZERO,
            )
            .unwrap();
        handle.observe_access(
            0,
            0x3fc8_8010,
            AccessWidth::Word,
            AccessKind::Read,
            0x4037_1000,
            0x3fc8_8000,
            0x55,
        );
        assert!(handle.interrupt_pending(0));
        assert_eq!(
            device.read(0x34, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x4037_1000
        );
        assert_eq!(
            device.read(0x64, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x3fc8_8010
        );
        handle.record_memory_exception(
            0,
            false,
            0x3fc8_8010,
            AccessWidth::Word,
            AccessKind::Write,
            0x4037_1000,
        );
        assert_ne!(
            device.read(0x04, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 11),
            0
        );
        device
            .write(
                0x0c,
                AccessWidth::Word,
                INTERRUPT_MASK.into(),
                SimTime::ZERO,
            )
            .unwrap();
        assert!(!handle.interrupt_pending(0));
    }

    #[test]
    fn trace_logger_advances_and_wraps_a_four_word_ring() {
        let (mut device, handle) = Esp32S3AssistDebug::new("assist");
        for (offset, value) in [
            (0x128, 0x44),
            (0x140, 0x6000_0000),
            (0x144, 0x6000_ffff),
            (0x148, 0x3fc8_8000),
            (0x14c, 0x3fc8_801f),
        ] {
            device
                .write(offset, AccessWidth::Word, value, SimTime::ZERO)
                .unwrap();
        }
        handle.observe_access(
            0,
            0x6000_1000,
            AccessWidth::Word,
            AccessKind::Write,
            0x4037_0000,
            0x3fc8_9000,
            0x1234,
        );
        handle.observe_access(
            0,
            0x6000_1004,
            AccessWidth::Word,
            AccessKind::Write,
            0x4037_0003,
            0x3fc8_9000,
            0x5678,
        );
        assert_eq!(handle.take_log_write().unwrap().address, 0x3fc8_8000);
        assert_eq!(handle.take_log_write().unwrap().address, 0x3fc8_8010);
        assert_eq!(
            device
                .read(0x150, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x3fc8_8000
        );
        assert_eq!(
            device
                .read(0x154, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1
        );
    }
}
