use super::*;

pub(super) fn map_esp32c6_gdma(
    bus: &mut AddressSpace,
    signals: SignalHub,
) -> Result<(), MachineError> {
    let (gdma, _) = EspGdma::new("esp32c6.gdma", "board.esp32c6.gdma", signals)?;
    bus.map_device("esp32c6.gdma", 0x6008_0000, 0x2b0, Box::new(gdma))?;
    Ok(())
}
