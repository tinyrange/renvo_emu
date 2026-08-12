use super::*;

impl RiscVMachine {
    pub(super) fn service_functional_bootrom(&mut self) -> Result<bool, String> {
        if self.target == TargetId::Esp32c6 {
            let pc = self.cpu.pc();
            if !Self::esp32c6_functional_service_address(pc) {
                return Ok(false);
            }
            let result = (|| {
                if self.service_esp32c6_bootrom_primary(pc)? {
                    return Ok(true);
                }
                self.service_esp32c6_bootrom_secondary(pc)
            })();
            return result
                .map_err(|error| format!("ESP32-C6 functional service at PC {pc:#010x}: {error}"));
        }
        self.service_rp2350_bootrom()
    }

    #[inline(always)]
    pub(super) const fn esp32c6_functional_service_address(pc: u32) -> bool {
        (pc >= 0x4000_0000 && pc < 0x4005_0000) || (pc >= 0x420f_0000 && pc < 0x4210_0000)
    }
}
