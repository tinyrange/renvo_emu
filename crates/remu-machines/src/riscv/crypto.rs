use remu_bus::{AddressSpace, MapError};
use remu_devices::EspSha;

pub(super) fn map_esp32c6_sha(bus: &mut AddressSpace) -> Result<(), MapError> {
    bus.map_device(
        "esp32c6.sha",
        0x6008_9000,
        0x1000,
        Box::new(EspSha::new("esp32c6.sha")),
    )?;
    Ok(())
}
