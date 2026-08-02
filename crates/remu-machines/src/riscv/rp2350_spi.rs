use super::*;

pub(super) fn map_rp2350_spi(
    bus: &mut AddressSpace,
    handles: &mut Vec<Rp2350SpiHandle>,
) -> Result<(), MapError> {
    for (name, base) in [("rp2350.spi0", 0x4008_0000), ("rp2350.spi1", 0x4008_8000)] {
        let (device, handle) = Rp2350Spi::new(name);
        bus.map_device(name, base, 0x4000, Box::new(device))?;
        handles.push(handle);
    }
    Ok(())
}

pub(super) fn set_rp2350_spi_interrupts(
    cpu: &mut RiscVCpu,
    handles: &[Rp2350SpiHandle],
) -> Result<(), CpuFault> {
    for (index, spi) in handles.iter().enumerate() {
        let line = 31_u16 + u16::try_from(index).expect("RP2350 SPI index fits IRQ line");
        cpu.set_hazard3_external_interrupt(line, spi.interrupt_pending())?;
    }
    Ok(())
}
