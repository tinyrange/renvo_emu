use remu_bus::{AddressSpace, MapError};
use remu_devices::EspLpI2c;

/// Maps the ESP32-C6 low-power I2C controller at its native address.
pub(super) fn map_esp32c6_lp_i2c(bus: &mut AddressSpace) -> Result<(), MapError> {
    let (device, _handle) = EspLpI2c::new("esp32c6.lp-i2c");
    bus.map_device("esp32c6.lp-i2c", 0x600b_1800, 0x400, Box::new(device))
}
