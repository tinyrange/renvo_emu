//! ESP32-S3 RTC-domain GPIO and pad controller.

use super::*;

const PINS: usize = 22;
const DATA_MASK: u32 = 0x003f_ffff;
const DATA_SHIFT: u32 = 10;
const PIN_MASK: u32 = 0x0000_0784;
const PAD_MUX_SEL: u32 = 1 << 19;
const DATE_RESET: u32 = 0x0210_1180;
const PAD_MASKS: [u32; PINS] = [
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x787f_e000,
    0x780f_e000,
    0x780f_e000,
    0x780f_fff8,
    0x780f_fff8,
    0x780f_e000,
    0x780f_e000,
    0x780f_e000,
];
const PAD_RESETS: [u32; PINS] = [
    0x5000_0000,
    0x4800_0000,
    0x5000_0000,
    0x4800_0000,
    0x5000_0000,
    0x5000_0000,
    0x4800_0000,
    0x4000_0000,
    0x4000_0000,
    0x4000_0000,
    0x4000_0000,
    0x4000_0000,
    0x4000_0000,
    0x4000_0000,
    0x4000_0000,
    0x4000_0000,
    0x4000_0000,
    0x4000_0000,
    0x4000_0000,
    0x5000_0000,
    0x5000_0000,
    0x5000_0000,
];

#[derive(Clone)]
struct RtcIoState {
    output: u32,
    enable: u32,
    status: u32,
    previous_input: u32,
    pin_config: [u32; PINS],
    pad_config: [u32; PINS],
    debug_select: u32,
    ext_wakeup: u32,
    xtal_control: u32,
    sar_i2c_io: u32,
    touch_control: u32,
    date: u32,
}

impl RtcIoState {
    fn reset() -> Self {
        Self {
            output: 0,
            enable: 0,
            status: 0,
            previous_input: 0,
            pin_config: [0; PINS],
            pad_config: PAD_RESETS,
            debug_select: 0,
            ext_wakeup: 0,
            xtal_control: 0,
            sar_i2c_io: 0,
            touch_control: 0,
            date: DATE_RESET,
        }
    }
}

/// Host-side RTC GPIO status and wakeup view.
#[derive(Clone)]
pub struct Esp32S3RtcIoHandle {
    state: Rc<RefCell<RtcIoState>>,
    gpio: GpioHandle,
}

impl Esp32S3RtcIoHandle {
    fn input_mask(&self) -> u32 {
        (0..PINS).fold(0, |mask, pin| {
            if self.gpio.resolved(pin as u8).ok() == Some(Logic::One) {
                mask | (1 << pin)
            } else {
                mask
            }
        })
    }

    fn refresh_inputs(&self) -> u32 {
        let current = self.input_mask();
        let mut state = self.state.borrow_mut();
        let previous = state.previous_input;
        for pin in 0..PINS {
            let bit = 1_u32 << pin;
            let before = previous & bit != 0;
            let after = current & bit != 0;
            let interrupt_type = (state.pin_config[pin] >> 7) & 7;
            let triggered = match interrupt_type {
                1 => !before && after,
                2 => before && !after,
                3 => before != after,
                4 => !after,
                5 => after,
                _ => false,
            };
            if triggered {
                state.status |= bit;
            }
        }
        state.previous_input = current;
        current
    }

    /// Returns true when a configured RTC GPIO interrupt is latched.
    pub fn interrupt_pending(&self) -> bool {
        {
            let state = self.state.borrow();
            if state.status != 0 {
                return true;
            }
            if state
                .pin_config
                .iter()
                .all(|configuration| (configuration >> 7) & 7 == 0)
            {
                return false;
            }
        }
        self.refresh_inputs();
        self.state.borrow().status != 0
    }

    /// Returns the unshifted 22-bit RTC input sample.
    pub fn input_mask_unshifted(&self) -> u32 {
        self.refresh_inputs()
    }
}

