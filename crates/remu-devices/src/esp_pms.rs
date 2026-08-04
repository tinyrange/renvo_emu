use super::{AccessWidth, Device, DeviceError, Rc, RefCell, ResetKind, SimTime};
use remu_core::AccessKind;

const PMS_WORDS: usize = 0x310 / 4;
const DATE_OFFSET: u64 = 0xffc;
const DATE_RESET: u32 = 0x0210_1280;
const DATE_MASK: u32 = 0x0fff_ffff;

// Generated from ESP-IDF sensitive_reg.h at f992ff36f68a783d786d83178e5f85e9a9c76ead
// (SHA-256 f989baceaf409133537146ca7c377e3502ff792573b40aca48e516b503ba575c).
#[allow(clippy::unreadable_literal)]
const PMS_RESET: [u32; PMS_WORDS] = [
    0x00000000, 0x000000ff, 0x00000000, 0x00000001, 0x00000000, 0x000007ff, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x0000000f, 0x00000000, 0x00000003, 0x00000000, 0x00000fff,
    0x00000000, 0x00000fff, 0x00000000, 0x00000fff, 0x00000000, 0x00000fff, 0x00000000, 0x00000fff,
    0x00000000, 0x00000fff, 0x00000000, 0x00000fff, 0x00000000, 0x00000fff, 0x00000000, 0x00000fff,
    0x00000000, 0x00000fff, 0x00000000, 0x00000fff, 0x00000000, 0x00000fff, 0x00000000, 0x00000fff,
    0x00000000, 0x00000fff, 0x00000000, 0x00000fff, 0x00000000, 0x00000003, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x001fffff,
    0x001fffff, 0x00000000, 0x00000003, 0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000000,
    0x0fffffff, 0x00000000, 0x00000003, 0x00000000, 0x00000000, 0x00000000, 0x00000003, 0x00000000,
    0x00000000, 0x00000000, 0xff33cfff, 0xffcffff3, 0x3cc3ffff, 0xffffffff, 0xff33cfff, 0xffcffff3,
    0x3cc3ffff, 0xffffffff, 0x003fffff, 0x00000fff, 0x003fffff, 0x00000fff, 0x003fffff, 0x00000fff,
    0x00000000, 0x003fffff, 0x003fffff, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000003, 0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000000, 0x00000000, 0x00000001,
    0x00000000, 0x00000000, 0x00000000, 0x00000001, 0x00000000, 0xff33cfff, 0xffcffff3, 0x3cc3ffff,
    0xffffffff, 0xff33cfff, 0xffcffff3, 0x3cc3ffff, 0xffffffff, 0x003fffff, 0x00000fff, 0x003fffff,
    0x00000fff, 0x003fffff, 0x00000fff, 0x00000000, 0x003fffff, 0x003fffff, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00000000,
    0x00000000, 0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000000, 0x00000003, 0x00000000,
    0x00000000, 0x00000000, 0x00000001, 0x00000000, 0x00000000, 0x00000000, 0x00000001, 0x00000000,
    0xff33cfff, 0xffcffff3, 0x3cc3ffff, 0xffffffff, 0x000007ff, 0x0000003f, 0x00000000, 0x00000003,
    0x00000000, 0x00000000, 0x00000000, 0x00000000, 0x00002000, 0x00002000, 0x00000000, 0x0000000f,
    0x00000000, 0x0000000f, 0x00000000, 0x0000000f, 0x00000000, 0x0000000f, 0x00000000, 0x0000000f,
    0x00000000, 0x0000000f, 0x00000000, 0x0000000f, 0x00000000, 0x0000000f, 0x00000000, 0x0000000f,
    0x00000000, 0x0000000f, 0x00000001, 0x00000000,
];

#[allow(clippy::unreadable_literal)]
const PMS_READ: [u32; PMS_WORDS] = [
    0x00000001, 0x000000ff, 0x00000001, 0x00000001, 0x00000001, 0x000007ff, 0x0003ffff, 0x0000000f,
    0x0000007f, 0x00000001, 0x00000001, 0x0000000f, 0x00000001, 0x00000003, 0x00000001, 0x00000fff,
    0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff,
    0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff,
    0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff,
    0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000003, 0x01ffffff, 0x0001ffff,
    0x00000001, 0x003fffff, 0x003fffff, 0x003fffff, 0x003fffff, 0x003fffff, 0x00000001, 0x001fffff,
    0x001fffff, 0x00000001, 0x00000003, 0x1fffffff, 0x00000001, 0x00000003, 0x1fffffff, 0x00000001,
    0x0fffffff, 0x00000001, 0x00000003, 0x03ffffff, 0x0001ffff, 0x00000001, 0x00000003, 0x03ffffff,
    0x0001ffff, 0x00000001, 0xff33cfff, 0xffcffff3, 0x3cc3ffff, 0xffffffff, 0xff33cfff, 0xffcffff3,
    0x3cc3ffff, 0xffffffff, 0x003fffff, 0x00000fff, 0x003fffff, 0x00000fff, 0x003fffff, 0x00000fff,
    0x00000001, 0x003fffff, 0x003fffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff,
    0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x00000001,
    0x00000003, 0x000000ff, 0xffffffff, 0x00000003, 0x0000001f, 0xffffffff, 0x00000001, 0x00000001,
    0x00ffffff, 0x003fffff, 0x00000001, 0x00000001, 0x00000001, 0xff33cfff, 0xffcffff3, 0x3cc3ffff,
    0xffffffff, 0xff33cfff, 0xffcffff3, 0x3cc3ffff, 0xffffffff, 0x003fffff, 0x00000fff, 0x003fffff,
    0x00000fff, 0x003fffff, 0x00000fff, 0x00000001, 0x003fffff, 0x003fffff, 0x3fffffff, 0x3fffffff,
    0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff,
    0x3fffffff, 0x3fffffff, 0x00000001, 0x00000003, 0x000000ff, 0xffffffff, 0x00000003, 0x0000001f,
    0xffffffff, 0x00000001, 0x00000001, 0x00ffffff, 0x003fffff, 0x00000001, 0x00000001, 0x00000001,
    0xff33cfff, 0xffcffff3, 0x3cc3ffff, 0xffffffff, 0x000007ff, 0x0000003f, 0x00000001, 0x00000003,
    0x0000007f, 0xffffffff, 0x00000001, 0x00003fff, 0x00003fff, 0x00003fff, 0x00000001, 0x0000000f,
    0x00000001, 0x0000000f, 0x00000001, 0x0000000f, 0x00000001, 0x0000000f, 0x00000001, 0x0000000f,
    0x00000001, 0x0000000f, 0x00000001, 0x0000000f, 0x00000001, 0x0000000f, 0x00000001, 0x0000000f,
    0x00000001, 0x0000000f, 0x00000001, 0x00000001,
];

