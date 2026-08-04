use super::{AccessWidth, Device, DeviceError, Rc, RefCell, ResetKind, SimTime};
use remu_core::AccessKind;

const SYSCON_WORDS: usize = 0xcc / 4;
const EXTERNAL_PERMISSION_FIRST: u64 = 0x28;
const EXTERNAL_PERMISSION_LAST: u64 = 0x84;

/// One of the external-memory permission tables controlled by SYSCON.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32S3ExternalMemory {
    /// External SPI flash.
    Flash,
    /// External SPI RAM.
    Sram,
}

/// Hardware path and security world used for an external-memory access.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32S3ExternalAccessPath {
    /// Cache access from the secure world.
    SecureCache,
    /// Cache access from the non-secure world.
    NonSecureCache,
    /// Direct SPI1 access.
    Spi,
}

/// Native ESP32-S3 SYSCON register identifiers.
#[allow(missing_docs)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32S3SysconRegister {
    SysclkConf,
    TickConf,
    ClockOutEnable,
    RadioBbConfig(u8),
    RadioClockEnable,
    RadioResetEnable,
    HostInterfaceSelect,
    ExternalMemoryPermissionLock,
    ExternalMemoryWritebackBypass,
    FlashAttribute(u8),
    FlashAddress(u8),
    FlashSize(u8),
    SramAttribute(u8),
    SramAddress(u8),
    SramSize(u8),
    SpiMemoryPermissionControl,
    SpiMemoryRejectAddress,
    SdioControl,
    RedundancySignal(u8),
    FrontEndMemoryPower,
    SpiMemoryEccControl,
    ClockGateForceOn,
    MemoryPowerDown,
    MemoryPowerUp,
    RetentionControl(u8),
    Date,
}

impl Esp32S3SysconRegister {
    /// Returns the native byte offset within the SYSCON page.
    pub const fn offset(self) -> u64 {
        match self {
            Self::SysclkConf => 0x00,
            Self::TickConf => 0x04,
            Self::ClockOutEnable => 0x08,
            Self::RadioBbConfig(index) => 0x0c + index as u64 * 4,
            Self::RadioClockEnable => 0x14,
            Self::RadioResetEnable => 0x18,
            Self::HostInterfaceSelect => 0x1c,
            Self::ExternalMemoryPermissionLock => 0x20,
            Self::ExternalMemoryWritebackBypass => 0x24,
            Self::FlashAttribute(index) => 0x28 + index as u64 * 4,
            Self::FlashAddress(index) => 0x38 + index as u64 * 4,
            Self::FlashSize(index) => 0x48 + index as u64 * 4,
            Self::SramAttribute(index) => 0x58 + index as u64 * 4,
            Self::SramAddress(index) => 0x68 + index as u64 * 4,
            Self::SramSize(index) => 0x78 + index as u64 * 4,
            Self::SpiMemoryPermissionControl => 0x88,
            Self::SpiMemoryRejectAddress => 0x8c,
            Self::SdioControl => 0x90,
            Self::RedundancySignal(index) => 0x94 + index as u64 * 4,
            Self::FrontEndMemoryPower => 0x9c,
            Self::SpiMemoryEccControl => 0xa0,
            Self::ClockGateForceOn => 0xa8,
            Self::MemoryPowerDown => 0xac,
            Self::MemoryPowerUp => 0xb0,
            Self::RetentionControl(index) => 0xb4 + index as u64 * 4,
            Self::Date => 0x3fc,
        }
    }