/// Functional ESP32-S3 RTC_IO register page.
pub struct Esp32S3RtcIo {
    name: String,
    state: Rc<RefCell<RtcIoState>>,
    gpio: GpioHandle,
    rtc: EspRtcControlHandle,
    handle: Esp32S3RtcIoHandle,
}

impl Esp32S3RtcIo {
    /// Creates RTC GPIO0..21 coupled to the digital pad nets and RTC hold bits.
    pub fn new(
        name: impl Into<String>,
        gpio: GpioHandle,
        rtc: EspRtcControlHandle,
    ) -> (Self, Esp32S3RtcIoHandle) {
        let state = Rc::new(RefCell::new(RtcIoState::reset()));
        let handle = Esp32S3RtcIoHandle {
            state: state.clone(),
            gpio: gpio.clone(),
        };
        (
            Self {
                name: name.into(),
                state,
                gpio,
                rtc,
                handle: handle.clone(),
            },
            handle,
        )
    }

    fn check(&self, offset: u64, width: AccessWidth, operation: &str) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !offset.is_multiple_of(4) {
            return Err(DeviceError::new(
                "ESP32-S3 RTC_IO requires aligned word access",
            ));
        }
        if !(offset <= 0xe8 || offset == 0x1fc) {
            return Err(DeviceError::new(format!(
                "{} {operation} at reserved RTC_IO offset {offset:#x}",
                self.name
            )));
        }
        Ok(())
    }

    fn refresh_outputs(&self, at: SimTime) -> Result<(), DeviceError> {
        let state = self.state.borrow();
        for pin in 0..PINS {
            if self.rtc.pad_held(pin as u8) {
                continue;
            }
            let bit = 1_u32 << pin;
            let rtc_selected = state.pad_config[pin] & PAD_MUX_SEL != 0;
            let enabled = state.enable & bit != 0;
            let open_drain = state.pin_config[pin] & (1 << 2) != 0;
            let high = state.output & bit != 0;
            let value = if !rtc_selected || !enabled || (open_drain && high) {
                Logic::Z
            } else if high {
                Logic::One
            } else {
                Logic::Zero
            };
            self.gpio.drive_peripheral(pin as u8, 2, value, at)?;
        }
        Ok(())
    }

    fn read_value(&self, offset: u64) -> u32 {
        let input = self.handle.refresh_inputs();
        let state = self.state.borrow();
        match offset {
            0x00 => state.output << DATA_SHIFT,
            0x04 | 0x08 => 0,
            0x0c => state.enable << DATA_SHIFT,
            0x10 | 0x14 => 0,
            0x18 => state.status << DATA_SHIFT,
            0x1c | 0x20 => 0,
            0x24 => input << DATA_SHIFT,
            0x28..=0x7c => state.pin_config[(offset as usize - 0x28) / 4],
            0x80 => state.debug_select,
            0x84..=0xd8 => state.pad_config[(offset as usize - 0x84) / 4],
            0xdc => state.ext_wakeup,
            0xe0 => state.xtal_control,
            0xe4 => state.sar_i2c_io,
            0xe8 => state.touch_control,
            0x1fc => state.date,
            _ => unreachable!(),
        }
    }
}