#[allow(clippy::unreadable_literal)]
const PMS_WRITE: [u32; PMS_WORDS] = [
    0x00000001, 0x000000ff, 0x00000001, 0x00000001, 0x00000001, 0x000007ff, 0x0003ffff, 0x0000000f,
    0x0000007f, 0x00000001, 0x00000001, 0x0000000f, 0x00000001, 0x00000003, 0x00000001, 0x00000fff,
    0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff,
    0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff,
    0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000fff,
    0x00000001, 0x00000fff, 0x00000001, 0x00000fff, 0x00000001, 0x00000003, 0x00000000, 0x00000000,
    0x00000001, 0x003fffff, 0x003fffff, 0x003fffff, 0x003fffff, 0x003fffff, 0x00000001, 0x001fffff,
    0x001fffff, 0x00000001, 0x00000003, 0x00000000, 0x00000001, 0x00000003, 0x00000000, 0x00000001,
    0x0fffffff, 0x00000001, 0x00000003, 0x00000000, 0x00000000, 0x00000001, 0x00000003, 0x00000000,
    0x00000000, 0x00000001, 0xff33cfff, 0xffcffff3, 0x3cc3ffff, 0xffffffff, 0xff33cfff, 0xffcffff3,
    0x3cc3ffff, 0xffffffff, 0x003fffff, 0x00000fff, 0x003fffff, 0x00000fff, 0x003fffff, 0x00000fff,
    0x00000001, 0x003fffff, 0x003fffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff,
    0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x00000001,
    0x00000003, 0x00000000, 0x00000000, 0x00000003, 0x00000000, 0x00000000, 0x00000001, 0x00000001,
    0x00ffffff, 0x003fffff, 0x00000001, 0x00000001, 0x00000001, 0xff33cfff, 0xffcffff3, 0x3cc3ffff,
    0xffffffff, 0xff33cfff, 0xffcffff3, 0x3cc3ffff, 0xffffffff, 0x003fffff, 0x00000fff, 0x003fffff,
    0x00000fff, 0x003fffff, 0x00000fff, 0x00000001, 0x003fffff, 0x003fffff, 0x3fffffff, 0x3fffffff,
    0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff, 0x3fffffff,
    0x3fffffff, 0x3fffffff, 0x00000001, 0x00000003, 0x00000000, 0x00000000, 0x00000003, 0x00000000,
    0x00000000, 0x00000001, 0x00000001, 0x00ffffff, 0x003fffff, 0x00000001, 0x00000001, 0x00000001,
    0xff33cfff, 0xffcffff3, 0x3cc3ffff, 0xffffffff, 0x000007ff, 0x0000003f, 0x00000001, 0x00000003,
    0x00000000, 0x00000000, 0x00000001, 0x00003fff, 0x00003fff, 0x00003fff, 0x00000001, 0x0000000f,
    0x00000001, 0x0000000f, 0x00000001, 0x0000000f, 0x00000001, 0x0000000f, 0x00000001, 0x0000000f,
    0x00000001, 0x0000000f, 0x00000001, 0x0000000f, 0x00000001, 0x0000000f, 0x00000001, 0x0000000f,
    0x00000001, 0x0000000f, 0x00000001, 0x00000001,
];

/// Security world encoded by ESP32-S3 PMS monitor status fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32S3World {
    /// Secure World (World0).
    Secure,
    /// Non-secure World (World1).
    NonSecure,
}

impl Esp32S3World {
    const fn status(self) -> u32 {
        match self {
            Self::Secure => 1,
            Self::NonSecure => 2,
        }
    }

    const fn is_nonsecure(self) -> bool {
        matches!(self, Self::NonSecure)
    }
}

