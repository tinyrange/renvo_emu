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

/// RP2350 security-permission register block.
///
/// The model keeps the documented reset masks, atomic aliases, configuration
/// reset command, and monotonic lock bits. The bus does not yet carry a
/// secure/non-secure or privilege attribute, so these masks are observable
/// policy state rather than an access filter.
pub struct Rp2350AccessCtrl {
    name: String,
    reset: [u32; REGISTER_COUNT],
    registers: [u32; REGISTER_COUNT],
}

impl Rp2350AccessCtrl {
    /// Creates the reset-state ACCESSCTRL block.
    pub fn new(name: impl Into<String>) -> Self {
        let mut reset = [0; REGISTER_COUNT];
        reset[LOCK] = 0x04;
        reset[GPIO_NSMASK0] = 0;
        reset[GPIO_NSMASK1] = 0;
        for register in reset.iter_mut().skip(5) {
            *register = 0xff;
        }
        Self {
            name: name.into(),
            registers: reset,
            reset,
        }
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
        self.registers.get(index).copied().map(|value| value as u8)
    }

    /// Returns the GPIO non-secure mask pair.
    pub fn gpio_nonsecure_masks(&self) -> (u32, u32) {
        (self.registers[GPIO_NSMASK0], self.registers[GPIO_NSMASK1])
    }

    fn reset_configuration(&mut self) {
        for index in 3..REGISTER_COUNT {
            self.registers[index] = self.reset[index];
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
        Ok(u64::from(self.registers[index]))
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
        let current = self.registers[index];
        if matches!(register, Rp2350AccessCtrlRegister::CfgReset) {
            if value & 1 != 0 {
                self.reset_configuration();
            }
            return Ok(());
        }
        if matches!(register, Rp2350AccessCtrlRegister::Lock) {
            // DMA is permanently locked by hardware; the other lock bits are
            // write-once until a full ACCESSCTRL reset.
            self.registers[index] = (current | value | DMA_LOCK) & LOCK_MASK;
            return Ok(());
        }
        let updated = atomic_update(current, (offset >> 12) & 3, value)?;
        self.registers[index] = updated & Self::mask(register);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers = self.reset;
    }
}
