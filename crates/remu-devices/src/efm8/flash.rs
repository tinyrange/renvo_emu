use super::*;

pub(super) const PSCTL: usize = 0x8f;
pub(super) const FLKEY: usize = 0xb7;
pub(super) const FLASH_BYTES: usize = 32 * 1024;
const FLASH_PAGE_BYTES: usize = 2048;
const PSCTL_OKVDDF: u8 = 0x40;
const PSCTL_PERRF: u8 = 0x08;
const PSCTL_PSEE: u8 = 0x02;
pub(super) const PSCTL_PSWE: u8 = 0x01;

impl Efm8State {
    pub(super) fn flash_read(&self, address: usize) -> Result<u8, DeviceError> {
        self.flash.get(address).copied().ok_or_else(|| {
            DeviceError::new(format!("EFM8 flash address outside 32 KiB: {address:#x}"))
        })
    }

    pub(super) fn flash_key_write(&mut self, value: u8) {
        if self.flash_locked_out {
            return;
        }
        match (self.flash_key_stage, value) {
            (0, 0xa5) => self.flash_key_stage = 1,
            (1, 0xf1) => {
                self.flash_key_stage = 2;
                self.flash_unlocked = true;
            }
            _ => {
                self.flash_key_stage = 0;
                self.flash_unlocked = false;
                self.flash_locked_out = true;
            }
        }
    }

    pub(super) fn flash_write(&mut self, address: usize, value: u8) -> Result<(), DeviceError> {
        if address >= FLASH_BYTES {
            return Err(DeviceError::new(format!(
                "EFM8 flash address outside 32 KiB: {address:#x}"
            )));
        }
        let controls = self.registers[PSCTL];
        if controls & PSCTL_PSWE == 0
            || controls & PSCTL_OKVDDF == 0
            || !self.flash_unlocked
            || self.flash_locked_out
        {
            self.registers[PSCTL] |= PSCTL_PERRF;
            return Ok(());
        }
        if controls & PSCTL_PSEE != 0 {
            let page_start = address / FLASH_PAGE_BYTES * FLASH_PAGE_BYTES;
            let page_end = (page_start + FLASH_PAGE_BYTES).min(FLASH_BYTES);
            self.flash[page_start..page_end].fill(0xff);
        } else {
            self.flash[address] &= value;
        }
        self.flash_key_stage = 0;
        self.flash_unlocked = false;
        Ok(())
    }

    pub(super) fn load_flash(&mut self, address: usize, bytes: &[u8]) -> Result<(), DeviceError> {
        let end = address
            .checked_add(bytes.len())
            .ok_or_else(|| DeviceError::new("EFM8 flash image range overflow"))?;
        let destination = self
            .flash
            .get_mut(address..end)
            .ok_or_else(|| DeviceError::new("EFM8 flash image exceeds 32 KiB"))?;
        destination.copy_from_slice(bytes);
        Ok(())
    }
}