/// GDMA-capable peripheral selected by the PMS permission tables.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Esp32S3DmaPeripheral {
    /// General-purpose SPI2.
    Spi2,
    /// General-purpose SPI3.
    Spi3,
    /// UHCI0.
    Uhci0,
    /// I2S0.
    I2s0,
    /// I2S1.
    I2s1,
    /// AES accelerator.
    Aes,
    /// SHA accelerator.
    Sha,
    /// ADC/DAC controller.
    AdcDac,
    /// RMT.
    Rmt,
    /// LCD/CAM controller.
    LcdCam,
    /// USB OTG.
    Usb,
    /// SD/MMC host.
    Sdio,
}

impl Esp32S3DmaPeripheral {
    const fn internal_permission_offset(self) -> u64 {
        match self {
            Self::Spi2 => 0x3c,
            Self::Spi3 => 0x44,
            Self::Uhci0 => 0x4c,
            Self::I2s0 => 0x54,
            Self::I2s1 => 0x5c,
            Self::Aes => 0x74,
            Self::Sha => 0x7c,
            Self::AdcDac => 0x84,
            Self::Rmt => 0x8c,
            Self::LcdCam => 0x94,
            Self::Usb => 0x9c,
            Self::Sdio => 0xac,
        }
    }

    const fn external_permission_offset(self) -> Option<u64> {
        match self {
            Self::Spi2 => Some(0x2bc),
            Self::Spi3 => Some(0x2c4),
            Self::Uhci0 => Some(0x2cc),
            Self::I2s0 => Some(0x2d4),
            Self::I2s1 => Some(0x2dc),
            Self::LcdCam => Some(0x2e4),
            Self::Aes => Some(0x2ec),
            Self::Sha => Some(0x2f4),
            Self::AdcDac => Some(0x2fc),
            Self::Rmt => Some(0x304),
            Self::Usb | Self::Sdio => None,
        }
    }
}

#[derive(Clone)]
struct Esp32S3PmsState {
    registers: [u32; PMS_WORDS],
    date: u32,
}

impl Esp32S3PmsState {
    const fn new() -> Self {
        Self {
            registers: PMS_RESET,
            date: DATE_RESET,
        }
    }

    fn register(&self, offset: u64) -> u32 {
        if offset == DATE_OFFSET {
            self.date
        } else {
            self.registers[pms_index(offset)]
        }
    }

    fn set_register(&mut self, offset: u64, value: u32) {
        if offset == DATE_OFFSET {
            self.date = value;
        } else {
            self.registers[pms_index(offset)] = value;
        }
    }

    fn latch_pif(
        &mut self,
        core: u8,
        world: Esp32S3World,
        address: u32,
        width: AccessWidth,
        access: AccessKind,
    ) {
        let base = pif_monitor_base(core);
        let status = base + 8;
        if self.register(base + 4) & 2 != 0 && self.register(status) & 1 == 0 {
            let hsize = width_code(width);
            let hwrite = u32::from(access == AccessKind::Write);
            let hport = u32::from(access != AccessKind::Execute);
            self.set_register(
                status,
                (world.status() << 6) | (hwrite << 5) | (hsize << 2) | (hport << 1) | 1,
            );
            self.set_register(base + 12, address);
        }
    }

    fn latch_pif_size(&mut self, core: u8, world: Esp32S3World, address: u32, width: AccessWidth) {
        let base = pif_monitor_base(core);
        let control = base + 16;
        let status = base + 20;
        if self.register(control) & 2 != 0 && self.register(status) & 1 == 0 {
            self.set_register(status, (world.status() << 3) | (width_code(width) << 1) | 1);
            self.set_register(base + 24, address);
        }
    }

    fn latch_internal(
        &mut self,
        core: u8,
        world: Esp32S3World,
        address: u32,
        width: AccessWidth,
        access: AccessKind,
        instruction_bus: bool,
    ) {
        let base = if instruction_bus {
            0xe4 + u64::from(core) * 0x0c
        } else {
            0x104 + u64::from(core) * 0x10
        };
        if self.register(base + 4) & 2 == 0 || self.register(base + 8) & 1 != 0 {
            return;
        }
        if instruction_bus {
            let load_store = u32::from(access != AccessKind::Execute);
            let write = u32::from(access == AccessKind::Write);
            self.set_register(
                base + 8,
                (address & 0x1fff_ffe0)
                    | (world.status() << 3)
                    | (load_store << 2)
                    | (write << 1)
                    | 1,
            );
        } else {
            self.set_register(
                base + 8,
                (address & 0x03ff_fff0) | (world.status() << 2) | 1,
            );
            let byte_mask = ((1_u32 << u32::from(width.bytes())) - 1) << (address & 3);
            self.set_register(
                base + 12,
                (byte_mask << 1) | u32::from(access == AccessKind::Write),
            );
        }
    }

    fn latch_dma(
        &mut self,
        world: Esp32S3World,
        address: u32,
        width: AccessWidth,
        access: AccessKind,
    ) {
        if self.register(0xb4) & 2 == 0 || self.register(0xb8) & 1 != 0 {
            return;
        }
        self.set_register(0xb8, (address & 0x01ff_fff8) | (world.status() << 1) | 1);
        let byte_mask = ((1_u32 << u32::from(width.bytes())) - 1) << (address & 3);
        self.set_register(
            0xbc,
            (byte_mask << 1) | u32::from(access == AccessKind::Write),
        );
    }
}

