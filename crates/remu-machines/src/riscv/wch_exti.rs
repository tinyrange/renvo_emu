use super::MachineError;
use crate::TargetId;
use remu_bus::AddressSpace;
use remu_core::{RunStats, SimTime};
use remu_cpu_riscv::RiscVCpu;
use remu_devices::{
    GpioHandle, WchAdcHandle, WchDmaHandle, WchExti, WchExtiHandle, WchI2cHandle, WchPficHandle,
    WchPowerHandle, WchSpiHandle, WchTimerHandle, WchTouchKeyHandle, WchWatchdogHandle,
};
use remu_signals::Logic;

const WCH_EXTI7_0_INTERRUPT: u16 = 20;
const WCH_I2C1_EVENT_INTERRUPT: u16 = 30;
const WCH_I2C1_ERROR_INTERRUPT: u16 = 31;
const WCH_SPI1_INTERRUPT: u16 = 33;
const WCH_TIM1_UPDATE_INTERRUPT: u16 = 35;
const WCH_TIM2_INTERRUPT: u16 = 38;

pub(super) struct WchHandles {
    pub(super) timer: WchTimerHandle,
    pub(super) timer1: WchTimerHandle,
    pub(super) pfic: WchPficHandle,
    pub(super) exti: WchExtiHandle,
    pub(super) spi: WchSpiHandle,
    pub(super) watchdogs: [WchWatchdogHandle; 2],
    pub(super) adc: Option<WchAdcHandle>,
    pub(super) touch: Option<WchTouchKeyHandle>,
    pub(super) dma: WchDmaHandle,
    pub(super) i2c: WchI2cHandle,
    pub(super) power: WchPowerHandle,
}

impl super::RiscVMachine {
    /// Returns the native WCH SPI1 MOSI transcript, when this is a WCH target.
    pub fn wch_spi_tx_bytes(&self) -> Vec<u8> {
        self.wch
            .as_ref()
            .map_or_else(Vec::new, |wch| wch.spi.tx_bytes())
    }

    /// Supplies the MISO byte returned by the next native WCH SPI1 transfer.
    pub fn inject_wch_spi_rx(&self, value: u8) {
        if let Some(wch) = &self.wch {
            wch.spi.inject_rx(value);
        }
    }

    /// Returns the host-facing WCH I2C1 handle for CH32V003/006 targets.
    pub fn wch_i2c(&self) -> Option<WchI2cHandle> {
        self.wch.as_ref().map(|wch| wch.i2c.clone())
    }

    /// Returns the host-facing WCH power-control handle for CH32V003/006.
    pub fn wch_power(&self) -> Option<WchPowerHandle> {
        self.wch.as_ref().map(|wch| wch.power.clone())
    }
}

/// Polls both WCH interrupt sources for one scheduler step.
pub(super) fn poll_wch(
    handles: &WchHandles,
    gpios: &[GpioHandle],
    cpu: &mut RiscVCpu,
    timer_was_pending: &mut bool,
    timer1_was_pending: &mut bool,
    exti_was_pending: &mut bool,
    spi_was_pending: &mut bool,
    i2c_was_pending: &mut [bool; 2],
    stats: &mut RunStats,
    now: SimTime,
) -> Result<(), MachineError> {
    poll_wch_timer(
        &handles.timer,
        &handles.pfic,
        cpu,
        timer_was_pending,
        stats,
        now,
        WCH_TIM2_INTERRUPT,
    )?;
    poll_wch_timer(
        &handles.timer1,
        &handles.pfic,
        cpu,
        timer1_was_pending,
        stats,
        now,
        WCH_TIM1_UPDATE_INTERRUPT,
    )?;
    poll_wch_exti(
        &handles.exti,
        &handles.pfic,
        gpios,
        cpu,
        exti_was_pending,
        stats,
    )?;
    poll_wch_spi(&handles.spi, &handles.pfic, cpu, spi_was_pending, stats)?;
    poll_wch_i2c(&handles.i2c, &handles.pfic, cpu, i2c_was_pending, stats)
}