impl Device for Esp32S3RtcIo {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        self.check(offset, width, "read")?;
        Ok(u64::from(self.read_value(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        self.check(offset, width, "write")?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 RTC_IO word exceeds 32 bits"))?;
        let data = value >> DATA_SHIFT & DATA_MASK;
        let mut state = self.state.borrow_mut();
        match offset {
            0x00 => state.output = data,
            0x04 => state.output |= data,
            0x08 => state.output &= !data,
            0x0c => state.enable = data,
            0x10 => state.enable |= data,
            0x14 => state.enable &= !data,
            0x18 => state.status = data,
            0x1c => state.status |= data,
            0x20 => state.status &= !data,
            0x24 => {}
            0x28..=0x7c => state.pin_config[(offset as usize - 0x28) / 4] = value & PIN_MASK,
            0x80 => state.debug_select = value & 0x03ff_ffff,
            0x84..=0xd8 => {
                let pin = (offset as usize - 0x84) / 4;
                state.pad_config[pin] = value & PAD_MASKS[pin];
            }
            0xdc => state.ext_wakeup = value & 0xf800_0000,
            0xe0 => state.xtal_control = value & 0xf800_0000,
            0xe4 => state.sar_i2c_io = value & 0xff80_0000,
            0xe8 => state.touch_control = value & 0x1f,
            0x1fc => state.date = value & 0x0fff_ffff,
            _ => unreachable!(),
        }
        drop(state);
        self.refresh_outputs(at)
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.borrow_mut() = RtcIoState::reset();
        for pin in 0..PINS {
            let _ = self
                .gpio
                .drive_peripheral(pin as u8, 2, Logic::Z, SimTime::ZERO);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Esp32S3RtcIo, Esp32S3RtcIoHandle, GpioHandle) {
        let hub = SignalHub::new();
        let (_gpio_device, gpio) = EspGpio::new("gpio", 49, "test.gpio", hub).unwrap();
        let (_rtc_device, rtc) = EspRtcControl::new_with_signals("rtc", SignalHub::new()).unwrap();
        let (rtc_io, handle) = Esp32S3RtcIo::new("rtc-io", gpio.clone(), rtc);
        (rtc_io, handle, gpio)
    }

    #[test]
    fn exact_register_masks_resets_and_reserved_holes() {
        let (mut rtc_io, _, _) = fixture();
        assert_eq!(
            rtc_io.read(0x84, AccessWidth::Word, SimTime::ZERO),
            Ok(0x5000_0000)
        );
        assert_eq!(
            rtc_io.read(0x88, AccessWidth::Word, SimTime::ZERO),
            Ok(0x4800_0000)
        );
        assert_eq!(
            rtc_io.read(0x1fc, AccessWidth::Word, SimTime::ZERO),
            Ok(DATE_RESET.into())
        );
        rtc_io
            .write(0x28, AccessWidth::Word, u32::MAX.into(), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rtc_io.read(0x28, AccessWidth::Word, SimTime::ZERO),
            Ok(PIN_MASK.into())
        );
        assert!(rtc_io.read(0xec, AccessWidth::Word, SimTime::ZERO).is_err());
    }

    #[test]
    fn rtc_mux_output_open_drain_input_and_interrupts_are_coupled() {
        let (mut rtc_io, handle, gpio) = fixture();
        rtc_io
            .write(
                0x84,
                AccessWidth::Word,
                (PAD_MUX_SEL | 0x4000_0000).into(),
                SimTime::ZERO,
            )
            .unwrap();
        rtc_io
            .write(
                0x0c,
                AccessWidth::Word,
                u64::from(1_u32 << DATA_SHIFT),
                SimTime::ZERO,
            )
            .unwrap();
        rtc_io
            .write(
                0x04,
                AccessWidth::Word,
                u64::from(1_u32 << DATA_SHIFT),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(gpio.resolved(0).unwrap(), Logic::One);
        rtc_io
            .write(
                0x28,
                AccessWidth::Word,
                u64::from((1_u32 << 10) | (2 << 7) | (1 << 2)),
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(gpio.resolved(0).unwrap(), Logic::Z);
        gpio.set_input(0, Logic::One, SimTime::from_ticks(1))
            .unwrap();
        handle.input_mask_unshifted();
        gpio.set_input(0, Logic::Zero, SimTime::from_ticks(2))
            .unwrap();
        assert!(handle.interrupt_pending());
        rtc_io
            .write(
                0x20,
                AccessWidth::Word,
                u64::from(1_u32 << DATA_SHIFT),
                SimTime::from_ticks(2),
            )
            .unwrap();
        assert!(!handle.interrupt_pending());
    }
}