    /// Resolves an aligned documented register offset.
    pub fn from_offset(offset: u64) -> Option<Self> {
        if !offset.is_multiple_of(4) {
            return None;
        }
        match offset {
            0x00 => Some(Self::SysclkConf),
            0x04 => Some(Self::TickConf),
            0x08 => Some(Self::ClockOutEnable),
            0x0c | 0x10 => Some(Self::RadioBbConfig(field_index(offset, 0x0c))),
            0x14 => Some(Self::RadioClockEnable),
            0x18 => Some(Self::RadioResetEnable),
            0x1c => Some(Self::HostInterfaceSelect),
            0x20 => Some(Self::ExternalMemoryPermissionLock),
            0x24 => Some(Self::ExternalMemoryWritebackBypass),
            0x28..=0x34 => Some(Self::FlashAttribute(field_index(offset, 0x28))),
            0x38..=0x44 => Some(Self::FlashAddress(field_index(offset, 0x38))),
            0x48..=0x54 => Some(Self::FlashSize(field_index(offset, 0x48))),
            0x58..=0x64 => Some(Self::SramAttribute(field_index(offset, 0x58))),
            0x68..=0x74 => Some(Self::SramAddress(field_index(offset, 0x68))),
            0x78..=0x84 => Some(Self::SramSize(field_index(offset, 0x78))),
            0x88 => Some(Self::SpiMemoryPermissionControl),
            0x8c => Some(Self::SpiMemoryRejectAddress),
            0x90 => Some(Self::SdioControl),
            0x94 | 0x98 => Some(Self::RedundancySignal(field_index(offset, 0x94))),
            0x9c => Some(Self::FrontEndMemoryPower),
            0xa0 => Some(Self::SpiMemoryEccControl),
            0xa8 => Some(Self::ClockGateForceOn),
            0xac => Some(Self::MemoryPowerDown),
            0xb0 => Some(Self::MemoryPowerUp),
            0xb4..=0xc8 => Some(Self::RetentionControl(field_index(offset, 0xb4))),
            0x3fc => Some(Self::Date),
            _ => None,
        }
    }

    /// Hardware reset value from Espressif's ESP32-S3 register definition.
    pub const fn reset_value(self) -> u32 {
        match self {
            Self::SysclkConf => 0x0000_0001,
            Self::TickConf => 0x0001_0727,
            Self::ClockOutEnable => 0x0000_07ff,
            Self::RadioClockEnable => 0xfffc_e030,
            Self::RadioBbConfig(_)
            | Self::RadioResetEnable
            | Self::HostInterfaceSelect
            | Self::ExternalMemoryPermissionLock
            | Self::ExternalMemoryWritebackBypass
            | Self::SpiMemoryPermissionControl
            | Self::SpiMemoryRejectAddress
            | Self::SdioControl
            | Self::RedundancySignal(_)
            | Self::MemoryPowerDown => 0,
            Self::FlashAttribute(_) | Self::SramAttribute(_) => 0xff,
            Self::FlashAddress(index) | Self::SramAddress(index) => (index as u32) << 28,
            Self::FlashSize(_) | Self::SramSize(_) => 0x1000,
            Self::FrontEndMemoryPower => 0x55,
            Self::SpiMemoryEccControl => 0x0020_0000,
            Self::ClockGateForceOn | Self::MemoryPowerUp => 0x3fff,
            Self::RetentionControl(2) => 0x001f_eff0,
            Self::RetentionControl(3) => 0x003f_fff0,
            Self::RetentionControl(4) => u32::MAX,
            Self::RetentionControl(_) => 0,
            Self::Date => 0x0210_1150,
        }
    }

    /// Bits visible on a native read.
    pub const fn read_mask(self) -> u32 {
        match self {
            Self::SysclkConf => 0x1fff,
            Self::TickConf => 0x0001_ffff,
            Self::ClockOutEnable => 0x07ff,
            Self::RadioBbConfig(_)
            | Self::RadioClockEnable
            | Self::RadioResetEnable
            | Self::SpiMemoryRejectAddress
            | Self::Date
            | Self::FlashAddress(_)
            | Self::SramAddress(_)
            | Self::RedundancySignal(_)
            | Self::RetentionControl(4) => u32::MAX,
            Self::HostInterfaceSelect | Self::FrontEndMemoryPower => 0xff,
            Self::ExternalMemoryPermissionLock
            | Self::ExternalMemoryWritebackBypass
            | Self::SdioControl
            | Self::RetentionControl(5) => 1,
            Self::FlashAttribute(_) | Self::SramAttribute(_) => 0x01ff,
            Self::FlashSize(_) | Self::SramSize(_) => 0xffff,
            Self::SpiMemoryPermissionControl => 0x7d,
            Self::SpiMemoryEccControl => 0x003c_0000,
            Self::ClockGateForceOn | Self::MemoryPowerDown | Self::MemoryPowerUp => 0x3fff,
            Self::RetentionControl(0) => 0x0fff_ffff,
            Self::RetentionControl(1) => 0x07ff_ffff,
            Self::RetentionControl(2) => 0xbfff_fff0,
            Self::RetentionControl(3) => 0xffff_fff0,
            Self::RetentionControl(_) => 0,
        }
    }

