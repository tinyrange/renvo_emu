use remu_core::{CpuFault, RunStats, SimTime};
use remu_cpu_riscv::RiscVCpu;
use remu_devices::{WchPficHandle, WchTimerHandle};

/// Polls the WCH TIM2 and TIM1 update sources and exposes their PFIC lines.
pub(crate) fn poll(
    timers: [Option<&WchTimerHandle>; 2],
    pfic: &WchPficHandle,
    now: SimTime,
    was_pending: &mut [bool; 2],
    stats: &mut RunStats,
    cpu: &mut RiscVCpu,
) -> Result<(), CpuFault> {
    let lines = [38_u16, 35_u16];
    let pending = timers.map(|timer| timer.is_some_and(|timer| timer.pending(now)));
    for (line, pending) in lines.into_iter().zip(pending) {
        pfic.set_pending(line, pending);
    }
    let next = pfic.next_pending();
    for (index, line) in lines.into_iter().enumerate() {
        let deliver = next == Some(line);
        if deliver && !was_pending[index] {
            stats.events = stats.events.saturating_add(1);
        }
        was_pending[index] = deliver;
        cpu.set_qingke_external_interrupt(line, deliver)?;
    }
    Ok(())
}
