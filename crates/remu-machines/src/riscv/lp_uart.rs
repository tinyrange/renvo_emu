use super::*;

impl RiscVMachine {
    /// Queues bytes for reads from the ESP32-C6 low-power UART FIFO.
    pub fn queue_esp32c6_lp_uart_rx(&self, bytes: &[u8]) -> Result<(), MachineError> {
        let Some(handle) = &self.esp_lp_uart else {
            return Err(MachineError::UnsupportedTarget(self.target));
        };
        handle.queue_rx(bytes);
        Ok(())
    }
}
