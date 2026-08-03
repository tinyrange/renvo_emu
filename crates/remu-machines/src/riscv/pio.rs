use super::*;

pub(super) fn map_secondary_rp2350_pios(
    bus: &mut AddressSpace,
    pio: &mut Vec<RpPioHandle>,
    signals: &SignalHub,
    pins: u16,
) -> Result<(), MachineError> {
    for index in 1..3 {
        let name = format!("rp2350.pio{index}");
        let (device, handle) = RpPio::new_with_version(
            &name,
            pins,
            &format!("board.rp2350.pio{index}.gpio"),
            signals.clone(),
            RpPioVersion::Rp2350,
        )?;
        let base = 0x5020_0000 + index as u64 * 0x0010_0000;
        bus.map_device(name, base, 0x4000, Box::new(device))?;
        pio.push(handle);
    }
    Ok(())
}
