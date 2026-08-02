use super::*;

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
        let Some(touch) = &self.wch_touch_key else {
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
        let (Some(touch), Some(pfic)) = (&self.wch_touch_key, &self.wch_pfic) else {
            return Ok(());
        };
        let pending = touch.pending(self.now);
        pfic.set_pending(WCH_ADC_INTERRUPT, pending);
        let deliver = pfic.next_pending() == Some(WCH_ADC_INTERRUPT);
        if deliver && !*was_pending {
            stats.events = stats.events.saturating_add(1);
        }
        *was_pending = deliver;
        self.cpu
            .set_qingke_external_interrupt(WCH_ADC_INTERRUPT, deliver)?;
        Ok(())
    }
}