    /// Bits accepted by a native write.
    pub const fn write_mask(self) -> u32 {
        match self {
            Self::SpiMemoryPermissionControl => 0x2,
            Self::SpiMemoryRejectAddress => 0,
            Self::RedundancySignal(_) => 0x7fff_ffff,
            _ => self.read_mask(),
        }
    }

    const fn is_external_permission(self) -> bool {
        let offset = self.offset();
        offset >= EXTERNAL_PERMISSION_FIRST && offset <= EXTERNAL_PERMISSION_LAST
    }
}

fn field_index(offset: u64, base: u64) -> u8 {
    u8::try_from((offset - base) / 4).expect("bounded SYSCON field index fits u8")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExternalRegion {
    start: u32,
    end: u64,
    attributes: u32,
}

struct Esp32S3SysconState {
    registers: [u32; SYSCON_WORDS],
    date: u32,
    reject_cause: u8,
    reject_address: u32,
}

impl Esp32S3SysconState {
    fn new() -> Self {
        let mut state = Self {
            registers: [0; SYSCON_WORDS],
            date: Esp32S3SysconRegister::Date.reset_value(),
            reject_cause: 0,
            reject_address: 0,
        };
        for offset in (0..=0xc8).step_by(4) {
            if let Some(register) = Esp32S3SysconRegister::from_offset(offset) {
                state.set_register(register, register.reset_value());
            }
        }
        state
    }

    fn register(&self, register: Esp32S3SysconRegister) -> u32 {
        match register {
            Esp32S3SysconRegister::SpiMemoryPermissionControl => {
                (u32::from(self.reject_cause) << 2) | u32::from(self.reject_cause != 0)
            }
            Esp32S3SysconRegister::SpiMemoryRejectAddress => self.reject_address,
            Esp32S3SysconRegister::Date => self.date,
            _ => self.registers[register_index(register)],
        }
    }

    fn set_register(&mut self, register: Esp32S3SysconRegister, value: u32) {
        if register == Esp32S3SysconRegister::Date {
            self.date = value;
        } else {
            self.registers[register_index(register)] = value;
        }
    }

    fn region(&self, memory: Esp32S3ExternalMemory, index: u8) -> ExternalRegion {
        let (attribute, address, size) = match memory {
            Esp32S3ExternalMemory::Flash => (
                Esp32S3SysconRegister::FlashAttribute(index),
                Esp32S3SysconRegister::FlashAddress(index),
                Esp32S3SysconRegister::FlashSize(index),
            ),
            Esp32S3ExternalMemory::Sram => (
                Esp32S3SysconRegister::SramAttribute(index),
                Esp32S3SysconRegister::SramAddress(index),
                Esp32S3SysconRegister::SramSize(index),
            ),
        };
        let start = self.register(address);
        let length = u64::from(self.register(size)) << 16;
        ExternalRegion {
            start,
            end: u64::from(start).saturating_add(length),
            attributes: self.register(attribute),
        }
    }

    fn matching_regions(&self, memory: Esp32S3ExternalMemory, address: u32) -> Vec<ExternalRegion> {
        (0..4)
            .map(|index| self.region(memory, index))
            .filter(|region| {
                u64::from(address) >= u64::from(region.start) && u64::from(address) < region.end
            })
            .collect()
    }

    fn latch_reject(&mut self, address: u32, cause: u8) {
        if self.reject_cause == 0 {
            self.reject_address = address;
            self.reject_cause = cause & 0x1f;
        }
    }
}

fn register_index(register: Esp32S3SysconRegister) -> usize {
    usize::try_from(register.offset() / 4).expect("SYSCON register index fits usize")
}

/// Host and scheduler-facing view of ESP32-S3 SYSCON behavior.
#[derive(Clone)]
pub struct Esp32S3SysconHandle {
    state: Rc<RefCell<Esp32S3SysconState>>,
}

impl Esp32S3SysconHandle {
    /// Returns whether the external-memory rejection interrupt is asserted.
    pub fn interrupt_pending(&self) -> bool {
        self.state.borrow().reject_cause != 0
    }

    /// Reports an external-memory rejection. Only the first uncleared event is latched.
    pub fn report_external_reject(&self, address: u32, cause: u8) {
        self.state.borrow_mut().latch_reject(address, cause);
    }

    /// Checks one access against the four configured external-memory regions.
    /// A failed check latches the documented rejection cause and address.
    pub fn check_external_access(
        &self,
        memory: Esp32S3ExternalMemory,
        address: u32,
        path: Esp32S3ExternalAccessPath,
        access: AccessKind,
    ) -> bool {
        let mut state = self.state.borrow_mut();
        let regions = state.matching_regions(memory, address);
        let cause = if regions.is_empty() {
            Some(0x10)
        } else if regions.len() > 1 {
            Some(0x08)
        } else {
            let attributes = regions[0].attributes;
            let allowed = match (path, access) {
                (Esp32S3ExternalAccessPath::SecureCache, AccessKind::Write) => {
                    attributes & (1 << 2) != 0
                }
                (Esp32S3ExternalAccessPath::SecureCache, AccessKind::Read) => {
                    attributes & (1 << 1) != 0
                }
                (Esp32S3ExternalAccessPath::SecureCache, AccessKind::Execute) => {
                    attributes & 1 != 0
                }
                (Esp32S3ExternalAccessPath::NonSecureCache, AccessKind::Write) => {
                    attributes & (1 << 5) != 0
                }
                (Esp32S3ExternalAccessPath::NonSecureCache, AccessKind::Read) => {
                    attributes & (1 << 4) != 0
                }
                (Esp32S3ExternalAccessPath::NonSecureCache, AccessKind::Execute) => {
                    attributes & (1 << 3) != 0
                }
                (Esp32S3ExternalAccessPath::Spi, AccessKind::Write) => attributes & (1 << 7) != 0,
                (Esp32S3ExternalAccessPath::Spi, AccessKind::Read) => attributes & (1 << 6) != 0,
                (Esp32S3ExternalAccessPath::Spi, AccessKind::Execute) => false,
            };
            (!allowed).then_some(match access {
                AccessKind::Execute => 0x01,
                AccessKind::Read => 0x02,
                AccessKind::Write => 0x04,
            })
        };
        if let Some(cause) = cause {
            state.latch_reject(address, cause);
            false
        } else {
            true
        }
    }

    /// Returns whether an internal ROM/SRAM address is outside forced retention.
    pub fn internal_memory_accessible(&self, address: u32) -> bool {
        let Some(bit) = internal_memory_power_bit(address) else {
            return true;
        };
        self.state
            .borrow()
            .register(Esp32S3SysconRegister::MemoryPowerDown)
            & (1 << bit)
            == 0
    }

    /// Returns the configured XTAL tick count, including the current cycle.
    pub fn xtal_tick_cycles(&self) -> u16 {
        let tick = self
            .state
            .borrow()
            .register(Esp32S3SysconRegister::TickConf);
        u16::try_from((tick & 0xff) + 1).expect("eight-bit tick count plus one fits u16")
    }

    /// Returns whether the programmable tick generator is enabled.
    pub fn tick_enabled(&self) -> bool {
        self.state
            .borrow()
            .register(Esp32S3SysconRegister::TickConf)
            & (1 << 16)
            != 0
    }
}

fn internal_memory_power_bit(address: u32) -> Option<u8> {
    match address {
        0x4000_0000..=0x4003_ffff => Some(0),
        0x4004_0000..=0x4004_ffff => Some(1),
        0x4005_0000..=0x4005_ffff | 0x3ff0_0000..=0x3ff0_ffff => Some(2),
        0x4037_0000..=0x4037_3fff => Some(3),
        0x4037_4000..=0x4037_7fff => Some(4),
        0x4037_8000..=0x4037_ffff | 0x3fc8_8000..=0x3fc8_ffff => Some(5),
        0x4038_0000..=0x4038_ffff | 0x3fc9_0000..=0x3fc9_ffff => Some(6),
        0x4039_8000..=0x4039_ffff | 0x3fca_0000..=0x3fca_ffff => Some(7),
        0x403a_c000..=0x403a_ffff | 0x3fcb_c000..=0x3fcb_ffff => Some(8),
        0x403b_0000..=0x403b_ffff | 0x3fcc_0000..=0x3fcc_ffff => Some(9),
        0x403c_0000..=0x403c_ffff | 0x3fcd_4000..=0x3fcd_ffff => Some(10),
        0x403d_0000..=0x403d_bfff | 0x3fce_8000..=0x3fce_ffff => Some(11),
        0x3fcf_0000..=0x3fcf_7fff => Some(12),
        0x3fcf_8000..=0x3fcf_ffff => Some(13),
        _ => None,
    }
}

/// Functional ESP32-S3 SYSCON register block.
pub struct Esp32S3Syscon {
    name: String,
    state: Rc<RefCell<Esp32S3SysconState>>,
}

impl Esp32S3Syscon {
    /// Creates reset SYSCON state and its scheduler-facing handle.
    pub fn new(name: impl Into<String>) -> (Self, Esp32S3SysconHandle) {
        let state = Rc::new(RefCell::new(Esp32S3SysconState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3SysconHandle { state },
        )
    }
}

impl Device for Esp32S3Syscon {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-S3 SYSCON requires aligned word access",
            ));
        }
        let register = Esp32S3SysconRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!("{} read at reserved offset {offset:#x}", self.name))
        })?;
        Ok(u64::from(
            self.state.borrow().register(register) & register.read_mask(),
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
                "ESP32-S3 SYSCON requires aligned word access",
            ));
        }
        let register = Esp32S3SysconRegister::from_offset(offset).ok_or_else(|| {
            DeviceError::new(format!(
                "{} write at reserved offset {offset:#x}",
                self.name
            ))
        })?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new(format!("{} word write exceeds 32 bits", self.name)))?;
        let mut state = self.state.borrow_mut();
        if register == Esp32S3SysconRegister::SpiMemoryPermissionControl {
            if value & 0x2 != 0 {
                state.reject_cause = 0;
                state.reject_address = 0;
            }
            return Ok(());
        }
        if register.is_external_permission()
            && state.register(Esp32S3SysconRegister::ExternalMemoryPermissionLock) != 0
        {
            return Ok(());
        }
        let old = state.register(register);
        let mask = register.write_mask();
        let value = (old & !mask) | (value & mask);
        if register == Esp32S3SysconRegister::ExternalMemoryPermissionLock {
            state.set_register(register, old | value);
        } else {
            state.set_register(register, value);
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = Esp32S3SysconState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(device: &mut Esp32S3Syscon, register: Esp32S3SysconRegister) -> u32 {
        device
            .read(register.offset(), AccessWidth::Word, SimTime::ZERO)
            .unwrap() as u32
    }

    fn write(device: &mut Esp32S3Syscon, register: Esp32S3SysconRegister, value: u32) {
        device
            .write(
                register.offset(),
                AccessWidth::Word,
                u64::from(value),
                SimTime::ZERO,
            )
            .unwrap();
    }

    #[test]
    fn documented_offsets_have_exact_reset_values_and_reserved_holes_fault() {
        let (mut device, _) = Esp32S3Syscon::new("syscon");
        let mut count = 0;
        for offset in (0..0x400).step_by(4) {
            if let Some(register) = Esp32S3SysconRegister::from_offset(offset) {
                count += 1;
                assert_eq!(
                    read(&mut device, register),
                    register.reset_value() & register.read_mask()
                );
            } else {
                assert!(
                    device
                        .read(offset, AccessWidth::Word, SimTime::ZERO)
                        .is_err()
                );
            }
        }
        assert_eq!(count, 51);
        assert!(device.read(1, AccessWidth::Byte, SimTime::ZERO).is_err());
    }

    #[test]
    fn masks_fields_and_external_permission_lock_is_sticky_until_reset() {
        let (mut device, _) = Esp32S3Syscon::new("syscon");
        write(&mut device, Esp32S3SysconRegister::TickConf, u32::MAX);
        assert_eq!(read(&mut device, Esp32S3SysconRegister::TickConf), 0x1ffff);
        write(&mut device, Esp32S3SysconRegister::FlashAttribute(0), 0x12a);
        write(
            &mut device,
            Esp32S3SysconRegister::ExternalMemoryPermissionLock,
            1,
        );
        write(&mut device, Esp32S3SysconRegister::FlashAttribute(0), 0x55);
        write(
            &mut device,
            Esp32S3SysconRegister::ExternalMemoryPermissionLock,
            0,
        );
        assert_eq!(
            read(&mut device, Esp32S3SysconRegister::FlashAttribute(0)),
            0x12a
        );
        assert_eq!(
            read(
                &mut device,
                Esp32S3SysconRegister::ExternalMemoryPermissionLock
            ),
            1
        );
        device.reset(ResetKind::Software);
        assert_eq!(
            read(
                &mut device,
                Esp32S3SysconRegister::ExternalMemoryPermissionLock
            ),
            0
        );
    }

    #[test]
    fn every_documented_register_applies_its_vendor_write_mask() {
        for offset in (0..0x400).step_by(4) {
            let Some(register) = Esp32S3SysconRegister::from_offset(offset) else {
                continue;
            };
            let (mut device, _) = Esp32S3Syscon::new("syscon");
            write(&mut device, register, u32::MAX);
            let expected = (register.reset_value() & !register.write_mask())
                | (u32::MAX & register.write_mask());
            assert_eq!(
                read(&mut device, register),
                expected & register.read_mask(),
                "write mask mismatch for {register:?}"
            );
        }
    }

    #[test]
    fn evaluates_world_and_spi_permissions_and_latches_first_rejection() {
        let (mut device, handle) = Esp32S3Syscon::new("syscon");
        assert!(handle.check_external_access(
            Esp32S3ExternalMemory::Flash,
            0x0001_0000,
            Esp32S3ExternalAccessPath::SecureCache,
            AccessKind::Read,
        ));
        assert!(handle.check_external_access(
            Esp32S3ExternalMemory::Flash,
            0x0001_0000,
            Esp32S3ExternalAccessPath::NonSecureCache,
            AccessKind::Execute,
        ));
        assert!(handle.check_external_access(
            Esp32S3ExternalMemory::Flash,
            0x0001_0000,
            Esp32S3ExternalAccessPath::Spi,
            AccessKind::Write,
        ));
        write(
            &mut device,
            Esp32S3SysconRegister::FlashAttribute(0),
            1 << 1,
        );
        assert!(!handle.check_external_access(
            Esp32S3ExternalMemory::Flash,
            0x0001_0000,
            Esp32S3ExternalAccessPath::SecureCache,
            AccessKind::Write,
        ));
        assert!(handle.interrupt_pending());
        assert_eq!(
            read(
                &mut device,
                Esp32S3SysconRegister::SpiMemoryPermissionControl
            ),
            0x11
        );
        assert_eq!(
            read(&mut device, Esp32S3SysconRegister::SpiMemoryRejectAddress),
            0x0001_0000
        );
        handle.report_external_reject(0x2222_0000, 0x10);
        assert_eq!(
            read(&mut device, Esp32S3SysconRegister::SpiMemoryRejectAddress),
            0x0001_0000
        );
        write(
            &mut device,
            Esp32S3SysconRegister::SpiMemoryPermissionControl,
            2,
        );
        assert!(!handle.interrupt_pending());
        assert_eq!(
            read(&mut device, Esp32S3SysconRegister::SpiMemoryRejectAddress),
            0
        );
    }

    #[test]
    fn distinguishes_overlapping_and_invalid_external_regions() {
        let (mut device, handle) = Esp32S3Syscon::new("syscon");
        write(&mut device, Esp32S3SysconRegister::FlashAddress(1), 0);
        assert!(!handle.check_external_access(
            Esp32S3ExternalMemory::Flash,
            0x0001_0000,
            Esp32S3ExternalAccessPath::SecureCache,
            AccessKind::Read,
        ));
        assert_eq!(
            read(
                &mut device,
                Esp32S3SysconRegister::SpiMemoryPermissionControl
            ),
            0x21
        );
        write(
            &mut device,
            Esp32S3SysconRegister::SpiMemoryPermissionControl,
            2,
        );
        assert!(!handle.check_external_access(
            Esp32S3ExternalMemory::Flash,
            0xffff_0000,
            Esp32S3ExternalAccessPath::SecureCache,
            AccessKind::Read,
        ));
        assert_eq!(
            read(
                &mut device,
                Esp32S3SysconRegister::SpiMemoryPermissionControl
            ),
            0x41
        );
    }

    #[test]
    fn decodes_tick_and_internal_memory_power_controls() {
        let (mut device, handle) = Esp32S3Syscon::new("syscon");
        assert!(handle.tick_enabled());
        assert_eq!(handle.xtal_tick_cycles(), 40);
        assert!(handle.internal_memory_accessible(0x3fc9_0000));
        write(&mut device, Esp32S3SysconRegister::MemoryPowerDown, 1 << 6);
        assert!(!handle.internal_memory_accessible(0x3fc9_0000));
        assert!(handle.internal_memory_accessible(0x3fca_0000));
    }
}