/// CPU, interrupt-matrix, and DMA-facing view of the PMS configuration.
#[derive(Clone)]
pub struct Esp32S3PmsHandle {
    state: Rc<RefCell<Esp32S3PmsState>>,
}

impl Esp32S3PmsHandle {
    /// Checks a CPU access and latches the appropriate first-fault monitor on rejection.
    pub fn check_cpu_access(
        &self,
        core: u8,
        world: Esp32S3World,
        address: u32,
        width: AccessWidth,
        access: AccessKind,
    ) -> bool {
        let mut state = self.state.borrow_mut();
        if let Some(allowed) = internal_memory_allowed(&state, world, address, access) {
            if !allowed {
                state.latch_internal(
                    core,
                    world,
                    address,
                    width,
                    access,
                    is_instruction_bus_address(address),
                );
            }
            return allowed;
        }
        if let Some(allowed) = rtc_memory_allowed(&state, core, world, address, access) {
            if !allowed {
                state.latch_pif(core, world, address, width, access);
            }
            return allowed;
        }
        let Some((register, shift)) = peripheral_permission(address) else {
            return true;
        };
        if width != AccessWidth::Word {
            state.latch_pif_size(core, world, address, width);
            return false;
        }
        let core_base = 0x124 + u64::from(core.min(1)) * 0xac;
        let world_register = register + if world.is_nonsecure() { 4 } else { 0 };
        let permission = (state.register(core_base + u64::from(world_register) * 4) >> shift) & 3;
        let allowed = pair_allows(permission, access);
        if !allowed {
            state.latch_pif(core, world, address, width, access);
        }
        allowed
    }

    /// Checks a GDMA access to internal or external SRAM.
    pub fn check_dma_access(
        &self,
        peripheral: Esp32S3DmaPeripheral,
        world: Esp32S3World,
        address: u32,
        width: AccessWidth,
        access: AccessKind,
    ) -> bool {
        let mut state = self.state.borrow_mut();
        let allowed = if let Some(region) = internal_sram_region(&state, address) {
            let usage = state.register(0x14);
            let allocated = region.block.is_none_or(|block| usage & (1 << block) != 0);
            let permission =
                (state.register(peripheral.internal_permission_offset()) >> region.dma_shift()) & 3;
            allocated && pair_allows(permission, access)
        } else if (0x3c00_0000..0x3e00_0000).contains(&address) {
            external_dma_allowed(&state, peripheral, address, access)
        } else {
            true
        };
        if !allowed && !(0x3c00_0000..0x3e00_0000).contains(&address) {
            state.latch_dma(world, address, width, access);
        }
        allowed
    }

    /// Returns whether one of interrupt-matrix PMS sources 84 through 93 is asserted.
    pub fn interrupt_pending(&self, source: usize) -> bool {
        let state = self.state.borrow();
        let (control, status) = match source {
            84 => (0xb4, 0xb8),
            85 => (0xe8, 0xec),
            86 => (0x108, 0x10c),
            87 => (0x1a0, 0x1a4),
            88 => (0x1ac, 0x1b0),
            89 => (0xf4, 0xf8),
            90 => (0x118, 0x11c),
            91 => (0x24c, 0x250),
            92 => (0x258, 0x25c),
            93 => (0x29c, 0x2a0),
            _ => return false,
        };
        state.register(control) & 2 != 0 && state.register(status) & 1 != 0
    }
}

#[derive(Clone, Copy)]
struct InternalSramRegion {
    block: Option<u8>,
    shift: u8,
    instruction_region: bool,
}

impl InternalSramRegion {
    const fn dma_shift(self) -> u8 {
        if self.instruction_region {
            0
        } else {
            self.shift
        }
    }
}

fn internal_sram_region(state: &Esp32S3PmsState, address: u32) -> Option<InternalSramRegion> {
    let canonical = match address {
        0x4037_8000..=0x403d_ffff => address - 0x006f_0000,
        0x3fc8_8000..=0x3fce_ffff => address,
        0x3fcf_0000..=0x3fcf_7fff => {
            return Some(InternalSramRegion {
                block: Some(9),
                shift: 8,
                instruction_region: false,
            });
        }
        0x3fcf_8000..=0x3fcf_ffff => {
            return Some(InternalSramRegion {
                block: Some(10),
                shift: 10,
                instruction_region: false,
            });
        }
        _ => return None,
    };
    let block = sram1_block(canonical);
    let main = split_line(state.register(0xc4)).unwrap_or(0x3fcf_0000);
    let instruction_region = canonical < main;
    let shift = if instruction_region {
        let line0 = split_line(state.register(0xc8)).unwrap_or(main);
        let line1 = split_line(state.register(0xcc)).unwrap_or(main);
        u8::try_from(region_index(canonical, line0, line1) * 2).expect("SRAM region shift fits")
    } else {
        let line0 = split_line(state.register(0xd0)).unwrap_or(main);
        let line1 = split_line(state.register(0xd4)).unwrap_or(main);
        2 + u8::try_from(region_index(canonical, line0, line1) * 2).expect("SRAM region shift fits")
    };
    Some(InternalSramRegion {
        block,
        shift,
        instruction_region,
    })
}

