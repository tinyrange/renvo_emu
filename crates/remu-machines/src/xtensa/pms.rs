use remu_bus::AddressSpace;
use remu_core::{AccessKind, AccessWidth, Bus, BusFault, SimTime};
use remu_devices::{Esp32S3PmsHandle, Esp32S3World};

/// CPU-facing address-space wrapper that applies PMS before the underlying bus.
pub(super) struct Esp32S3PmsBus<'a> {
    bus: &'a mut AddressSpace,
    pms: &'a Esp32S3PmsHandle,
    core: u8,
    world: Esp32S3World,
}

impl<'a> Esp32S3PmsBus<'a> {
    pub(super) const fn new(
        bus: &'a mut AddressSpace,
        pms: &'a Esp32S3PmsHandle,
        core: u8,
        world: Esp32S3World,
    ) -> Self {
        Self {
            bus,
            pms,
            core,
            world,
        }
    }
}

impl Bus for Esp32S3PmsBus<'_> {
    fn read(
        &mut self,
        address: u64,
        width: AccessWidth,
        kind: AccessKind,
        at: SimTime,
    ) -> Result<u64, BusFault> {
        let checked = u32::try_from(address).is_ok_and(|address| {
            self.pms
                .check_cpu_access(self.core, self.world, address, width, kind)
        });
        if checked {
            self.bus.read(address, width, kind, at)
        } else {
            // The PMS fabric responds to denied internal/PIF reads with zero.
            Ok(0)
        }
    }

    fn write(
        &mut self,
        address: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), BusFault> {
        let checked = u32::try_from(address).is_ok_and(|address| {
            self.pms
                .check_cpu_access(self.core, self.world, address, width, AccessKind::Write)
        });
        if checked {
            self.bus.write(address, width, value, at)
        } else {
            // Denied writes complete on the fabric without reaching the slave.
            Ok(())
        }
    }
}
