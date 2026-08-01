use remu_core::{CpuFault, RunStats, SimTime};
use remu_cpu_riscv::RiscVCpu;
use remu_devices::{WchPficHandle, WchSpiHandle, WchTimerHandle};

impl super::RiscVMachine {
    /// Returns the native WCH SPI1 MOSI transcript, when this is a WCH target.
    pub fn wch_spi_tx_bytes(&self) -> Vec<u8> {
        self.wch_spi
            .as_ref()
            .map_or_else(Vec::new, WchSpiHandle::tx_bytes)
    }

    /// Supplies the MISO byte returned by the next native WCH SPI1 transfer.
    pub fn inject_wch_spi_rx(&self, value: u8) {
        if let Some(spi) = &self.wch_spi {
            spi.inject_rx(value);
        }
    }
}

/// Polls the WCH timer and SPI interrupt sources for one functional tick.
pub(crate) fn poll(
    cpu: &mut RiscVCpu,
    timer: Option<&WchTimerHandle>,
    spi: Option<&WchSpiHandle>,
    pfic: Option<&WchPficHandle>,
    now: SimTime,
    timer_was_pending: &mut bool,
    spi_was_pending: &mut bool,
    stats: &mut RunStats,
) -> Result<(), CpuFault> {
    let Some(pfic) = pfic else { return Ok(()) };
    for (source, pending, previous) in [
        (
            38_u16,
            timer.is_some_and(|timer| timer.pending(now)),
            timer_was_pending,
        ),
        (
            35_u16,
            spi.is_some_and(WchSpiHandle::pending),
            spi_was_pending,
        ),
    ] {
        pfic.set_pending(source, pending);
        let deliver = pfic.next_pending() == Some(source);
        if deliver && !*previous {
            stats.events = stats.events.saturating_add(1);
        }
        *previous = deliver;
        cpu.set_qingke_external_interrupt(source, deliver)?;
    }
    Ok(())
}