fn internal_memory_allowed(
    state: &Esp32S3PmsState,
    world: Esp32S3World,
    address: u32,
    access: AccessKind,
) -> Option<bool> {
    if (0x4000_0000..=0x4005_ffff).contains(&address) {
        let register = if world.is_nonsecure() { 0xdc } else { 0xe0 };
        return Some(triad_allows((state.register(register) >> 18) & 7, access));
    }
    if (0x3ff0_0000..=0x3ff1_ffff).contains(&address) {
        let shift = if world.is_nonsecure() { 26 } else { 24 };
        return Some(pair_allows((state.register(0x100) >> shift) & 3, access));
    }
    let sram0 = match address {
        0x4037_0000..=0x4037_3fff => Some((0_u8, 12_u8)),
        0x4037_4000..=0x4037_7fff => Some((1, 15)),
        _ => None,
    };
    if let Some((block, shift)) = sram0 {
        let register = if world.is_nonsecure() { 0xdc } else { 0xe0 };
        return Some(
            state.register(0x14) & (1 << block) != 0
                && triad_allows((state.register(register) >> shift) & 7, access),
        );
    }
    let region = internal_sram_region(state, address)?;
    let usage = region
        .block
        .is_none_or(|block| state.register(0x14) & (1 << block) != 0);
    let instruction_bus = is_instruction_bus_address(address);
    let allowed = if instruction_bus {
        let register = if world.is_nonsecure() { 0xdc } else { 0xe0 };
        let shift = if region.instruction_region {
            region.shift / 2 * 3
        } else {
            9
        };
        triad_allows((state.register(register) >> shift) & 7, access)
    } else {
        let world_base = if world.is_nonsecure() { 12 } else { 0 };
        let shift = if region.instruction_region {
            world_base
        } else {
            world_base + region.shift
        };
        pair_allows((state.register(0x100) >> shift) & 3, access)
    };
    Some(usage && allowed)
}

fn rtc_memory_allowed(
    state: &Esp32S3PmsState,
    core: u8,
    world: Esp32S3World,
    address: u32,
    access: AccessKind,
) -> Option<bool> {
    let base = 0x124 + u64::from(core.min(1)) * 0xac;
    let (offset, split_register, permission_register) = match address {
        0x600f_e000..=0x600f_ffff => (address - 0x600f_e000, 9_u64, 10_u64),
        0x5000_0000..=0x5000_1fff => (address - 0x5000_0000, 11, 12),
        0x6002_1000..=0x6002_2fff => (address - 0x6002_1000, 13, 14),
        _ => return None,
    };
    let split_value = state.register(base + split_register * 4);
    let split_shift = if world.is_nonsecure() { 11 } else { 0 };
    let split = ((split_value >> split_shift) & 0x7ff) << 2;
    let high = offset >= split;
    let world_shift = if world.is_nonsecure() { 6 } else { 0 };
    let region_shift = if high { 3 } else { 0 };
    let permission =
        (state.register(base + permission_register * 4) >> (world_shift + region_shift)) & 7;
    Some(triad_allows(permission, access))
}

fn external_dma_allowed(
    state: &Esp32S3PmsState,
    peripheral: Esp32S3DmaPeripheral,
    address: u32,
    access: AccessKind,
) -> bool {
    let boundaries = [
        0x3c00_0000 + (state.register(0x2ac) << 12),
        0x3c00_0000 + (state.register(0x2b0) << 12),
        0x3c00_0000 + (state.register(0x2b4) << 12),
    ];
    let region = if address < boundaries[0] {
        0
    } else if address < boundaries[1] {
        1
    } else if address < boundaries[2] {
        2
    } else {
        3
    };
    let Some(offset) = peripheral.external_permission_offset() else {
        return false;
    };
    match region {
        1 | 2 => pair_allows((state.register(offset) >> ((region - 1) * 2)) & 3, access),
        _ => false,
    }
}

fn pms_index(offset: u64) -> usize {
    usize::try_from(offset / 4).expect("PMS register index fits usize")
}

fn documented_offset(offset: u64) -> bool {
    offset == DATE_OFFSET || (offset <= 0x30c && offset.is_multiple_of(4))
}

fn read_mask(offset: u64) -> u32 {
    if offset == DATE_OFFSET {
        DATE_MASK
    } else {
        PMS_READ[pms_index(offset)]
    }
}

fn write_mask(offset: u64) -> u32 {
    if offset == DATE_OFFSET {
        DATE_MASK
    } else {
        PMS_WRITE[pms_index(offset)]
    }
}

fn is_lock(offset: u64) -> bool {
    matches!(
        offset,
        0x000
            | 0x008
            | 0x010
            | 0x028
            | 0x030
            | 0x0b0
            | 0x0c0
            | 0x0d8
            | 0x0e4
            | 0x0f0
            | 0x0fc
            | 0x104
            | 0x114
            | 0x124
            | 0x160
            | 0x19c
            | 0x1b8
            | 0x1c8
            | 0x1d0
            | 0x20c
            | 0x248
            | 0x264
            | 0x274
            | 0x27c
            | 0x298
            | 0x2a8
            | 0x2b8
            | 0x2c0
            | 0x2c8
            | 0x2d0
            | 0x2d8
            | 0x2e0
            | 0x2e8
            | 0x2f0
            | 0x2f8
            | 0x300
    ) || (0x038..=0x0a8).contains(&offset) && (offset - 0x038).is_multiple_of(8)
}

