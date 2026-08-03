use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

const CS: u64 = 0x00;
const RESULT: u64 = 0x04;
const FCS: u64 = 0x08;
const FIFO: u64 = 0x0c;
const DIV: u64 = 0x10;
const INTR: u64 = 0x14;
const INTE: u64 = 0x18;
const INTF: u64 = 0x1c;
const INTS: u64 = 0x20;
const EN: u32 = 1;
const TS_EN: u32 = 1 << 1;
const START_ONCE: u32 = 1 << 2;
const START_MANY: u32 = 1 << 3;
const READY: u32 = 1 << 8;
const ERR: u32 = 1 << 9;
const ERR_STICKY: u32 = 1 << 10;
const AINSEL_SHIFT: u32 = 12;
const RROBIN_SHIFT: u32 = 16;
const FCS_EN: u32 = 1;
const FCS_SHIFT: u32 = 1 << 1;
const FCS_ERR: u32 = 1 << 2;
const FCS_DREQ_EN: u32 = 1 << 3;
const FCS_UNDER: u32 = 1 << 10;
const FCS_OVER: u32 = 1 << 11;
const FCS_LEVEL_SHIFT: u32 = 16;
const FCS_THRESH_SHIFT: u32 = 24;
const FIFO_CAPACITY: usize = 8;
const MAX_CHANNELS: usize = 9;

#[derive(Clone)]
struct AdcState {
    control: u32,
    result: u16,
    samples: [u16; MAX_CHANNELS],
    fifo: VecDeque<u32>,
    fifo_control: u32,
    fifo_under: bool,
    fifo_over: bool,
    divider: u32,
    interrupt_enable: u32,
    interrupt_force: u32,
}

impl Default for AdcState {
    fn default() -> Self {
        Self {
            control: 0,
            result: 0,
            samples: [0; MAX_CHANNELS],
            fifo: VecDeque::with_capacity(FIFO_CAPACITY),
            fifo_control: 0,
            fifo_under: false,
            fifo_over: false,
            divider: 0,
            interrupt_enable: 0,
            interrupt_force: 0,
        }
    }
}

/// Host-facing deterministic RP ADC state.
#[derive(Clone)]
pub struct RpAdcHandle(Arc<Mutex<AdcState>>);

impl RpAdcHandle {
    /// Sets the 12-bit sample returned for one ADC input channel.
    pub fn set_sample(&self, channel: usize, value: u16) {
        if let Some(sample) = self
            .0
            .lock()
            .expect("RP ADC lock poisoned")
            .samples
            .get_mut(channel)
        {
            *sample = value & 0x0fff;
        }
    }

    /// Returns the most recently converted sample.
    pub fn result(&self) -> u16 {
        self.0.lock().expect("RP ADC lock poisoned").result
    }
}

/// Functional RP2040/RP2350 ADC and temperature-sensor register subset.
pub struct RpAdc {
    name: String,
    channel_count: usize,
    state: Arc<Mutex<AdcState>>,
}

impl RpAdc {
    /// Creates a reset ADC with the RP2040's four external inputs and sensor.
    pub fn new(name: impl Into<String>) -> (Self, RpAdcHandle) {
        Self::with_channels(name.into(), 5)
    }

    /// Creates a reset ADC with the RP2350's eight external inputs and sensor.
    pub fn new_rp2350(name: impl Into<String>) -> (Self, RpAdcHandle) {
        Self::with_channels(name.into(), MAX_CHANNELS)
    }

    fn with_channels(name: String, channel_count: usize) -> (Self, RpAdcHandle) {
        let state = Arc::new(Mutex::new(AdcState::default()));
        (
            Self {
                name,
                channel_count,
                state: state.clone(),
            },
            RpAdcHandle(state),
        )
    }

