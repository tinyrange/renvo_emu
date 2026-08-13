use remu_bus::AddressSpace;
use remu_core::{AccessKind, AccessWidth, Bus, BusFault, SimTime};
use remu_devices::{
    Esp32S3AssistDebugHandle, Esp32S3ExtmemHandle, Esp32S3PmsHandle, Esp32S3WorldControllerHandle,
};

/// CPU-facing address-space wrapper that applies PMS before the underlying bus.
pub(super) struct Esp32S3PmsBus<'a> {
    bus: &'a mut AddressSpace,
    pms: &'a Esp32S3PmsHandle,
    world_controller: &'a Esp32S3WorldControllerHandle,
    extmem: &'a Esp32S3ExtmemHandle,
    assist_debug: &'a Esp32S3AssistDebugHandle,
    core: u8,
    pc: u32,
    sp: u32,
    protection_active: bool,
}

impl<'a> Esp32S3PmsBus<'a> {
    #[cfg(test)]
    pub(super) const fn new(
        bus: &'a mut AddressSpace,
        pms: &'a Esp32S3PmsHandle,
        world_controller: &'a Esp32S3WorldControllerHandle,
        extmem: &'a Esp32S3ExtmemHandle,
        assist_debug: &'a Esp32S3AssistDebugHandle,
        core: u8,
        pc: u32,
        sp: u32,
    ) -> Self {
        Self::new_with_mode(
            bus,
            pms,
            world_controller,
            extmem,
            assist_debug,
            core,
            pc,
            sp,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) const fn new_with_mode(
        bus: &'a mut AddressSpace,
        pms: &'a Esp32S3PmsHandle,
        world_controller: &'a Esp32S3WorldControllerHandle,
        extmem: &'a Esp32S3ExtmemHandle,
        assist_debug: &'a Esp32S3AssistDebugHandle,
        core: u8,
        pc: u32,
        sp: u32,
        protection_active: bool,
    ) -> Self {
        Self {
            bus,
            pms,
            world_controller,
            extmem,
            assist_debug,
            core,
            pc,
            sp,
            protection_active,
        }
    }
}

impl Bus for Esp32S3PmsBus<'_> {
    fn fast_fetch32(&mut self, address: u64, at: SimTime) -> Option<Result<u32, BusFault>> {
        if self.protection_active {
            None
        } else {
            self.bus.fast_fetch32(address, at)
        }
    }

    fn fast_read(&mut self, address: u64, width: AccessWidth) -> Option<u64> {
        (!self.protection_active)
            .then(|| self.bus.fast_read(address, width))
            .flatten()
    }

    fn fast_write(&mut self, address: u64, width: AccessWidth, value: u64) -> bool {
        !self.protection_active && self.bus.fast_write(address, width, value)
    }

    fn read(
        &mut self,
        address: u64,
        width: AccessWidth,
        kind: AccessKind,
        at: SimTime,
    ) -> Result<u64, BusFault> {
        if !self.protection_active {
            return self.bus.read(address, width, kind, at);
        }
        let checked = u32::try_from(address).is_ok_and(|address| {
            if kind == AccessKind::Execute {
                self.world_controller.observe_execute(self.core, address);
            }
            let world = self.world_controller.world_for_access(self.core, kind);
            self.pms
                .check_cpu_access(self.core, world, address, width, kind)
        });
        let cache_allowed = checked
            && u32::try_from(address)
                .is_ok_and(|address| self.extmem.observe_access(self.core, address, kind));
        if cache_allowed {
            let value = self.bus.read(address, width, kind, at)?;
            if let Ok(address) = u32::try_from(address) {
                self.assist_debug.observe_access(
                    self.core,
                    address,
                    width,
                    kind,
                    self.pc,
                    self.sp,
                    u32::try_from(value).unwrap_or_default(),
                );
            }
            Ok(value)
        } else {
            if std::env::var_os("REMU_DEBUG_PMS").is_some() {
                eprintln!(
                    "S3 PMS denied {kind:?} {width:?} read at {address:#010x}, PC={:#010x}",
                    self.pc
                );
            }
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
        if !self.protection_active {
            return self.bus.write(address, width, value, at);
        }
        let checked = u32::try_from(address).is_ok_and(|address| {
            let world = self
                .world_controller
                .world_for_access(self.core, AccessKind::Write);
            self.pms
                .check_cpu_access(self.core, world, address, width, AccessKind::Write)
        });
        let cache_allowed = checked
            && u32::try_from(address).is_ok_and(|address| {
                self.extmem
                    .observe_access(self.core, address, AccessKind::Write)
            });
        if cache_allowed {
            self.bus.write(address, width, value, at)?;
            if let Ok(address) = u32::try_from(address) {
                self.world_controller
                    .observe_write(self.core, address, width, value);
                self.assist_debug.observe_access(
                    self.core,
                    address,
                    width,
                    AccessKind::Write,
                    self.pc,
                    self.sp,
                    u32::try_from(value).unwrap_or_default(),
                );
            }
            Ok(())
        } else {
            if std::env::var_os("REMU_DEBUG_PMS").is_some() {
                eprintln!(
                    "S3 PMS denied {width:?} write at {address:#010x}, value={value:#x}, PC={:#010x}",
                    self.pc
                );
            }
            // Denied writes complete on the fabric without reaching the slave.
            Ok(())
        }
    }
}
