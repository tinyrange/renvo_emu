use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

/// RA4M1 ELC event number for ADC140 group-A scan completion.
pub const RA4M1_EVENT_ADC0_SCAN_END: u16 = 41;

const CHANNELS: usize = 29;
const ADST: u16 = 1 << 15;
const ADIE: u16 = 1 << 12;
const ADF: u8 = 1;
const CONVERSION_TICKS: u64 = 8;

#[derive(Clone)]
struct AdcState {
    adcsr: u16,
    adref: u8,
    adexref: u8,
    adansa: [u16; 2],
    adads: [u16; 2],
    adadc: u8,
    adcer: u16,
    adstrgr: u16,
    adexicr: u16,
    adansb: [u16; 2],
    addbldr: u16,
    adtsdr: u16,
    adocdr: u16,
    data: [u16; CHANNELS],
    inputs: [u16; CHANNELS],
    active: bool,
    started: u64,
}

impl Default for AdcState {
    fn default() -> Self {
        Self {
            adcsr: 0,
            adref: 0,
            adexref: 0,
            adansa: [0; 2],
            adads: [0; 2],
            adadc: 0,
            adcer: 0,
            adstrgr: 0,
            adexicr: 0,
            adansb: [0; 2],
            addbldr: 0,
            adtsdr: 0,
            adocdr: 0,
            data: [0; CHANNELS],
            inputs: [0; CHANNELS],
            active: false,
            started: 0,
        }
    }
}

/// Host-facing ADC140 input and scan-completion state.
#[derive(Clone)]
pub struct RaAdcHandle(Arc<Mutex<AdcState>>);

impl RaAdcHandle {
    /// Sets one externally driven analog channel in native 14-bit units.
    pub fn set_input(&self, channel: u8, value: u16) -> Result<(), String> {
        let channel = usize::from(channel);
        let mut state = self.0.lock().expect("RA ADC lock poisoned");
        let Some(input) = state.inputs.get_mut(channel) else {
            return Err(format!("RA ADC channel {channel} is out of range"));
        };
        *input = value.min(0x3fff);
        Ok(())
    }

    /// Advances a conversion and returns the enabled scan-end interrupt level.
    pub fn poll(&self, now: SimTime) -> bool {
        let mut state = self.0.lock().expect("RA ADC lock poisoned");
        if state.active && now.ticks().saturating_sub(state.started) >= CONVERSION_TICKS {
            let channel = selected_channel(state.adansa[0]);
            let input = u32::from(state.inputs[channel]);
            let accuracy = match (state.adcer >> 1) & 0x3 {
                0 => 14,
                1 => 12,
                2 => 10,
                _ => 8,
            };
            let maximum = (1_u32 << accuracy) - 1;
            let mut value = (input.saturating_mul(maximum) + 0x1fff) / 0x3fff;
            if state.adcer & (1 << 14) != 0 {
                value = maximum.saturating_sub(value);
            }
            let value = if state.adcer & (1 << 15) != 0 {
                value << (16 - accuracy)
            } else {
                value
            } as u16;
            state.data[channel] = value;
            state.adref |= ADF;
            state.adcsr &= !ADST;
            state.active = false;
        }
        state.adref & ADF != 0 && state.adcsr & ADIE != 0
    }

    /// Returns the most recently converted channel value.
    pub fn sample(&self, channel: u8) -> Option<u16> {
        self.0
            .lock()
            .expect("RA ADC lock poisoned")
            .data
            .get(usize::from(channel))
            .copied()
    }
}

fn selected_channel(mask: u16) -> usize {
    if mask == 0 {
        0
    } else {
        usize::try_from(mask.trailing_zeros())
            .unwrap_or(0)
            .min(CHANNELS - 1)
    }
}

