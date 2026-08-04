use super::*;
use remu_devices::{Esp32S3DmaPeripheral, Esp32S3World};

impl XtensaMachine {
    pub(super) fn service_peri_backup(&mut self) -> Result<bool, XtensaMachineError> {
        let Some(request) = self.peri_backup.take_request() else {
            return Ok(false);
        };
        for (packed_index, word_offset) in request.word_offsets.iter().copied().enumerate() {
            let peripheral = u64::from(request.apb_address) + u64::from(word_offset) * 4;
            let memory = u64::from(request.memory_address) + packed_index as u64 * 4;
            let (source, destination) = if request.to_memory {
                (peripheral, memory)
            } else {
                (memory, peripheral)
            };
            let memory_access = if request.to_memory {
                AccessKind::Write
            } else {
                AccessKind::Read
            };
            if !self.pms.check_dma_access(
                Esp32S3DmaPeripheral::Backup,
                Esp32S3World::Secure,
                u32::try_from(memory).unwrap_or(u32::MAX),
                AccessWidth::Word,
                memory_access,
            ) {
                self.peri_backup.complete(Some(4));
                return Ok(true);
            }
            let value = match self
                .bus
                .read(source, AccessWidth::Word, AccessKind::Read, self.now)
            {
                Ok(value) => value,
                Err(_) => {
                    self.peri_backup.complete(Some(3));
                    return Ok(true);
                }
            };
            if self
                .bus
                .write(destination, AccessWidth::Word, value, self.now)
                .is_err()
            {
                self.peri_backup.complete(Some(3));
                return Ok(true);
            }
        }
        self.peri_backup.complete(None);
        Ok(true)
    }
}
