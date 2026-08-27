use super::*;

const REGISTER_COUNT: usize = 0xe8 / 4 + 1;
const LOCK: usize = 0x00 / 4;
const FORCE_CORE_NS: usize = 0x04 / 4;
const CFGRESET: usize = 0x08 / 4;
const GPIO_NSMASK0: usize = 0x0c / 4;
const GPIO_NSMASK1: usize = 0x10 / 4;
const LOCK_MASK: u32 = 0x0f;
const DMA_LOCK: u32 = 1 << 2;

/// RP2350 ACCESSCTRL register offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rp2350AccessCtrlRegister {
    /// Monotonic master lock bits.
    Lock,
    /// Force core 1 accesses to non-secure.
    ForceCoreNs,
    /// Self-clearing configuration reset command.
    CfgReset,
    /// GPIO0..31 non-secure mask.
    GpioNsMask0,
    /// GPIO32..47 and QSPI/USB non-secure mask.
    GpioNsMask1,
    /// Eight-bit permission mask for a named peripheral slot.
    Peripheral(u8),
}

impl TryFrom<u64> for Rp2350AccessCtrlRegister {
    type Error = DeviceError;

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        let register = match offset {
            0x00 => Self::Lock,
            0x04 => Self::ForceCoreNs,
            0x08 => Self::CfgReset,
            0x0c => Self::GpioNsMask0,
            0x10 => Self::GpioNsMask1,
            0x14..=0xe8 if (offset - 0x14) % 4 == 0 => {
                Self::Peripheral(u8::try_from((offset - 0x14) / 4).expect("ACCESSCTRL slot fits"))
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "invalid RP2350 ACCESSCTRL register offset {offset:#x}"
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
        _ => Err(DeviceError::new("invalid RP2350 ACCESSCTRL atomic alias")),
    }
}

/// Bus master represented by an RP2350 ACCESSCTRL permission bit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Rp2350AccessMaster {
    /// Processor core 0.
    Core0,
    /// Processor core 1.
    Core1,
    /// DMA bus master.
    Dma,
    /// Debugger bus master.
    Debugger,
}

impl Rp2350AccessMaster {
    const fn permission_bit(self) -> u8 {
        match self {
            Self::Core0 => 1 << 4,
            Self::Core1 => 1 << 5,
            Self::Dma => 1 << 6,
            Self::Debugger => 1 << 7,
        }
    }

