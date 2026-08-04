use super::{AccessWidth, Device, DeviceError, Rc, RefCell, ResetKind, SimTime};
use std::collections::BTreeMap;

// Generated from ESP-IDF rtc_i2c_reg.h at f992ff36f68a783d786d83178e5f85e9a9c76ead.
// Header SHA-256: 66076f2be4bf7b694a163aac6420d5c7134403bfb89660a6ad1b67100365f6aa.

#[derive(Clone, Copy)]
struct RegisterSpec {
    offset: u16,
    reset: u32,
    read_mask: u32,
    write_mask: u32,
}

const fn spec(offset: u16, reset: u32, read_mask: u32, write_mask: u32) -> RegisterSpec {
    RegisterSpec {
        offset,
        reset,
        read_mask,
        write_mask,
    }
}

const SPECS: [RegisterSpec; 31] = [
    spec(0x000, 0x00000100, 0x000fffff, 0x000fffff),
    spec(0x004, 0x00000000, 0xe000003f, 0xe000003f),
    spec(0x008, 0x00000000, 0x77ff00ff, 0x00000000),
    spec(0x00c, 0x00010000, 0x000fffff, 0x000fffff),
    spec(0x010, 0x00000000, 0x80007fff, 0x80007fff),
    spec(0x014, 0x00000100, 0x000fffff, 0x000fffff),
    spec(0x018, 0x00000010, 0x000fffff, 0x000fffff),
    spec(0x01c, 0x00000008, 0x000fffff, 0x000fffff),
    spec(0x020, 0x00000008, 0x000fffff, 0x000fffff),
    spec(0x024, 0x00000000, 0x000001ff, 0x000001ff),
    spec(0x028, 0x00000000, 0x000001ff, 0x00000000),
    spec(0x02c, 0x00000000, 0x000001ff, 0x00000000),
    spec(0x030, 0x00000000, 0x000001ff, 0x000001ff),
    spec(0x034, 0x00000000, 0x8000ffff, 0x0000ff00),
    spec(0x038, 0x00000903, 0x80003fff, 0x00003fff),
    spec(0x03c, 0x00001901, 0x80003fff, 0x00003fff),
    spec(0x040, 0x00000902, 0x80003fff, 0x00003fff),
    spec(0x044, 0x00000101, 0x80003fff, 0x00003fff),
    spec(0x048, 0x00000901, 0x80003fff, 0x00003fff),
    spec(0x04c, 0x00001701, 0x80003fff, 0x00003fff),
    spec(0x050, 0x00001901, 0x80003fff, 0x00003fff),
    spec(0x054, 0x00000904, 0x80003fff, 0x00003fff),
    spec(0x058, 0x00001901, 0x80003fff, 0x00003fff),
    spec(0x05c, 0x00000903, 0x80003fff, 0x00003fff),
    spec(0x060, 0x00000101, 0x80003fff, 0x00003fff),
    spec(0x064, 0x00000901, 0x80003fff, 0x00003fff),
    spec(0x068, 0x00001701, 0x80003fff, 0x00003fff),
    spec(0x06c, 0x00001901, 0x80003fff, 0x00003fff),
    spec(0x070, 0x00000000, 0x80003fff, 0x00003fff),
    spec(0x074, 0x00000000, 0x80003fff, 0x00003fff),
    spec(0x0fc, 0x01905310, 0x0fffffff, 0x0fffffff),
];

fn spec_for(offset: u64) -> Option<(usize, RegisterSpec)> {
    let offset = u16::try_from(offset).ok()?;
    SPECS
        .binary_search_by_key(&offset, |spec| spec.offset)
        .ok()
        .map(|index| (index, SPECS[index]))
}

struct Esp32S3RtcI2cState {
    registers: [u32; SPECS.len()],
    slave_memory: BTreeMap<(u16, u8), u8>,
    pointers: BTreeMap<u16, u8>,
}

