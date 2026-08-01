use remu_bus::{AddressSpace, MapError};
use remu_devices::new_esp_sdio_slave;

pub(super) fn map_esp32c6_sdio(bus: &mut AddressSpace) -> Result<(), MapError> {
    let (hinf, slc, _handle) = new_esp_sdio_slave("esp32c6.sdio");
    bus.map_device("esp32c6.hinf", 0x6001_6000, 0x1000, Box::new(hinf))?;
    bus.map_device("esp32c6.slc", 0x6001_7000, 0x1000, Box::new(slc))?;
    Ok(())
}
