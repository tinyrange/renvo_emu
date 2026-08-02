use remu_bus::AddressSpace;
use remu_devices::{UartHandle, WchUsart};

use crate::{MachineError, TargetId};

/// Maps the CH32V006-only USART2 functional slice.
pub(super) fn map(bus: &mut AddressSpace, target: TargetId) -> Result<UartHandle, MachineError> {
    let (device, handle) = WchUsart::new(format!("{target}.usart2"));
    bus.map_device(
        format!("{target}.usart2"),
        0x4000_4400,
        0x400,
        Box::new(device),
    )?;
    Ok(handle)
}