impl Esp32S3RtcI2cState {
    fn new() -> Self {
        let mut state = Self {
            registers: [0; SPECS.len()],
            slave_memory: BTreeMap::new(),
            pointers: BTreeMap::new(),
        };
        for (index, spec) in SPECS.iter().enumerate() {
            state.registers[index] = spec.reset;
        }
        state
    }

    fn register(&self, offset: u64) -> u32 {
        spec_for(offset)
            .map(|(index, _)| self.registers[index])
            .unwrap_or(0)
    }

    fn set_register(&mut self, offset: u64, value: u32) {
        if let Some((index, _)) = spec_for(offset) {
            self.registers[index] = value;
        }
    }

    fn refresh_interrupts(&mut self) {
        self.set_register(0x02c, self.register(0x028) & self.register(0x030));
    }

    fn reset_controller(&mut self) {
        let memory = std::mem::take(&mut self.slave_memory);
        let pointers = std::mem::take(&mut self.pointers);
        *self = Self::new();
        self.slave_memory = memory;
        self.pointers = pointers;
    }

    fn execute(&mut self) {
        let control = self.register(0x004);
        if control & (1 << 30) != 0 {
            self.reset_controller();
            return;
        }
        // The RTC-domain controller needs both its register and controller
        // clocks. A disabled start request times out deterministically.
        if control & 0xa000_0000 != 0xa000_0000 {
            self.set_register(0x028, self.register(0x028) | (1 << 4));
            self.set_register(0x004, control & !(1 << 3));
            self.refresh_interrupts();
            return;
        }

        let address = u16::try_from(self.register(0x010) & 0x7fff).expect("15-bit address fits");
        let tx = u8::try_from((self.register(0x034) >> 8) & 0xff).expect("byte fits");
        let pointer = *self.pointers.entry(address).or_insert(0);
        let read = control & (1 << 5) != 0;
        let data = if read {
            let value = self
                .slave_memory
                .get(&(address, pointer))
                .copied()
                .unwrap_or(0xff);
            self.pointers.insert(address, pointer.wrapping_add(1));
            value
        } else {
            self.slave_memory.insert((address, pointer), tx);
            self.pointers.insert(address, pointer.wrapping_add(1));
            tx
        };

        self.set_register(0x034, (u32::from(tx) << 8) | u32::from(data) | (1 << 31));
        for offset in (0x038..=0x074).step_by(4) {
            let command = self.register(offset);
            if command & 0x3fff == 0 {
                break;
            }
            self.set_register(offset, command | (1 << 31));
            if (command >> 11) & 7 == 4 {
                break;
            }
        }
        let event = (1 << 2) | (1 << 3) | if read { 1 << 6 } else { 1 << 7 };
        self.set_register(0x028, self.register(0x028) | event);
        self.set_register(0x008, (1 << 5) | (u32::from(data) << 16));
        self.set_register(0x004, control & !(1 << 3));
        self.refresh_interrupts();
    }
}

/// Host and SENS-facing view of the RTC-domain I²C controller.
#[derive(Clone)]
pub struct Esp32S3RtcI2cHandle {
    state: Rc<RefCell<Esp32S3RtcI2cState>>,
}

impl Esp32S3RtcI2cHandle {
    /// Installs one deterministic byte in an RTC-I²C slave register.
    pub fn set_slave_register(&self, address: u16, register: u8, value: u8) {
        self.state
            .borrow_mut()
            .slave_memory
            .insert((address & 0x7fff, register), value);
    }

    /// Reads one RTC-I²C slave register for host-side verification.
    pub fn slave_register(&self, address: u16, register: u8) -> u8 {
        self.state
            .borrow()
            .slave_memory
            .get(&(address & 0x7fff, register))
            .copied()
            .unwrap_or(0xff)
    }

    /// Sets the register pointer used by the next controller transfer.
    pub fn set_pointer(&self, address: u16, register: u8) {
        self.state
            .borrow_mut()
            .pointers
            .insert(address & 0x7fff, register);
    }

