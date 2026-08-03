use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Register identifiers for the RP2040/RP2350 SAR ADC block.
///
/// The offsets are shared by both parts; the control-field masks differ because
/// RP2350 exposes a wider channel mux on its QFN-80 package.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpAdcRegister {
    /// ADC control and status.
    Cs,
    /// Most recent conversion result.
    Result,
    /// FIFO control and status.
    Fcs,
    /// Conversion-result FIFO.
    Fifo,
    /// Conversion pacing divider.
    Div,
    /// Raw FIFO-level interrupt.
    Intr,
    /// FIFO-level interrupt enable.
    Inte,
    /// FIFO-level interrupt force.
    Intf,
    /// Masked and forced FIFO-level interrupt status.
    Ints,
}

impl RpAdcRegister {
    /// Returns the native byte offset of this register.
    pub const fn offset(self) -> u64 {
        match self {
            Self::Cs => 0x00,
            Self::Result => 0x04,
            Self::Fcs => 0x08,
            Self::Fifo => 0x0c,
            Self::Div => 0x10,
            Self::Intr => 0x14,
            Self::Inte => 0x18,
            Self::Intf => 0x1c,
            Self::Ints => 0x20,
        }
    }
}

impl TryFrom<u64> for RpAdcRegister {
    type Error = ();

    fn try_from(offset: u64) -> Result<Self, Self::Error> {
        match offset {
            0x00 => Ok(Self::Cs),
            0x04 => Ok(Self::Result),
            0x08 => Ok(Self::Fcs),
            0x0c => Ok(Self::Fifo),
            0x10 => Ok(Self::Div),
            0x14 => Ok(Self::Intr),
            0x18 => Ok(Self::Inte),
            0x1c => Ok(Self::Intf),
            0x20 => Ok(Self::Ints),
            _ => Err(()),
        }
    }
}

/// Package-level ADC mux variants supported by the functional model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpAdcVariant {
    /// RP2040 and RP2350 QFN-60: four external inputs plus temperature.
    FiveChannel,
    /// RP2350 QFN-80: eight external inputs plus temperature.
    NineChannel,
}

impl RpAdcVariant {
    const fn channel_count(self) -> usize {
        match self {
            Self::FiveChannel => 5,
            Self::NineChannel => 9,
        }
    }

    const fn rrobin_mask(self) -> u32 {
        match self {
            Self::FiveChannel => 0x001f_0000,
            Self::NineChannel => 0x01ff_0000,
        }
    }

    const fn ainsel_mask(self) -> u32 {
        match self {
            Self::FiveChannel => 0x0000_7000,
            Self::NineChannel => 0x0000_f000,
        }
    }
}

#[derive(Clone)]
struct AdcState {
    control: u32,
    result: u16,
    samples: [u16; 9],
    fifo_control: u32,
    fifo: VecDeque<u16>,
    fifo_over: bool,
    fifo_under: bool,
    divider: u32,
    interrupt_enable: u32,
    interrupt_force: u32,
}

impl Default for AdcState {
    fn default() -> Self {
        Self {
            control: 0,
            result: 0,
            samples: [0; 9],
            fifo_control: 0,
            fifo: VecDeque::with_capacity(8),
            fifo_over: false,
            fifo_under: false,
            divider: 0,
            interrupt_enable: 0,
            interrupt_force: 0,
        }
    }
}

/// Host-facing deterministic RP ADC state.
#[derive(Clone)]
pub struct RpAdcHandle {
    state: Arc<Mutex<AdcState>>,
    channel_count: usize,
}

impl RpAdcHandle {
    /// Sets the 12-bit sample returned for one ADC input channel.
    pub fn set_sample(&self, channel: usize, value: u16) -> bool {
        if channel >= self.channel_count {
            return false;
        }
        let mut state = self.state.lock().expect("RP ADC lock poisoned");
        let Some(sample) = state.samples.get_mut(channel) else {
            return false;
        };
        *sample = value & 0x0fff;
        true
    }

    /// Returns the most recently converted sample.
    pub fn result(&self) -> u16 {
        self.state.lock().expect("RP ADC lock poisoned").result
    }

    /// Returns the number of samples waiting in the receive FIFO.
    pub fn fifo_level(&self) -> usize {
        self.state.lock().expect("RP ADC lock poisoned").fifo.len()
    }
}