fn lane_u16(value: u16, offset: u64, base: u64, width: AccessWidth) -> Result<u64, DeviceError> {
    let relative = offset.saturating_sub(base);
    let shift = usize::try_from(relative)
        .map_err(|_| DeviceError::new("RA ADC register offset overflow"))?
        .saturating_mul(8);
    let bits = usize::from(width.bytes()) * 8;
    if shift + bits > 16 {
        return Err(DeviceError::new("RA ADC access crosses a halfword"));
    }
    let mask = if bits == 16 {
        u16::MAX
    } else {
        (1_u16 << bits) - 1
    };
    Ok(u64::from((value >> shift) & mask))
}

fn merge_u16(
    current: u16,
    offset: u64,
    base: u64,
    width: AccessWidth,
    value: u64,
) -> Result<u16, DeviceError> {
    let relative = offset.saturating_sub(base);
    let shift = usize::try_from(relative)
        .map_err(|_| DeviceError::new("RA ADC register offset overflow"))?
        .saturating_mul(8);
    let bits = usize::from(width.bytes()) * 8;
    if shift + bits > 16 {
        return Err(DeviceError::new("RA ADC access crosses a halfword"));
    }
    let lane = if bits == 16 {
        u16::MAX
    } else {
        (1_u16 << bits) - 1
    };
    let mask = lane << shift;
    Ok((current & !mask) | (((value as u16) & lane) << shift))
}

/// Functional RA4M1 ADC140 single-scan register slice.
pub struct RaAdc {
    name: String,
    state: Arc<Mutex<AdcState>>,
}