/// Routes I2C1 event and error conditions through their native PFIC lines.
fn poll_wch_i2c(
    i2c: &WchI2cHandle,
    pfic: &WchPficHandle,
    cpu: &mut RiscVCpu,
    was_pending: &mut [bool; 2],
    stats: &mut RunStats,
) -> Result<(), MachineError> {
    let (event, error) = i2c.interrupt_pending();
    for (index, (interrupt, pending)) in [
        (WCH_I2C1_EVENT_INTERRUPT, event),
        (WCH_I2C1_ERROR_INTERRUPT, error),
    ]
    .into_iter()
    .enumerate()
    {
        pfic.set_pending(interrupt, pending);
        let deliver = pfic.next_pending() == Some(interrupt);
        if deliver && !was_pending[index] {
            stats.events = stats.events.saturating_add(1);
        }
        was_pending[index] = deliver;
        cpu.set_qingke_external_interrupt(interrupt, deliver)?;
    }
    Ok(())
}

/// Forwards the SPI1 status request through its native PFIC line.
pub(super) fn poll_wch_spi(
    spi: &WchSpiHandle,
    pfic: &WchPficHandle,
    cpu: &mut RiscVCpu,
    was_pending: &mut bool,
    stats: &mut RunStats,
) -> Result<(), MachineError> {
    let pending = spi.pending();
    pfic.set_pending(WCH_SPI1_INTERRUPT, pending);
    let deliver = pfic.next_pending() == Some(WCH_SPI1_INTERRUPT);
    if deliver && !*was_pending {
        stats.events = stats.events.saturating_add(1);
    }
    *was_pending = deliver;
    cpu.set_qingke_external_interrupt(WCH_SPI1_INTERRUPT, deliver)?;
    Ok(())
}

/// Maps the coupled AFIO and EXTI blocks for a WCH V00x target.
pub(super) fn map_wch_exti(
    bus: &mut AddressSpace,
    target: TargetId,
) -> Result<WchExtiHandle, MachineError> {
    let (exti, handle, afio) = WchExti::new(format!("{target}.exti"), format!("{target}.afio"));
    bus.map_device(format!("{target}.afio"), 0x4001_0000, 0x400, Box::new(afio))?;
    bus.map_device(format!("{target}.exti"), 0x4001_0400, 0x400, Box::new(exti))?;
    Ok(handle)
}

/// Polls TIM2 and forwards its level-sensitive interrupt through the PFIC.
pub(super) fn poll_wch_timer(
    timer: &WchTimerHandle,
    pfic: &WchPficHandle,
    cpu: &mut RiscVCpu,
    was_pending: &mut bool,
    stats: &mut RunStats,
    now: SimTime,
    interrupt: u16,
) -> Result<(), MachineError> {
    let pending = timer.pending(now);
    pfic.set_pending(interrupt, pending);
    let deliver = pfic.next_pending() == Some(interrupt);
    if deliver && !*was_pending {
        stats.events = stats.events.saturating_add(1);
    }
    *was_pending = deliver;
    cpu.set_qingke_external_interrupt(interrupt, deliver)?;
    Ok(())
}

/// Samples WCH GPIO ports and forwards EXTI7_0 to the QingKe PFIC.
pub(super) fn poll_wch_exti(
    exti: &WchExtiHandle,
    pfic: &WchPficHandle,
    gpios: &[GpioHandle],
    cpu: &mut RiscVCpu,
    was_pending: &mut bool,
    stats: &mut RunStats,
) -> Result<(), MachineError> {
    let mut inputs = [0_u32; 3];
    for (port, gpio) in gpios.iter().take(3).enumerate() {
        for pin in 0..gpio.pin_count().min(8) {
            let pin = u8::try_from(pin).expect("WCH EXTI pin fits u8");
            if gpio.resolved(pin)? == Logic::One {
                inputs[port] |= 1 << pin;
            }
        }
    }
    let pending = exti.pending(inputs);
    pfic.set_pending(WCH_EXTI7_0_INTERRUPT, pending);
    let deliver = pfic.next_pending() == Some(WCH_EXTI7_0_INTERRUPT);
    if deliver && !*was_pending {
        stats.events = stats.events.saturating_add(1);
    }
    *was_pending = deliver;
    cpu.set_qingke_external_interrupt(WCH_EXTI7_0_INTERRUPT, deliver)?;
    Ok(())
}
