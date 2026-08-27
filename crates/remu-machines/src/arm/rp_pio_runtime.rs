use super::*;

impl ArmMachine {
    pub(super) fn refresh_pio_dma_requests(&self) -> Result<(), ArmMachineError> {
        for pin in 0..self.chip_gpio.pin_count() {
            let pin = pin as u8;
            let pull = self
                .pads
                .as_ref()
                .and_then(|pads| pads.pull(pin))
                .unwrap_or(Logic::Z);
            self.chip_gpio.drive_weak_source(pin, 1, pull, self.now)?;
        }
        for (block, pio) in self.pio.iter().enumerate() {
            let (_, _, gpio_base) = pio.pad_state();
            let mut inputs = 0_u32;
            for logical_pin in 0..32 {
                let physical_pin = usize::from(gpio_base) + logical_pin;
                if physical_pin >= self.chip_gpio.pin_count() {
                    continue;
                }
                let pin = physical_pin as u8;
                let control = self
                    .rp2040_io_bank
                    .as_ref()
                    .and_then(|bank| bank.pin_control(pin))
                    .or_else(|| {
                        self.rp2350_io_bank
                            .as_ref()
                            .and_then(|bank| bank.pin_control(pin))
                    })
                    .unwrap_or(0x1f);
                let input_enabled = self
                    .pads
                    .as_ref()
                    .is_none_or(|pads| pads.input_enabled(pin));
                let mut high = input_enabled && self.chip_gpio.resolved(pin)? == Logic::One;
                high = match control >> 16 & 3 {
                    0 => high,
                    1 => !high,
                    2 => false,
                    _ => true,
                };
                inputs |= u32::from(high) << logical_pin;
            }
            pio.set_inputs(inputs);
            for machine in 0..4 {
                let base = block * 8 + machine;
                self.dma.set_dreq(base as u8, pio.tx_dreq(machine));
                self.dma.set_dreq((base + 4) as u8, pio.rx_dreq(machine));
            }
        }

        for pin in 0..self.chip_gpio.pin_count() {
            let pin = pin as u8;
            let control = self
                .rp2040_io_bank
                .as_ref()
                .and_then(|bank| bank.pin_control(pin))
                .or_else(|| {
                    self.rp2350_io_bank
                        .as_ref()
                        .and_then(|bank| bank.pin_control(pin))
                })
                .unwrap_or(0x1f);
            let selected = match control & 0x1f {
                function @ 6..=8 => usize::try_from(function - 6)
                    .ok()
                    .filter(|block| *block < self.pio.len()),
                _ => None,
            };
            let bit = if pin < 32 {
                1_u32 << pin
            } else {
                1_u32 << (pin - 32)
            };
            let (sio_direction, sio_output) = if pin < 32 {
                (self.chip_gpio.direction(), self.chip_gpio.output())
            } else {
                (
                    self.chip_gpio.direction_high(),
                    self.chip_gpio.output_high(),
                )
            };
            let output_disabled = self
                .pads
                .as_ref()
                .is_some_and(|pads| pads.output_disabled(pin));
            let sio = if output_disabled || selected.is_some() || sio_direction & bit == 0 {
                Logic::Z
            } else if sio_output & bit == 0 {
                Logic::Zero
            } else {
                Logic::One
            };
            self.chip_gpio.drive_source(pin, 0, sio, self.now)?;

            for (block, pio) in self.pio.iter().enumerate() {
                let (output, direction, gpio_base) = pio.pad_state();
                let logical = usize::from(pin).checked_sub(usize::from(gpio_base));
                let source_selected =
                    selected == Some(block) && logical.is_some_and(|logical| logical < 32);
                let mut output_enabled =
                    source_selected && direction & (1 << logical.unwrap_or(0)) != 0;
                let mut high = logical
                    .filter(|logical| *logical < 32)
                    .is_some_and(|logical| output & (1 << logical) != 0);
                high = match control >> 8 & 3 {
                    0 => high,
                    1 => !high,
                    2 => false,
                    _ => true,
                };
                output_enabled = match control >> 12 & 3 {
                    0 => output_enabled,
                    1 => !output_enabled,
                    2 => false,
                    _ => true,
                };
                output_enabled &= !output_disabled;
                let logic = if !source_selected || !output_enabled {
                    Logic::Z
                } else if high {
                    Logic::One
                } else {
                    Logic::Zero
                };
                self.chip_gpio
                    .drive_source(pin, 16 + block as u16, logic, self.now)?;
            }
        }
        Ok(())
    }
}