fn controlling_lock(offset: u64) -> Option<u64> {
    match offset {
        0x004 => Some(0x000),
        0x00c => Some(0x008),
        0x014..=0x020 => Some(0x010),
        0x02c => Some(0x028),
        0x034 => Some(0x030),
        0x03c..=0x0ac if (offset - 0x03c).is_multiple_of(8) => Some(offset - 4),
        0x0b4..=0x0bc => Some(0x0b0),
        0x0c4..=0x0d4 => Some(0x0c0),
        0x0dc..=0x0e0 => Some(0x0d8),
        0x0e8..=0x0ec => Some(0x0e4),
        0x0f4..=0x0f8 => Some(0x0f0),
        0x100 => Some(0x0fc),
        0x108..=0x110 => Some(0x104),
        0x118..=0x120 => Some(0x114),
        0x128..=0x15c => Some(0x124),
        0x164..=0x198 => Some(0x160),
        0x1a0..=0x1b4 => Some(0x19c),
        0x1bc..=0x1c4 => Some(0x1b8),
        0x1cc => Some(0x1c8),
        0x1d4..=0x208 => Some(0x1d0),
        0x210..=0x244 => Some(0x20c),
        0x24c..=0x260 => Some(0x248),
        0x268..=0x270 => Some(0x264),
        0x278 => Some(0x274),
        0x280..=0x294 => Some(0x27c),
        0x29c..=0x2a4 => Some(0x298),
        0x2ac..=0x2b4 => Some(0x2a8),
        0x2bc..=0x304 if (offset - 0x2bc).is_multiple_of(8) => Some(offset - 4),
        _ => None,
    }
}

fn pif_monitor_base(core: u8) -> u64 {
    0x19c + u64::from(core.min(1)) * 0xac
}

fn peripheral_permission(address: u32) -> Option<(u8, u8)> {
    let page = address & 0xffff_f000;
    match page {
        0x6000_0000 => Some((1, 0)),
        0x6000_2000 => Some((1, 2)),
        0x6000_3000 => Some((1, 4)),
        0x6000_4000 => Some((1, 6)),
        0x6000_7000 => Some((1, 14)),
        0x6000_9000 => Some((1, 16)),
        0x6000_f000 => Some((1, 28)),
        0x6001_0000 => Some((1, 30)),
        0x6001_3000 => Some((2, 4)),
        0x6001_4000 => Some((2, 6)),
        0x6001_6000 => Some((2, 10)),
        0x6001_7000 => Some((2, 12)),
        0x6001_9000 => Some((2, 16)),
        0x6001_e000 => Some((2, 24)),
        0x6001_f000 => Some((2, 26)),
        0x6002_0000 => Some((2, 28)),
        0x6002_3000 => Some((2, 30)),
        0x6002_4000 => Some((3, 0)),
        0x6002_5000 => Some((3, 2)),
        0x6002_6000 => Some((3, 4)),
        0x6002_7000 => Some((3, 6)),
        0x6002_8000 => Some((3, 8)),
        0x6002_b000 => Some((3, 10)),
        0x6002_c000 => Some((3, 12)),
        0x6002_d000 => Some((3, 14)),
        0x6002_e000 => Some((3, 16)),
        0x6003_8000 => Some((4, 0)),
        0x6003_9000 => Some((4, 2)),
        0x6003_a000..=0x6003_e000 => Some((4, 4)),
        0x6003_f000 => Some((4, 6)),
        0x6004_0000 => Some((4, 8)),
        0x6004_1000 => Some((4, 10)),
        0x6008_0000..=0x6008_f000 => Some((4, 14)),
        0x600c_0000 | 0x600c_5000 => Some((4, 16)),
        0x600c_1000 => Some((4, 18)),
        0x600c_2000 => Some((4, 20)),
        0x600c_4000 => Some((4, 24)),
        0x600d_0000 => Some((4, 30)),
        _ => None,
    }
}

fn pair_allows(permission: u32, access: AccessKind) -> bool {
    match access {
        AccessKind::Read => permission & 2 != 0,
        AccessKind::Write => permission & 1 != 0,
        AccessKind::Execute => false,
    }
}

fn triad_allows(permission: u32, access: AccessKind) -> bool {
    let bit = match access {
        AccessKind::Read => 0,
        AccessKind::Write => 1,
        AccessKind::Execute => 2,
    };
    permission & (1 << bit) != 0
}

fn width_code(width: AccessWidth) -> u32 {
    match width {
        AccessWidth::Byte => 0,
        AccessWidth::HalfWord => 1,
        AccessWidth::Word => 2,
        AccessWidth::DoubleWord => 3,
    }
}

fn is_instruction_bus_address(address: u32) -> bool {
    (0x4000_0000..=0x4005_ffff).contains(&address) || (0x4037_0000..=0x403d_ffff).contains(&address)
}

fn sram1_block(address: u32) -> Option<u8> {
    match address {
        0x3fc8_8000..=0x3fc8_ffff => Some(2),
        0x3fc9_0000..=0x3fc9_ffff => Some(3),
        0x3fca_0000..=0x3fca_ffff => Some(4),
        0x3fcb_0000..=0x3fcb_ffff => Some(5),
        0x3fcc_0000..=0x3fcc_ffff => Some(6),
        0x3fcd_0000..=0x3fcd_ffff => Some(7),
        0x3fce_0000..=0x3fce_ffff => Some(8),
        _ => None,
    }
}

