use super::*;

impl RiscVMachine {
    /// Returns the current A/B output state for one RP2350 PWM slice.
    pub fn pwm_outputs(&self, slice: usize) -> Option<[bool; 2]> {
        self.chip_pwm.as_ref().and_then(|pwm| pwm.outputs(slice))
    }

    /// Returns pending bits for the selected RP2350 PWM interrupt bank.
    pub fn pwm_pending_interrupts(&self, irq: usize) -> u32 {
        self.chip_pwm
            .as_ref()
            .map_or(0, |pwm| pwm.pending_interrupts_for(irq))
    }
}
