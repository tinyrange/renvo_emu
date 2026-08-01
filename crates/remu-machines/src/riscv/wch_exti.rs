use super::MachineError;
use crate::TargetId;
use remu_bus::AddressSpace;
use remu_core::{RunStats, SimTime};
use remu_cpu_riscv::RiscVCpu;
use remu_devices::{GpioHandle, WchExti, WchExtiHandle, WchPficHandle, WchTimerHandle};
use remu_signals::Logic;

const WCH_EXTI7_0_INTERRUPT: u16 = 20;
const WCH_TIMER_INTERRUPT: u16 = 38;

pub(super) struct WchHandles {
    pub(super) timer: WchTimerHandle,
    pub(super) pfic: WchPficHandle,
    pub(super) exti: WchExtiHandle,
}

/// Polls both WCH interrupt sources for one scheduler step.
pub(super) fn poll_wch(
    handles: &WchHandles,
    gpios: &[GpioHandle],
    cpu: &mut RiscVCpu,
    timer_was_pending: &mut bool,
    exti_was_pending: &mut bool,
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
    )?;
    poll_wch_exti(
        &handles.exti,
        &handles.pfic,
        gpios,
        cpu,
        exti_was_pending,
        stats,
    )
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
) -> Result<(), MachineError> {
    let pending = timer.pending(now);
    pfic.set_pending(WCH_TIMER_INTERRUPT, pending);
    let deliver = pfic.next_pending() == Some(WCH_TIMER_INTERRUPT);
    if deliver && !*was_pending {
        stats.events = stats.events.saturating_add(1);
    }
    *was_pending = deliver;
    cpu.set_qingke_external_interrupt(WCH_TIMER_INTERRUPT, deliver)?;
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
