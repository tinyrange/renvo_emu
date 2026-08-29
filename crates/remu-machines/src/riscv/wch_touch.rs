use super::{MachineError, RiscVMachine};
use crate::TargetId;
use remu_bus::AddressSpace;
use remu_core::RunStats;
use remu_devices::{WchTouchKey, WchTouchKeyHandle};

const WCH_ADC_INTERRUPT: u16 = 15;

pub(super) fn map(
    bus: &mut AddressSpace,
    target: TargetId,
) -> Result<WchTouchKeyHandle, MachineError> {
    let (touch, handle) = WchTouchKey::new(format!("{target}.adc-tkey"));
    bus.map_device(
        format!("{target}.adc-tkey"),
        0x4001_2400,
        0x400,
        Box::new(touch),
    )?;
    Ok(handle)
}

impl RiscVMachine {
    /// Sets the deterministic converted value returned by a CH32V006 channel.
    pub fn set_touch_key(&self, channel: u8, value: u16) -> Result<(), MachineError> {
        let Some(touch) = self.wch.as_ref().and_then(|wch| wch.touch.as_ref()) else {
            return Err(MachineError::UnsupportedTarget(self.target));
        };
        touch.set_channel_value(channel, value);
        Ok(())
    }

    pub(super) fn poll_wch_touch_key(
        &mut self,
        stats: &mut RunStats,
        was_pending: &mut bool,
    ) -> Result<(), MachineError> {
        let Some(wch) = &self.wch else {
            return Ok(());
        };
        let Some(touch) = &wch.touch else {
            return Ok(());
        };
        let pending = touch.pending(self.now);
        wch.pfic.set_pending(WCH_ADC_INTERRUPT, pending);
        let deliver = wch.pfic.next_pending() == Some(WCH_ADC_INTERRUPT);
        if deliver && !*was_pending {
            stats.events = stats.events.saturating_add(1);
        }
        *was_pending = deliver;
        self.cpu
            .set_qingke_external_interrupt(WCH_ADC_INTERRUPT, deliver)?;
        Ok(())
    }
}