    const fn lock_bit(self) -> u32 {
        match self {
            Self::Core0 => 1,
            Self::Core1 => 2,
            Self::Dma => 4,
            Self::Debugger => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Rp2350AccessContext {
    master: Rp2350AccessMaster,
    secure: bool,
    privileged: bool,
}

struct Rp2350AccessCtrlState {
    reset: [u32; REGISTER_COUNT],
    registers: [u32; REGISTER_COUNT],
}

/// Machine-facing RP2350 access-policy and attribution handle.
#[derive(Clone)]
pub struct Rp2350AccessCtrlHandle {
    state: Rc<RefCell<Rp2350AccessCtrlState>>,
    context: Rc<RefCell<Rp2350AccessContext>>,
}

impl Rp2350AccessCtrlHandle {
    /// Selects the bus-master security context for subsequent accesses.
    pub fn set_context(&self, master: Rp2350AccessMaster, secure: bool, privileged: bool) {
        *self.context.borrow_mut() = Rp2350AccessContext {
            master,
            secure,
            privileged,
        };
    }

    /// Enforces the native ACCESSCTRL permission for one address.
    pub fn check_address(&self, address: u64) -> Result<(), String> {
        // ACCESSCTRL itself has contextual write rules in the device, while
        // SIO/private-peripheral accesses are not governed by a slot.
        let Some(offset) = accessctrl_offset_for_address(address) else {
            return Ok(());
        };
        let state = self.state.borrow();
        let permission = state.registers[(offset / 4) as usize] as u8;
        let mut context = *self.context.borrow();
        if context.master == Rp2350AccessMaster::Core1 && state.registers[FORCE_CORE_NS] & 2 != 0 {
            context.secure = false;
        }
        let level_mask = match (context.secure, context.privileged) {
            (true, true) => 1 << 3,
            (true, false) => (1 << 3) | (1 << 2),
            (false, true) => 1 << 1,
            (false, false) => (1 << 1) | 1,
        };
        if permission & context.master.permission_bit() != 0
            && permission & level_mask == level_mask
        {
            Ok(())
        } else {
            Err(format!(
                "RP2350 ACCESSCTRL denied {:?} {}{} access to slot {offset:#04x} (permission {permission:#04x})",
                context.master,
                if context.secure { "S" } else { "NS" },
                if context.privileged { "P" } else { "U" },
            ))
        }
    }

    /// Returns whether a GPIO is attributed Non-secure by the native masks.
    pub fn gpio_is_nonsecure(&self, pin: u8) -> bool {
        let state = self.state.borrow();
        if pin < 32 {
            state.registers[GPIO_NSMASK0] & (1 << pin) != 0
        } else if pin < 48 {
            state.registers[GPIO_NSMASK1] & (1 << (pin - 32)) != 0
        } else {
            false
        }
    }
}

fn accessctrl_offset_for_address(address: u64) -> Option<u64> {
    let slot = match address {
        0x0000_0000..=0x0000_7fff => 0x14,
        0x1000_0000..=0x1fff_ffff => 0x18,
        0x2000_0000..=0x2007_ffff => 0x1c + ((address - 0x2000_0000) / 0x1_0000) * 4,
        0x2008_0000..=0x2008_0fff => 0x3c,
        0x2008_1000..=0x2008_1fff => 0x40,
        0x5000_0000..=0x5000_7fff => 0x44,
        0x5010_0000..=0x5011_3fff => 0x48,
        0x5020_0000..=0x5020_3fff => 0x4c,
        0x5030_0000..=0x5030_3fff => 0x50,
        0x5040_0000..=0x5040_3fff => 0x54,
        0x5070_0000..=0x5070_3fff => 0x58,
        0x4014_0000..=0x4014_ffff => 0x5c,
        0x4000_0000..=0x4000_3fff => 0x60,
        0x4000_8000..=0x4000_bfff => 0xbc,
        0x4001_0000..=0x4001_3fff => 0xc0,
        0x4001_8000..=0x4001_bfff => 0xdc,
        0x4002_0000..=0x4002_3fff => 0x64,
        0x4002_8000..=0x4002_bfff => 0x68,
        0x4003_0000..=0x4003_3fff => 0x6c,
        0x4003_8000..=0x4003_bfff => 0x70,
        0x4004_0000..=0x4004_3fff => 0x74,
        0x4004_8000..=0x4004_bfff => 0xc4,
        0x4005_0000..=0x4005_3fff => 0xcc,
        0x4005_8000..=0x4005_bfff => 0xd0,
        0x4006_8000..=0x4006_bfff => 0x78,
        0x4007_0000..=0x4007_3fff => 0xa0,
        0x4007_8000..=0x4007_bfff => 0xa4,
        0x4008_0000..=0x4008_3fff => 0x90,
        0x4008_8000..=0x4008_bfff => 0x94,
        0x4009_0000..=0x4009_3fff => 0x84,
        0x4009_8000..=0x4009_bfff => 0x88,
        0x400a_0000..=0x400a_3fff => 0x7c,
        0x400a_8000..=0x400a_bfff => 0x8c,
        0x400b_0000..=0x400b_3fff => 0x98,
        0x400b_8000..=0x400b_bfff => 0x9c,
        0x400c_0000..=0x400c_3fff | 0x5060_0000..=0x5060_0fff => 0x80,
        0x400c_8000..=0x400c_bfff => 0xe0,
        0x400d_0000..=0x400d_3fff => 0xe4,
        0x400d_8000..=0x400d_bfff => 0xd8,
        0x400e_8000..=0x400e_bfff => 0xc8,
        0x400f_0000..=0x400f_3fff => 0xb4,
        0x400f_8000..=0x400f_bfff => 0xb8,
        0x4010_0000..=0x4010_3fff => 0xb0,
        0x4010_8000..=0x4010_bfff => 0xd4,
        0x4012_0000..=0x4013_ffff => 0xa8,
        0x4016_0000..=0x4016_3fff => 0xac,
        0x5050_0000..=0x5050_3fff => 0xe8,
        _ => return None,
    };
    Some(slot)
}

/// RP2350 security-permission register block with an enforcing policy handle.
pub struct Rp2350AccessCtrl {
    name: String,
    state: Rc<RefCell<Rp2350AccessCtrlState>>,
    context: Rc<RefCell<Rp2350AccessContext>>,
}

impl Rp2350AccessCtrl {
    /// Creates the reset-state ACCESSCTRL block.
    pub fn new(name: impl Into<String>) -> Self {
        Self::new_with_handle(name).0
    }

    /// Creates the register block and its machine-facing enforcement handle.
    pub fn new_with_handle(name: impl Into<String>) -> (Self, Rp2350AccessCtrlHandle) {
        let mut reset = [0; REGISTER_COUNT];
        reset[LOCK] = 0x04;
        reset[GPIO_NSMASK0] = 0;
        reset[GPIO_NSMASK1] = 0;
        for register in reset.iter_mut().skip(5) {
            *register = 0xff;
        }
        let state = Rc::new(RefCell::new(Rp2350AccessCtrlState {
            registers: reset,
            reset,
        }));
        let context = Rc::new(RefCell::new(Rp2350AccessContext {
            master: Rp2350AccessMaster::Core0,
            secure: true,
            privileged: true,
        }));
        (
            Self {
                name: name.into(),
                state: state.clone(),
                context: context.clone(),
            },
            Rp2350AccessCtrlHandle { state, context },
        )
    }

    fn mask(register: Rp2350AccessCtrlRegister) -> u32 {
        match register {
            Rp2350AccessCtrlRegister::Lock => LOCK_MASK,
            Rp2350AccessCtrlRegister::ForceCoreNs => 0x02,
            Rp2350AccessCtrlRegister::CfgReset => 0x01,
            Rp2350AccessCtrlRegister::GpioNsMask0 => u32::MAX,
            Rp2350AccessCtrlRegister::GpioNsMask1 => 0xff00_ffff,
            Rp2350AccessCtrlRegister::Peripheral(_) => 0xff,
        }
    }

    /// Returns one peripheral's eight-bit security/privilege mask by offset.
    pub fn permission(&self, offset: u64) -> Option<u8> {
        let register = Rp2350AccessCtrlRegister::try_from(offset & 0x0fff).ok()?;
        let Rp2350AccessCtrlRegister::Peripheral(slot) = register else {
            return None;
        };
        let index = usize::from(slot) + 5;
        self.state
            .borrow()
            .registers
            .get(index)
            .copied()
            .map(|value| value as u8)
    }

    /// Returns the GPIO non-secure mask pair.
    pub fn gpio_nonsecure_masks(&self) -> (u32, u32) {
        let state = self.state.borrow();
        (state.registers[GPIO_NSMASK0], state.registers[GPIO_NSMASK1])
    }

    fn reset_configuration(state: &mut Rp2350AccessCtrlState) {
        for index in 3..REGISTER_COUNT {
            state.registers[index] = state.reset[index];
        }
    }
}

impl Device for Rp2350AccessCtrl {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2350 ACCESSCTRL requires aligned word access",
            ));
        }
        let register = offset & 0x0fff;
        let register = Rp2350AccessCtrlRegister::try_from(register)?;
        let index = match register {
            Rp2350AccessCtrlRegister::Lock => LOCK,
            Rp2350AccessCtrlRegister::ForceCoreNs => FORCE_CORE_NS,
            Rp2350AccessCtrlRegister::CfgReset => CFGRESET,
            Rp2350AccessCtrlRegister::GpioNsMask0 => GPIO_NSMASK0,
            Rp2350AccessCtrlRegister::GpioNsMask1 => GPIO_NSMASK1,
            Rp2350AccessCtrlRegister::Peripheral(slot) => usize::from(slot) + 5,
        };
        Ok(u64::from(self.state.borrow().registers[index]))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "RP2350 ACCESSCTRL requires aligned word access",
            ));
        }
        let register = offset & 0x0fff;
        let register = Rp2350AccessCtrlRegister::try_from(register)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits");
        let index = match register {
            Rp2350AccessCtrlRegister::Lock => LOCK,
            Rp2350AccessCtrlRegister::ForceCoreNs => FORCE_CORE_NS,
            Rp2350AccessCtrlRegister::CfgReset => CFGRESET,
            Rp2350AccessCtrlRegister::GpioNsMask0 => GPIO_NSMASK0,
            Rp2350AccessCtrlRegister::GpioNsMask1 => GPIO_NSMASK1,
            Rp2350AccessCtrlRegister::Peripheral(slot) => usize::from(slot) + 5,
        };
        let context = *self.context.borrow();
        let mut state = self.state.borrow_mut();
        let current = state.registers[index];
        if state.registers[LOCK] & context.master.lock_bit() != 0
            && !matches!(register, Rp2350AccessCtrlRegister::Lock)
        {
            return Err(DeviceError::new(format!(
                "RP2350 ACCESSCTRL configuration is locked for {:?}",
                context.master
            )));
        }
        let secure_privileged = context.secure && context.privileged;
        if matches!(register, Rp2350AccessCtrlRegister::CfgReset) {
            if !secure_privileged {
                return Err(DeviceError::new(
                    "RP2350 ACCESSCTRL CFGRESET requires Secure Privileged access",
                ));
            }
            if value & 1 != 0 {
                Self::reset_configuration(&mut state);
            }
            return Ok(());
        }
        if matches!(register, Rp2350AccessCtrlRegister::Lock) {
            if !secure_privileged {
                return Err(DeviceError::new(
                    "RP2350 ACCESSCTRL LOCK requires Secure Privileged access",
                ));
            }
            // DMA is permanently locked by hardware; the other lock bits are
            // write-once until a full ACCESSCTRL reset.
            state.registers[index] = (current | value | DMA_LOCK) & LOCK_MASK;
            return Ok(());
        }
        if !secure_privileged {
            if !context.secure
                && context.privileged
                && matches!(register, Rp2350AccessCtrlRegister::Peripheral(_))
                && current & 2 != 0
            {
                // NSP may delegate only the NSU bit.
                let updated = atomic_update(current & 1, (offset >> 12) & 3, value & 1)?;
                state.registers[index] = (current & !1) | updated;
                return Ok(());
            }
            return Err(DeviceError::new(
                "RP2350 ACCESSCTRL policy write requires Secure Privileged access",
            ));
        }
        let updated = atomic_update(current, (offset >> 12) & 3, value)?;
        state.registers[index] = updated & Self::mask(register);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        let reset = self.state.borrow().reset;
        self.state.borrow_mut().registers = reset;
        *self.context.borrow_mut() = Rp2350AccessContext {
            master: Rp2350AccessMaster::Core0,
            secure: true,
            privileged: true,
        };
    }
}
