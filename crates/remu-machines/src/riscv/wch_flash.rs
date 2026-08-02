use remu_bus::{AddressSpace, MapError, Permissions};
use remu_devices::WchFlashMemory;

use crate::TargetId;

/// Returns whether a target uses the WCH flash controller.
pub(crate) const fn is_target(target: TargetId) -> bool {
    matches!(target, TargetId::Ch32v003 | TargetId::Ch32v006)
}

/// Maps a WCH on-chip flash image, its native alias, and the controller.
pub(crate) fn map_wch_flash(
    bus: &mut AddressSpace,
    target: TargetId,
    start: u64,
    size: usize,
) -> Result<(), MapError> {
    let (flash, controller) = WchFlashMemory::new(format!("{target}.flash"), size, 1024);
    let alias = flash.alias(format!("{target}.flash-alias"));
    bus.map_device_with_permissions(
        format!("{target}.flash"),
        start,
        size,
        Permissions::RWX,
        Box::new(flash),
    )?;
    if start == 0 {
        bus.map_device_with_permissions(
            format!("{target}.flash-alias"),
            0x0800_0000,
            size,
            Permissions::RWX,
            Box::new(alias),
        )?;
    }
    bus.map_device(
        format!("{target}.flash-controller"),
        0x4002_2000,
        0x400,
        Box::new(controller),
    )?;
    Ok(())
}
