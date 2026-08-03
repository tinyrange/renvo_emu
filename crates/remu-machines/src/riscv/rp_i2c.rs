use super::MachineError;
use remu_bus::AddressSpace;
use remu_devices::{RpI2c, RpI2cHandle, SignalHub};

pub(crate) fn map_rp2350_i2c(
    bus: &mut AddressSpace,
    signals: &SignalHub,
    handles: &mut Vec<RpI2cHandle>,
) -> Result<(), MachineError> {
    for (index, &(name, base)) in [("rp2350.i2c0", 0x4009_0000), ("rp2350.i2c1", 0x4009_8000)]
        .iter()
        .enumerate()
    {
        let (device, handle) =
            RpI2c::new(name, &format!("board.rp2350.i2c{index}"), signals.clone())?;
        bus.map_device(name, base, 0x4000, Box::new(device))?;
        handles.push(handle);
    }
    Ok(())
}