fn split_line(value: u32) -> Option<u32> {
    const BLOCK_STARTS: [u32; 7] = [
        0x3fc8_8000,
        0x3fc9_0000,
        0x3fca_0000,
        0x3fcb_0000,
        0x3fcc_0000,
        0x3fcd_0000,
        0x3fce_0000,
    ];
    let block = (0..7).find(|index| matches!((value >> (index * 2)) & 3, 1 | 2))?;
    Some(BLOCK_STARTS[block] + ((value >> 14) & 0xff) * 0x100)
}

fn region_index(address: u32, first: u32, second: u32) -> usize {
    let (low, high) = if first <= second {
        (first, second)
    } else {
        (second, first)
    };
    usize::from(address >= low) + usize::from(address >= high)
}

/// Functional ESP32-S3 Permission Control register block.
pub struct Esp32S3Pms {
    name: String,
    state: Rc<RefCell<Esp32S3PmsState>>,
}

impl Esp32S3Pms {
    /// Creates reset PMS state and its access-checking handle.
    pub fn new(name: impl Into<String>) -> (Self, Esp32S3PmsHandle) {
        let state = Rc::new(RefCell::new(Esp32S3PmsState::new()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3PmsHandle { state },
        )
    }
}

impl Device for Esp32S3Pms {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !documented_offset(offset) {
            return Err(DeviceError::new(format!(
                "{} requires aligned word access to a documented PMS register at {offset:#x}",
                self.name
            )));
        }
        Ok(u64::from(
            self.state.borrow().register(offset) & read_mask(offset),
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !documented_offset(offset) {
            return Err(DeviceError::new(format!(
                "{} requires aligned word access to a documented PMS register at {offset:#x}",
                self.name
            )));
        }
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new(format!("{} word write exceeds 32 bits", self.name)))?;
        let mut state = self.state.borrow_mut();
        if controlling_lock(offset).is_some_and(|lock| state.register(lock) & 1 != 0) {
            return Ok(());
        }
        for (control, status_offsets) in [
            (0xb4, &[0xb8, 0xbc][..]),
            (0xe8, &[0xec][..]),
            (0xf4, &[0xf8][..]),
            (0x108, &[0x10c, 0x110][..]),
            (0x118, &[0x11c, 0x120][..]),
            (0x1a0, &[0x1a4, 0x1a8][..]),
            (0x1ac, &[0x1b0, 0x1b4][..]),
            (0x24c, &[0x250, 0x254][..]),
            (0x258, &[0x25c, 0x260][..]),
            (0x29c, &[0x2a0, 0x2a4][..]),
        ] {
            if offset == control && value & 1 != 0 {
                for status in status_offsets {
                    state.set_register(*status, 0);
                }
            }
        }
        let old = state.register(offset);
        let mask = write_mask(offset);
        let value = (old & !mask) | (value & mask);
        state.set_register(offset, if is_lock(offset) { old | value } else { value });
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = Esp32S3PmsState::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(device: &mut Esp32S3Pms, offset: u64) -> u32 {
        u32::try_from(
            device
                .read(offset, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
        )
        .unwrap()
    }

    fn write(device: &mut Esp32S3Pms, offset: u64, value: u32) {
        device
            .write(offset, AccessWidth::Word, u64::from(value), SimTime::ZERO)
            .unwrap();
    }

    #[test]
    fn all_197_vendor_registers_have_exact_reset_and_write_masks() {
        let (mut device, _) = Esp32S3Pms::new("pms");
        let mut count = 0;
        for offset in (0..0x1000).step_by(4) {
            if documented_offset(offset) {
                count += 1;
                let expected = if offset == DATE_OFFSET {
                    DATE_RESET
                } else {
                    PMS_RESET[pms_index(offset)]
                };
                assert_eq!(read(&mut device, offset), expected & read_mask(offset));
                let (mut isolated, _) = Esp32S3Pms::new("pms");
                write(&mut isolated, offset, u32::MAX);
                assert_eq!(
                    read(&mut isolated, offset),
                    ((expected & !write_mask(offset)) | write_mask(offset)) & read_mask(offset),
                    "write mask mismatch at {offset:#x}"
                );
            } else {
                assert!(
                    device
                        .read(offset, AccessWidth::Word, SimTime::ZERO)
                        .is_err()
                );
            }
        }
        assert_eq!(count, 197);
    }

    #[test]
    fn lock_domains_are_sticky_and_reset_reopens_them() {
        let (mut device, _) = Esp32S3Pms::new("pms");
        write(&mut device, 0x128, 0);
        write(&mut device, 0x124, 1);
        write(&mut device, 0x128, u32::MAX);
        write(&mut device, 0x124, 0);
        assert_eq!(read(&mut device, 0x128), 0);
        assert_eq!(read(&mut device, 0x124), 1);
        device.reset(ResetKind::Software);
        assert_eq!(read(&mut device, 0x124), 0);
        assert_eq!(read(&mut device, 0x128), PMS_RESET[pms_index(0x128)]);
    }

    #[test]
    fn cpu_peripheral_permissions_latch_first_fault_and_clear_interrupts() {
        let (mut device, handle) = Esp32S3Pms::new("pms");
        assert!(handle.check_cpu_access(
            0,
            Esp32S3World::Secure,
            0x6000_0000,
            AccessWidth::Word,
            AccessKind::Write,
        ));
        let uart_permissions = read(&mut device, 0x128);
        write(&mut device, 0x128, uart_permissions & !3);
        assert!(!handle.check_cpu_access(
            0,
            Esp32S3World::Secure,
            0x6000_0000,
            AccessWidth::Word,
            AccessKind::Write,
        ));
        assert!(handle.interrupt_pending(87));
        assert_eq!(read(&mut device, 0x1a8), 0x6000_0000);
        assert_eq!(read(&mut device, 0x1a4), 0x6b);
        write(&mut device, 0x1a0, 3);
        assert!(!handle.interrupt_pending(87));
        assert_eq!(read(&mut device, 0x1a8), 0);

        // World1 has an independent four-register table after World0's.
        write(&mut device, 0x128, uart_permissions);
        let nonsecure_uart_permissions = read(&mut device, 0x138);
        write(&mut device, 0x138, nonsecure_uart_permissions & !1);
        assert!(handle.check_cpu_access(
            0,
            Esp32S3World::Secure,
            0x6000_0000,
            AccessWidth::Word,
            AccessKind::Write,
        ));
        assert!(!handle.check_cpu_access(
            0,
            Esp32S3World::NonSecure,
            0x6000_0000,
            AccessWidth::Word,
            AccessKind::Write,
        ));
    }

    #[test]
    fn internal_memory_rtc_alignment_and_dma_permissions_are_enforced() {
        let (mut device, handle) = Esp32S3Pms::new("pms");
        let iram_permissions = read(&mut device, 0xe0);
        write(&mut device, 0xe0, iram_permissions & !(7 << 18));
        assert!(!handle.check_cpu_access(
            0,
            Esp32S3World::Secure,
            0x4000_0100,
            AccessWidth::Word,
            AccessKind::Execute,
        ));
        assert!(handle.interrupt_pending(85));

        assert!(!handle.check_cpu_access(
            1,
            Esp32S3World::Secure,
            0x6000_0000,
            AccessWidth::Byte,
            AccessKind::Read,
        ));
        assert!(handle.interrupt_pending(92));

        write(&mut device, 0x2ac, 0x100);
        write(&mut device, 0x2b0, 0x200);
        write(&mut device, 0x2b4, 0x300);
        write(&mut device, 0x2bc, 2);
        assert!(handle.check_dma_access(
            Esp32S3DmaPeripheral::Spi2,
            Esp32S3World::Secure,
            0x3c10_0000,
            AccessWidth::Word,
            AccessKind::Read,
        ));
        assert!(!handle.check_dma_access(
            Esp32S3DmaPeripheral::Spi2,
            Esp32S3World::Secure,
            0x3c10_0000,
            AccessWidth::Word,
            AccessKind::Write,
        ));
    }

    #[test]
    fn sram_split_regions_use_the_cpu_and_dma_specific_permission_layouts() {
        let (mut device, handle) = Esp32S3Pms::new("pms");
        // Main split at 0x3fca0000; instruction-region sub-splits at
        // 0x3fc90000 and 0x3fc98000.
        write(&mut device, 0xc4, 1 << 4);
        write(&mut device, 0xc8, 1 << 2);
        write(&mut device, 0xcc, (0x80 << 14) | (1 << 2));

        let ibus_permissions = read(&mut device, 0xe0);
        write(&mut device, 0xe0, ibus_permissions & !(7 << 3));
        assert!(handle.check_cpu_access(
            0,
            Esp32S3World::Secure,
            0x4037_8000,
            AccessWidth::Word,
            AccessKind::Execute,
        ));
        assert!(!handle.check_cpu_access(
            0,
            Esp32S3World::Secure,
            0x4038_2000,
            AccessWidth::Word,
            AccessKind::Execute,
        ));
        assert!(handle.check_cpu_access(
            0,
            Esp32S3World::Secure,
            0x4038_a000,
            AccessWidth::Word,
            AccessKind::Execute,
        ));

        // DBUS and DMA each use one permission pair for the entire
        // instruction region, regardless of the IBUS sub-region.
        let dbus_permissions = read(&mut device, 0x100);
        write(&mut device, 0x100, dbus_permissions & !3);
        assert!(!handle.check_cpu_access(
            0,
            Esp32S3World::Secure,
            0x3fc9_2000,
            AccessWidth::Word,
            AccessKind::Read,
        ));
        assert!(handle.check_cpu_access(
            0,
            Esp32S3World::Secure,
            0x3fca_1000,
            AccessWidth::Word,
            AccessKind::Read,
        ));

        let spi2_permissions = read(&mut device, 0x3c);
        write(&mut device, 0x3c, spi2_permissions & !3);
        assert!(!handle.check_dma_access(
            Esp32S3DmaPeripheral::Spi2,
            Esp32S3World::Secure,
            0x3fc9_a000,
            AccessWidth::Word,
            AccessKind::Read,
        ));
        assert!(handle.check_dma_access(
            Esp32S3DmaPeripheral::Spi2,
            Esp32S3World::Secure,
            0x3fca_1000,
            AccessWidth::Word,
            AccessKind::Read,
        ));
    }
}