    fn access(offset: u64, width: AccessWidth) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new("RP ADC requires aligned word access"));
        }
        Ok(())
    }

    fn ain_mask(&self) -> u32 {
        let width = if self.channel_count <= 5 { 3 } else { 4 };
        ((1_u32 << width) - 1) << AINSEL_SHIFT
    }

    fn rrobin_mask(&self) -> u32 {
        ((1_u32 << self.channel_count.min(9)) - 1) << RROBIN_SHIFT
    }

    fn fifo_status(state: &AdcState) -> u32 {
        let level = u32::try_from(state.fifo.len().min(FIFO_CAPACITY)).expect("FIFO level fits");
        let mut value = state.fifo_control & ((0xf << FCS_THRESH_SHIFT) | 0xf);
        value |= level << FCS_LEVEL_SHIFT;
        if state.fifo_over {
            value |= FCS_OVER;
        }
        if state.fifo_under {
            value |= FCS_UNDER;
        }
        if state.fifo.len() >= FIFO_CAPACITY {
            value |= 1 << 9;
        }
        if state.fifo.is_empty() {
            value |= 1 << 8;
        }
        value
    }

    fn fifo_interrupt(state: &AdcState) -> u32 {
        let threshold = (state.fifo_control >> FCS_THRESH_SHIFT) & 0xf;
        u32::from(
            threshold != 0 && u32::try_from(state.fifo.len()).unwrap_or(u32::MAX) >= threshold,
        )
    }

    fn next_round_robin_channel(&self, state: &AdcState, current: usize) -> Option<usize> {
        let mask =
            ((state.control >> RROBIN_SHIFT) & (self.rrobin_mask() >> RROBIN_SHIFT)) as usize;
        if mask == 0 {
            return None;
        }
        (1..=self.channel_count)
            .map(|step| (current + step) % self.channel_count)
            .find(|channel| mask & (1 << channel) != 0)
    }

    fn convert(&self, state: &mut AdcState) {
        if state.control & EN == 0 {
            return;
        }
        let channel = ((state.control >> AINSEL_SHIFT) & 0xf) as usize;
        state.result = state.samples.get(channel).copied().unwrap_or(0);
        state.control = (state.control | READY) & !ERR;
        if state.fifo_control & FCS_EN != 0 {
            let value = if state.fifo_control & FCS_SHIFT != 0 {
                u32::from(state.result >> 4)
            } else {
                u32::from(state.result)
            };
            if state.fifo.len() == FIFO_CAPACITY {
                state.fifo_over = true;
            } else {
                state.fifo.push_back(value);
            }
        }
        if let Some(next) = self.next_round_robin_channel(state, channel) {
            state.control =
                (state.control & !(0xf << AINSEL_SHIFT)) | ((next as u32) << AINSEL_SHIFT);
        }
    }
}

