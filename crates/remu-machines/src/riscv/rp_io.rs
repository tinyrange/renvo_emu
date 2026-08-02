use super::MachineError;
use remu_bus::AddressSpace;
use remu_core::RunStats;
use remu_devices::{GpioHandle, RpIoBank, RpIoBankHandle};

/// Maps the functional RP2350 IO_BANK0 block alongside SIO.
pub(crate) fn map(
    bus: &mut AddressSpace,
    gpio: GpioHandle,
) -> Result<RpIoBankHandle, MachineError> {
    let (device, handle) = RpIoBank::new("rp2350.io-bank0", gpio, 48);
    bus.map_device("rp2350.io-bank0", 0x4002_8000, 0x4000, Box::new(device))?;
    Ok(handle)
}

/// Polls GPIO events and routes the bank's PROC0 interrupt to Hazard3 IRQ 21.
pub(crate) fn poll(
    machine: &mut super::RiscVMachine,
    stats: &mut RunStats,
    was_pending: &mut bool,
) -> Result<(), MachineError> {
    let Some(io_bank) = machine.io_bank.clone() else {
        return Ok(());
    };
    let pending = io_bank.poll(machine.now)?;
    if pending && !*was_pending {
        stats.events = stats.events.saturating_add(1);
    }
    *was_pending = pending;
    machine.cpu.set_hazard3_external_interrupt(21, pending)?;
    Ok(())
}
