impl RiscVMachine {
    fn complete_native_wifi_transmissions(
        &mut self,
        wifi_mac: &remu_devices::EspC6WifiMacHandle,
    ) -> Result<u64, MachineError> {
        let due = self
            .radio_pending_native_wifi
            .iter()
            .copied()
            .filter(|pending| pending.deadline <= self.now)
            .collect::<Vec<_>>();
        for pending in &due {
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::Wifi,
                    RadioLegalityRule::CompletionWithoutOperation,
                    wifi_mac.tx_active(pending.queue),
                    self.now,
                    format!(
                        "native TX queue {} reached its completion deadline without an active hardware operation",
                        pending.queue
                    ),
                )?;
            let outcome = if pending.ack_receiver.is_some() {
                remu_devices::EspWifiTxOutcome::AckTimeout
            } else {
                remu_devices::EspWifiTxOutcome::Success
            };
            self.radio_legality
                .as_mut()
                .expect("ESP32-C6 machine has a radio legality validator")
                .require(
                    RadioSubsystem::Wifi,
                    RadioLegalityRule::CompletionWithoutOperation,
                    wifi_mac.complete_tx(pending.queue, outcome),
                    self.now,
                    format!("native TX queue {} rejected its completion", pending.queue),
                )?;
        }
        self.radio_pending_native_wifi
            .retain(|pending| pending.deadline > self.now);
        Ok(due.len() as u64)
    }
}