    /// Returns whether an enabled RTC-I²C event is pending.
    pub fn interrupt_pending(&self) -> bool {
        self.state.borrow().register(0x02c) != 0
    }
}

/// Functional ESP32-S3 RTC-domain I²C controller.
pub struct Esp32S3RtcI2c {
    name: String,
    state: Rc<RefCell<Esp32S3RtcI2cState>>,
}

impl Esp32S3RtcI2c {
    /// Creates reset controller state and its integration handle.
    pub fn new(name: impl Into<String>) -> (Self, Esp32S3RtcI2cHandle) {
        let state = Rc::new(RefCell::new(Esp32S3RtcI2cState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3RtcI2cHandle { state },
        )
    }
}

impl Device for Esp32S3RtcI2c {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-S3 RTC-I2C requires aligned word access",
            ));
        }
        let (_, spec) = spec_for(offset).ok_or_else(|| {
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
                "ESP32-S3 RTC-I2C requires aligned word access",
            ));
        }
        let (_, spec) = spec_for(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write at reserved offset {offset:#x}",
                self.name
            ))
        })?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new(format!("{} word write exceeds 32 bits", self.name)))?;
        let mut state = self.state.borrow_mut();
        if offset == 0x024 {
            let raw = state.register(0x028) & !(value & 0x1ff);
            state.set_register(0x028, raw);
            state.refresh_interrupts();
            return Ok(());
        }
        let old = state.register(offset);
        state.set_register(offset, (old & !spec.write_mask) | (value & spec.write_mask));
        if offset == 0x004 && value & ((1 << 3) | (1 << 30)) != 0 {
            state.execute();
        } else if offset == 0x030 {
            state.refresh_interrupts();
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

    fn read(device: &mut Esp32S3RtcI2c, offset: u64) -> u32 {
        device
            .read(offset, AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32
    }

    fn write(device: &mut Esp32S3RtcI2c, offset: u64, value: u32) {
        device
            .write(offset, AccessWidth::Word, u64::from(value), SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn exact_vendor_register_contract_and_reserved_holes() {
        let (mut device, _) = Esp32S3RtcI2c::new("rtc-i2c");
        let mut count = 0;
        for offset in (0..0x100).step_by(4) {
            if let Some((_, spec)) = spec_for(offset) {
                count += 1;
                assert_eq!(read(&mut device, offset), spec.reset & spec.read_mask);
                if !matches!(offset, 0x004 | 0x024 | 0x030) {
                    let (mut isolated, _) = Esp32S3RtcI2c::new("isolated");
                    write(&mut isolated, offset, u32::MAX);
                    assert_eq!(
                        read(&mut isolated, offset),
                        ((spec.reset & !spec.write_mask) | spec.write_mask) & spec.read_mask
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
        assert_eq!(count, 31);
    }

    #[test]
    fn master_write_read_interrupt_and_reset_paths_are_functional() {
        let (mut device, handle) = Esp32S3RtcI2c::new("rtc-i2c");
        handle.set_pointer(0x42, 7);
        write(&mut device, 0x010, 0x42);
        write(&mut device, 0x034, 0xa5 << 8);
        write(&mut device, 0x030, (1 << 7) | (1 << 6));
        write(&mut device, 0x004, 0xa000_000c);
        assert_eq!(handle.slave_register(0x42, 7), 0xa5);
        assert!(handle.interrupt_pending());
        write(&mut device, 0x024, 1 << 7);
        handle.set_pointer(0x42, 7);
        write(&mut device, 0x004, 0xa000_002c);
        assert_eq!(read(&mut device, 0x034) & 0xff, 0xa5);
        assert_ne!(read(&mut device, 0x034) & (1 << 31), 0);
        assert_ne!(read(&mut device, 0x02c) & (1 << 6), 0);
        write(&mut device, 0x004, 1 << 30);
        assert_eq!(read(&mut device, 0x004), 0);
        assert_eq!(handle.slave_register(0x42, 7), 0xa5);
    }
}
