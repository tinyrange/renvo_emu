use remu_bus::{AddressSpace, MapError};
use remu_devices::{FunctionalUart, UartHandle};

pub(super) fn map_esp32c6_uarts(
    bus: &mut AddressSpace,
    chip_uarts: &mut Vec<UartHandle>,
) -> Result<(), MapError> {
    let (uart0, handle) = FunctionalUart::new_lenient("esp32c6.uart0", 0x00, 0x1c, 0);
    bus.map_device("esp32c6.uart0", 0x6000_0000, 0x1000, Box::new(uart0))?;
    chip_uarts.push(handle);

    let (uart1, handle) = FunctionalUart::new_lenient("esp32c6.uart1", 0x00, 0x1c, 0xe000_c000);
    bus.map_device("esp32c6.uart1", 0x6000_1000, 0x1000, Box::new(uart1))?;
    chip_uarts.push(handle);

    let (lp_uart, handle) = FunctionalUart::new_lenient("esp32c6.lp-uart", 0x00, 0x1c, 0xe000_c000);
    bus.map_device("esp32c6.lp-uart", 0x600b_1400, 0x400, Box::new(lp_uart))?;
    chip_uarts.push(handle);
    Ok(())
}