impl Device for RpAdc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        Self::access(offset, width)?;
        let mut state = self.state.lock().expect("RP ADC lock poisoned");
        let value = match offset {
            CS => state.control,
            RESULT => u32::from(state.result),
            FCS => Self::fifo_status(&state),
            FIFO => state.fifo.pop_front().unwrap_or_else(|| {
                state.fifo_under = true;
                0
            }),
            DIV => state.divider,
            INTR => Self::fifo_interrupt(&state),
            INTE => state.interrupt_enable,
            INTF => state.interrupt_force,
            INTS => (Self::fifo_interrupt(&state) & state.interrupt_enable) | state.interrupt_force,
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP ADC read at {offset:#x}"
                )));
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
        Self::access(offset, width)?;
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("ADC value fits");
        let mut state = self.state.lock().expect("RP ADC lock poisoned");
        match offset {
            CS => {
                let rw_mask = EN | TS_EN | START_MANY | self.ain_mask() | self.rrobin_mask();
                state.control = (state.control & !rw_mask) | (value & rw_mask);
                if value & ERR_STICKY != 0 {
                    state.control &= !ERR_STICKY;
                }
                if state.control & EN != 0 {
                    state.control |= READY;
                } else {
                    state.control &= !READY;
                }
                if value & (START_ONCE | START_MANY) != 0 {
                    self.convert(&mut state);
                }
            }
            FCS => {
                state.fifo_control = value
                    & ((0xf << FCS_THRESH_SHIFT) | FCS_EN | FCS_SHIFT | FCS_ERR | FCS_DREQ_EN);
                if value & FCS_OVER != 0 {
                    state.fifo_over = false;
                }
                if value & FCS_UNDER != 0 {
                    state.fifo_under = false;
                }
            }
            DIV => state.divider = value & 0x00ff_ffff,
            INTE => state.interrupt_enable = value & 1,
            INTF => state.interrupt_force = value & 1,
            RESULT | FIFO | INTR | INTS => {
                return Err(DeviceError::new("RP ADC register is read-only"));
            }
            _ => {
                return Err(DeviceError::new(format!(
                    "unmodeled RP ADC write at {offset:#x}"
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
    fn deterministic_channel_conversion_sets_ready_and_result() {
        let (mut adc, handle) = RpAdc::new("adc");
        handle.set_sample(3, 0xabc);
        adc.write(
            CS,
            AccessWidth::Word,
            u64::from(EN | START_ONCE | (3 << AINSEL_SHIFT)),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            adc.read(CS, AccessWidth::Word, SimTime::ZERO).unwrap() & u64::from(READY),
            u64::from(READY)
        );
        assert_eq!(
            adc.read(RESULT, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xabc
        );
        assert_eq!(handle.result(), 0xabc);
    }

    #[test]
    fn fifo_status_data_and_interrupts_follow_native_offsets() {
        let (mut adc, handle) = RpAdc::new("adc");
        handle.set_sample(0, 0xabc);
        adc.write(
            FCS,
            AccessWidth::Word,
            u64::from(FCS_EN | (1 << FCS_THRESH_SHIFT)),
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            CS,
            AccessWidth::Word,
            u64::from(EN | START_ONCE),
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(INTE, AccessWidth::Word, 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            adc.read(FCS, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from((1 << FCS_LEVEL_SHIFT) | FCS_EN | (1 << FCS_THRESH_SHIFT))
        );
        assert_eq!(adc.read(INTR, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
        assert_eq!(adc.read(INTS, AccessWidth::Word, SimTime::ZERO).unwrap(), 1);
        assert_eq!(
            adc.read(FIFO, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0xabc
        );
        assert_eq!(
            adc.read(FCS, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 8),
            1 << 8
        );
        assert_eq!(adc.read(FIFO, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
        assert_eq!(
            adc.read(FCS, AccessWidth::Word, SimTime::ZERO).unwrap() & u64::from(FCS_UNDER),
            u64::from(FCS_UNDER)
        );
        adc.write(FCS, AccessWidth::Word, u64::from(FCS_UNDER), SimTime::ZERO)
            .unwrap();
        assert_eq!(
            adc.read(FCS, AccessWidth::Word, SimTime::ZERO).unwrap() & u64::from(FCS_UNDER),
            0
        );
    }

    #[test]
    fn rp2350_exposes_eight_external_channels_and_four_bit_ain_selection() {
        let (mut adc, handle) = RpAdc::new_rp2350("rp2350.adc");
        handle.set_sample(8, 0x654);
        adc.write(
            CS,
            AccessWidth::Word,
            u64::from(EN | START_ONCE | (8 << AINSEL_SHIFT)),
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(
            adc.read(RESULT, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0x654
        );
        assert_eq!(
            adc.read(CS, AccessWidth::Word, SimTime::ZERO).unwrap() & (0xf << AINSEL_SHIFT),
            8 << AINSEL_SHIFT
        );
        assert!(adc.read(0x24, AccessWidth::Word, SimTime::ZERO).is_err());
    }

    #[test]
    fn disabled_adc_still_reports_empty_fifo_and_reset_registers() {
        let (mut adc, _) = RpAdc::new("adc");
        assert_eq!(
            adc.read(FCS, AccessWidth::Word, SimTime::ZERO).unwrap(),
            1 << 8
        );
        assert_eq!(
            adc.read(RESULT, AccessWidth::Word, SimTime::ZERO).unwrap(),
            0
        );
        assert_eq!(adc.read(DIV, AccessWidth::Word, SimTime::ZERO).unwrap(), 0);
        assert!(adc.read(0x24, AccessWidth::Word, SimTime::ZERO).is_err());
    }
}