/// Functional RP2040/RP2350 ADC and temperature-sensor register subset.
pub struct RpAdc {
    name: String,
    variant: RpAdcVariant,
    state: Arc<Mutex<AdcState>>,
}

impl RpAdc {
    const CS_EN: u32 = 1 << 0;
    const CS_TS_EN: u32 = 1 << 1;
    const CS_START_ONCE: u32 = 1 << 2;
    const CS_START_MANY: u32 = 1 << 3;
    const CS_READY: u32 = 1 << 8;
    const CS_ERR: u32 = 1 << 9;
    const CS_ERR_STICKY: u32 = 1 << 10;
    const FCS_THRESH_MASK: u32 = 0x0f00_0000;
    const FCS_OVER: u32 = 1 << 11;
    const FCS_UNDER: u32 = 1 << 10;
    const FCS_SHIFT: u32 = 1 << 1;
    const FCS_EN: u32 = 1 << 0;

    /// Creates the five-channel RP2040-compatible variant.
    pub fn new(name: impl Into<String>) -> (Self, RpAdcHandle) {
        Self::new_for_variant(name, RpAdcVariant::FiveChannel)
    }

    /// Creates an ADC with an explicit package mux variant.
    pub fn new_for_variant(name: impl Into<String>, variant: RpAdcVariant) -> (Self, RpAdcHandle) {
        let state = Arc::new(Mutex::new(AdcState::default()));
        let handle = RpAdcHandle {
            state: state.clone(),
            channel_count: variant.channel_count(),
        };
        (
            Self {
                name: name.into(),
                variant,
                state,
            },
            handle,
        )
    }

    fn register(offset: u64, width: AccessWidth) -> Result<RpAdcRegister, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RP ADC requires aligned word access"));
        }
        RpAdcRegister::try_from(offset).map_err(|_| {
            DeviceError::new(format!("unmodeled RP ADC register at offset {offset:#x}"))
        })
    }

    fn cs_writable_mask(&self) -> u32 {
        self.variant.rrobin_mask()
            | self.variant.ainsel_mask()
            | Self::CS_EN
            | Self::CS_TS_EN
            | Self::CS_START_ONCE
            | Self::CS_START_MANY
    }

    fn fifo_level(state: &AdcState) -> u32 {
        u32::try_from(state.fifo.len()).expect("RP ADC FIFO length fits")
    }

    fn fifo_interrupt(state: &AdcState) -> u32 {
        let threshold = (state.fifo_control >> 24) & 0x0f;
        u32::from(threshold != 0 && Self::fifo_level(state) >= threshold)
    }

    fn fifo_status(state: &AdcState) -> u32 {
        (state.fifo_control & (Self::FCS_THRESH_MASK | 0x0f))
            | (Self::fifo_level(state) << 16)
            | (u32::from(state.fifo_over) * Self::FCS_OVER)
            | (u32::from(state.fifo_under) * Self::FCS_UNDER)
            | (u32::from(state.fifo.len() == 8) << 9)
            | (u32::from(state.fifo.is_empty()) << 8)
    }

    fn next_round_robin_channel(&self, state: &AdcState, current: usize) -> Option<usize> {
        let mask = (state.control & self.variant.rrobin_mask()) >> 16;
        if mask == 0 {
            return None;
        }
        let count = self.variant.channel_count();
        (1..=count)
            .map(|step| (current + step) % count)
            .find(|channel| mask & (1 << channel) != 0)
    }

    fn convert(&self, state: &mut AdcState) {
        let channel = usize::try_from((state.control & self.variant.ainsel_mask()) >> 12)
            .expect("RP ADC channel fits");
        let valid = channel < self.variant.channel_count();
        let temperature_enabled =
            channel + 1 == self.variant.channel_count() && state.control & Self::CS_TS_EN != 0;
        state.control &= !Self::CS_ERR;
        if !valid || (channel + 1 == self.variant.channel_count() && !temperature_enabled) {
            state.control |= Self::CS_ERR | Self::CS_ERR_STICKY;
            state.result = 0;
        } else {
            state.result = state.samples[channel];
        }
        state.control |= Self::CS_READY;
        if state.fifo_control & Self::FCS_EN != 0 {
            if state.fifo.len() == 8 {
                state.fifo_over = true;
            } else {
                state.fifo.push_back(state.result);
            }
        }
        if let Some(next) = self.next_round_robin_channel(state, channel) {
            let ainsel = (u32::try_from(next).expect("RP ADC channel fits") << 12)
                & self.variant.ainsel_mask();
            state.control = (state.control & !self.variant.ainsel_mask()) | ainsel;
        }
    }
}