impl RaAdc {
    /// Creates ADC140 and its host input/completion handle.
    pub fn new(name: impl Into<String>) -> (Self, RaAdcHandle) {
        let state = Arc::new(Mutex::new(AdcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            RaAdcHandle(state),
        )
    }

    fn read_u16(
        offset: u64,
        width: AccessWidth,
        value: u16,
        base: u64,
    ) -> Result<u64, DeviceError> {
        lane_u16(value, offset, base, width)
    }
}

impl Device for RaAdc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let state = self.state.lock().expect("RA ADC lock poisoned");
        match offset {
            0x00 | 0x01 => Self::read_u16(
                offset,
                width,
                state.adcsr | u16::from(state.active) * ADST,
                0x00,
            ),
            0x02 => lane_u16(u16::from(state.adref), offset, 0x02, width),
            0x03 => lane_u16(u16::from(state.adexref), offset, 0x03, width),
            0x04..=0x06 => {
                let index = usize::try_from((offset - 0x04) / 2).expect("ADANSA index fits usize");
                lane_u16(
                    state.adansa[index],
                    offset,
                    0x04 + (index as u64) * 2,
                    width,
                )
            }
            0x08..=0x0a => {
                let index = usize::try_from((offset - 0x08) / 2).expect("ADADS index fits usize");
                lane_u16(state.adads[index], offset, 0x08 + (index as u64) * 2, width)
            }
            0x0c => lane_u16(u16::from(state.adadc), offset, 0x0c, width),
            0x0e => lane_u16(state.adcer, offset, 0x0e, width),
            0x10 => lane_u16(state.adstrgr, offset, 0x10, width),
            0x12 => lane_u16(state.adexicr, offset, 0x12, width),
            0x14..=0x16 => {
                let index = usize::try_from((offset - 0x14) / 2).expect("ADANSB index fits usize");
                lane_u16(
                    state.adansb[index],
                    offset,
                    0x14 + (index as u64) * 2,
                    width,
                )
            }
            0x18 => lane_u16(state.addbldr, offset, 0x18, width),
            0x1a => lane_u16(state.adtsdr, offset, 0x1a, width),
            0x1c => lane_u16(state.adocdr, offset, 0x1c, width),
            0x1e => Ok(0),
            0x20..=0x58 if offset & 1 == 0 => {
                let channel = usize::try_from((offset - 0x20) / 2).expect("ADDR index fits usize");
                state
                    .data
                    .get(channel)
                    .copied()
                    .map(u64::from)
                    .ok_or_else(|| {
                        DeviceError::new(format!("unmodeled RA ADC data read at {offset:#x}"))
                    })
            }
            _ => Ok(0),
        }
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        let mut state = self.state.lock().expect("RA ADC lock poisoned");
        match offset {
            0x00 | 0x01 => {
                let current = state.adcsr;
                let updated = merge_u16(current, offset, 0x00, width, value)?;
                state.adcsr = updated & !ADST;
                if updated & ADST != 0 && !state.active {
                    state.active = true;
                    state.started = at.ticks();
                    state.adref &= !ADF;
                }
            }
            0x02 => {
                let current = u16::from(state.adref);
                state.adref = merge_u16(current, offset, 0x02, width, value)? as u8 & ADF;
            }
            0x03 => {
                state.adexref =
                    merge_u16(u16::from(state.adexref), offset, 0x03, width, value)? as u8
            }
            0x04..=0x06 => {
                let index = usize::try_from((offset - 0x04) / 2).expect("ADANSA index fits usize");
                let base = 0x04 + (index as u64) * 2;
                state.adansa[index] = merge_u16(state.adansa[index], offset, base, width, value)?;
            }
            0x08..=0x0a => {
                let index = usize::try_from((offset - 0x08) / 2).expect("ADADS index fits usize");
                let base = 0x08 + (index as u64) * 2;
                state.adads[index] = merge_u16(state.adads[index], offset, base, width, value)?;
            }
            0x0c => {
                state.adadc = merge_u16(u16::from(state.adadc), offset, 0x0c, width, value)? as u8
            }
            0x0e => state.adcer = merge_u16(state.adcer, offset, 0x0e, width, value)?,
            0x10 => state.adstrgr = merge_u16(state.adstrgr, offset, 0x10, width, value)?,
            0x12 => state.adexicr = merge_u16(state.adexicr, offset, 0x12, width, value)?,
            0x14..=0x16 => {
                let index = usize::try_from((offset - 0x14) / 2).expect("ADANSB index fits usize");
                let base = 0x14 + (index as u64) * 2;
                state.adansb[index] = merge_u16(state.adansb[index], offset, base, width, value)?;
            }
            0x18 | 0x1a | 0x1c => {}
            0x1e..=0x5a => {}
            _ => {}
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        *self.state.lock().expect("RA ADC lock poisoned") = AdcState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_scan_quantizes_host_input_and_sets_scan_end() {
        let (mut adc, handle) = RaAdc::new("adc0");
        handle.set_input(3, 0x2aaa).unwrap();
        adc.write(0x04, AccessWidth::HalfWord, 1 << 3, SimTime::ZERO)
            .unwrap();
        adc.write(
            0x00,
            AccessWidth::HalfWord,
            (1 << 15) | (1 << 12),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(CONVERSION_TICKS - 1)));
        assert!(handle.poll(SimTime::from_ticks(CONVERSION_TICKS)));
        assert_eq!(
            adc.read(0x20 + 3 * 2, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0x2aaa
        );
        assert_eq!(adc.read(0x02, AccessWidth::Byte, SimTime::ZERO).unwrap(), 1);
    }

    #[test]
    fn left_format_and_input_range_are_deterministic() {
        let (mut adc, handle) = RaAdc::new("adc0");
        handle.set_input(0, 0x3fff).unwrap();
        adc.write(0x04, AccessWidth::HalfWord, 1, SimTime::ZERO)
            .unwrap();
        adc.write(
            0x0e,
            AccessWidth::HalfWord,
            (1 << 15) | (1 << 1),
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            0x00,
            AccessWidth::HalfWord,
            (1 << 15) | (1 << 12),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.poll(SimTime::from_ticks(CONVERSION_TICKS)));
        assert_eq!(handle.sample(0), Some(0xfff0));
        assert!(handle.set_input(29, 0).is_err());
    }
}
