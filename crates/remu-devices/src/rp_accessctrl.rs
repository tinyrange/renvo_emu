use super::*;

const REGISTER_COUNT: usize = 0xe8 / 4 + 1;
const LOCK: usize = 0x00 / 4;
const FORCE_CORE_NS: usize = 0x04 / 4;
const CFGRESET: usize = 0x08 / 4;
const GPIO_NSMASK0: usize = 0x0c / 4;
const GPIO_NSMASK1: usize = 0x10 / 4;
const LOCK_MASK: u32 = 0x0f;
const DMA_LOCK: u32 = 1 << 2;

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

    fn mask(index: usize) -> u32 {
        match index {
            LOCK => LOCK_MASK,
            FORCE_CORE_NS | CFGRESET => 0x03,
            GPIO_NSMASK0 => u32::MAX,
            GPIO_NSMASK1 => 0xff00_ffff,
            _ => 0xff,
        }
    }

    /// Returns one peripheral's eight-bit security/privilege mask by offset.
    pub fn permission(&self, offset: u64) -> Option<u8> {
        let index = usize::try_from(offset / 4).ok()?;
        (index >= 5 && index < REGISTER_COUNT).then(|| self.registers[index] as u8)
    }

    /// Returns the GPIO non-secure mask pair.
    pub fn gpio_nonsecure_masks(&self) -> (u32, u32) {
        (self.registers[GPIO_NSMASK0], self.registers[GPIO_NSMASK1])
    }

    fn reset_configuration(&mut self) {
        for index in 1..REGISTER_COUNT {
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
        let index = usize::try_from(register / 4).expect("ACCESSCTRL index fits");
        self.registers
            .get(index)
            .copied()
            .map(u64::from)
            .ok_or_else(|| {
                DeviceError::new(format!(
                    "unmodeled RP2350 ACCESSCTRL read at offset {register:#x}"
                ))
            })
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
        let index = usize::try_from(register / 4).expect("ACCESSCTRL index fits");
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits");
        let current = *self.registers.get(index).ok_or_else(|| {
            DeviceError::new(format!(
                "unmodeled RP2350 ACCESSCTRL write at offset {register:#x}"
            ))
        })?;
        if index == CFGRESET {
            if value & 1 != 0 {
                self.reset_configuration();
            }
            return Ok(());
        }
        if index == LOCK {
            // DMA is permanently locked by hardware; the other lock bits are
            // write-once until a full ACCESSCTRL reset.
            self.registers[index] = (current | value | DMA_LOCK) & LOCK_MASK;
            return Ok(());
        }
        let updated = atomic_update(current, (offset >> 12) & 3, value)?;
        self.registers[index] = updated & Self::mask(index);
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.registers = self.reset;
    }
}