impl Device for RpAdc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let register = Self::register(offset, width)?;
        let mut state = self.state.lock().expect("RP ADC lock poisoned");
        let value = match register {
            RpAdcRegister::Cs => state.control,
            RpAdcRegister::Result => u32::from(state.result),
            RpAdcRegister::Fcs => Self::fifo_status(&state),
            RpAdcRegister::Fifo => {
                let Some(sample) = state.fifo.pop_front() else {
                    state.fifo_under = true;
                    return Ok(0);
                };
                if state.fifo_control & Self::FCS_SHIFT != 0 {
                    u32::from(sample >> 4)
                } else {
                    u32::from(sample)
                }
            }
            RpAdcRegister::Div => state.divider,
            RpAdcRegister::Intr => Self::fifo_interrupt(&state),
            RpAdcRegister::Inte => state.interrupt_enable & 1,
            RpAdcRegister::Intf => state.interrupt_force & 1,
            RpAdcRegister::Ints => {
                Self::fifo_interrupt(&state) & (state.interrupt_enable & 1)
                    | (state.interrupt_force & 1)
            }
        };
        Ok(u64::from(value))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let register = Self::register(offset, width)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("ADC value fits");
        let mut state = self.state.lock().expect("RP ADC lock poisoned");
        match register {
            RpAdcRegister::Cs => {
                let writable = self.cs_writable_mask();
                state.control = (state.control & !writable) | (value & writable);
                if value & Self::CS_ERR_STICKY != 0 {
                    state.control &= !Self::CS_ERR_STICKY;
                }
                if state.control & Self::CS_EN == 0 {
                    state.control &= !Self::CS_READY;
                } else {
                    state.control |= Self::CS_READY;
                }
                if state.control & (Self::CS_START_ONCE | Self::CS_START_MANY) != 0
                    && state.control & Self::CS_EN != 0
                {
                    state.control &= !Self::CS_READY;
                    self.convert(&mut state);
                    if state.control & Self::CS_START_MANY == 0 {
                        state.control &= !Self::CS_START_ONCE;
                    }
                }
            }
            RpAdcRegister::Fcs => {
                state.fifo_control = (state.fifo_control & !Self::FCS_THRESH_MASK & !0x0f)
                    | (value & (Self::FCS_THRESH_MASK | 0x0f));
                if value & Self::FCS_OVER != 0 {
                    state.fifo_over = false;
                }
                if value & Self::FCS_UNDER != 0 {
                    state.fifo_under = false;
                }
            }
            RpAdcRegister::Div => state.divider = value & 0x00ff_ffff,
            RpAdcRegister::Inte => state.interrupt_enable = value & 1,
            RpAdcRegister::Intf => state.interrupt_force = value & 1,
            RpAdcRegister::Result
            | RpAdcRegister::Fifo
            | RpAdcRegister::Intr
            | RpAdcRegister::Ints => {
                return Err(DeviceError::new(format!(
                    "RP ADC {:?} is read-only",
                    register
                )));
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RP ADC lock poisoned") = AdcState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_ids_match_the_native_map() {
        assert_eq!(RpAdcRegister::Intr.offset(), 0x14);
        assert_eq!(RpAdcRegister::Inte.offset(), 0x18);
        assert_eq!(RpAdcRegister::Intf.offset(), 0x1c);
        assert_eq!(RpAdcRegister::Ints.offset(), 0x20);
        assert_eq!(RpAdcRegister::try_from(0x18), Ok(RpAdcRegister::Inte));
        assert!(RpAdcRegister::try_from(0x24).is_err());
    }

    #[test]
    fn deterministic_channel_conversion_sets_ready_and_result() {
        let (mut adc, handle) = RpAdc::new("adc");
        assert!(handle.set_sample(3, 0xabc));
        adc.write(
            RpAdcRegister::Cs.offset(),
            AccessWidth::Word,
            u64::from(RpAdc::CS_EN | RpAdc::CS_START_ONCE | (3 << 12)),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            adc.read(RpAdcRegister::Cs.offset(), AccessWidth::Word, SimTime::ZERO)
                .unwrap()
                & u64::from(RpAdc::CS_READY),
            u64::from(RpAdc::CS_READY)
        );
        assert_eq!(
            adc.read(
                RpAdcRegister::Result.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            0xabc
        );
        assert_eq!(handle.result(), 0xabc);
    }

    #[test]
    fn corrected_interrupt_offsets_and_fifo_status_are_functional() {
        let (mut adc, handle) = RpAdc::new("adc");
        assert!(handle.set_sample(0, 0xabc));
        adc.write(
            RpAdcRegister::Fcs.offset(),
            AccessWidth::Word,
            u64::from((1 << 24) | RpAdc::FCS_EN),
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            RpAdcRegister::Cs.offset(),
            AccessWidth::Word,
            u64::from(RpAdc::CS_EN | RpAdc::CS_START_ONCE),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.fifo_level(), 1);
        assert_eq!(
            adc.read(
                RpAdcRegister::Fcs.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap()
                >> 16
                & 0xf,
            1
        );
        assert_eq!(
            adc.read(
                RpAdcRegister::Intr.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            1
        );
        adc.write(
            RpAdcRegister::Inte.offset(),
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            RpAdcRegister::Intf.offset(),
            AccessWidth::Word,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            adc.read(
                RpAdcRegister::Ints.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            1
        );
    }

    #[test]
    fn fifo_shift_overflow_and_underflow_are_deterministic() {
        let (mut adc, handle) = RpAdc::new("adc");
        adc.write(
            RpAdcRegister::Fcs.offset(),
            AccessWidth::Word,
            u64::from(RpAdc::FCS_EN | RpAdc::FCS_SHIFT),
            SimTime::ZERO,
        )
        .unwrap();
        for value in 0..9_u16 {
            assert!(handle.set_sample(0, value << 4));
            adc.write(
                RpAdcRegister::Cs.offset(),
                AccessWidth::Word,
                u64::from(RpAdc::CS_EN | RpAdc::CS_START_ONCE),
                SimTime::ZERO,
            )
            .unwrap();
        }
        assert_eq!(handle.fifo_level(), 8);
        assert_ne!(
            adc.read(
                RpAdcRegister::Fcs.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap()
                & u64::from(RpAdc::FCS_OVER),
            0
        );
        assert_eq!(
            adc.read(
                RpAdcRegister::Fifo.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap(),
            0
        );
        for _ in 0..7 {
            let _ = adc.read(
                RpAdcRegister::Fifo.offset(),
                AccessWidth::Word,
                SimTime::ZERO,
            );
        }
        assert_eq!(handle.fifo_level(), 0);
        let _ = adc.read(
            RpAdcRegister::Fifo.offset(),
            AccessWidth::Word,
            SimTime::ZERO,
        );
        assert_ne!(
            adc.read(
                RpAdcRegister::Fcs.offset(),
                AccessWidth::Word,
                SimTime::ZERO
            )
            .unwrap()
                & u64::from(RpAdc::FCS_UNDER),
            0
        );
    }

    #[test]
    fn temperature_and_package_masks_follow_variant() {
        let (mut five, five_handle) = RpAdc::new("five");
        assert!(!five_handle.set_sample(8, 1));
        assert!(five_handle.set_sample(4, 0x456));
        five.write(
            RpAdcRegister::Cs.offset(),
            AccessWidth::Word,
            u64::from(RpAdc::CS_EN | RpAdc::CS_START_ONCE | (4 << 12)),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(five_handle.result(), 0);
        five.write(
            RpAdcRegister::Cs.offset(),
            AccessWidth::Word,
            u64::from(RpAdc::CS_EN | RpAdc::CS_TS_EN | RpAdc::CS_START_ONCE | (4 << 12)),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(five_handle.result(), 0x456);

        let (mut nine, nine_handle) = RpAdc::new_for_variant("nine", RpAdcVariant::NineChannel);
        assert!(nine_handle.set_sample(8, 0x789));
        nine.write(
            RpAdcRegister::Cs.offset(),
            AccessWidth::Word,
            u64::from(RpAdc::CS_EN | RpAdc::CS_TS_EN | RpAdc::CS_START_ONCE | (8 << 12)),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(nine_handle.result(), 0x789);
    }
}
