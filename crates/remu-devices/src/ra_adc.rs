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

/// Named RA4M1 ADC140 register identifiers for the modeled register surface.
///
/// `Addr` covers the 29 consecutive read-only conversion-result registers at
/// `0x20..=0x58`; all other variants correspond to the fixed registers in the
/// R7FA4M1AB ADC0 register block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum RaAdcRegister {
    /// A/D control register (ADCSR).
    Adcsr,
    /// A/D status register (ADREF).
    Adref,
    /// A/D enhancing status register (ADEXREF).
    Adexref,
    /// Group-A channel select register 0 (ADANSA0).
    Adansa0,
    /// Group-A channel select register 1 (ADANSA1).
    Adansa1,
    /// Addition/average channel select register 0 (ADADS0).
    Adads0,
    /// Addition/average channel select register 1 (ADADS1).
    Adads1,
    /// Addition/average count select register (ADADC).
    Adadc,
    /// A/D control extended register (ADCER).
    Adcer,
    /// Conversion start trigger select register (ADSTRGR).
    Adstrgr,
    /// Conversion extended input control register (ADEXICR).
    Adexicr,
    /// Group-B channel select register 0 (ADANSB0).
    Adansb0,
    /// Group-B channel select register 1 (ADANSB1).
    Adansb1,
    /// A/D data duplication register (ADDBLDR).
    Addbldr,
    /// A/D temperature sensor data register (ADTSDR).
    Adtsdr,
    /// A/D internal reference voltage data register (ADOCDR).
    Adocdr,
    /// A/D self-diagnosis data register (ADRD).
    Adrd,
    /// A/D conversion result register ADDR0 through ADDR28.
    Addr(u8),
}

impl RaAdcRegister {
    /// Returns the native ADC140 byte offset.
    pub const fn offset(self) -> u64 {
        match self {
            Self::Adcsr => 0x00,
            Self::Adref => 0x02,
            Self::Adexref => 0x03,
            Self::Adansa0 => 0x04,
            Self::Adansa1 => 0x06,
            Self::Adads0 => 0x08,
            Self::Adads1 => 0x0a,
            Self::Adadc => 0x0c,
            Self::Adcer => 0x0e,
            Self::Adstrgr => 0x10,
            Self::Adexicr => 0x12,
            Self::Adansb0 => 0x14,
            Self::Adansb1 => 0x16,
            Self::Addbldr => 0x18,
            Self::Adtsdr => 0x1a,
            Self::Adocdr => 0x1c,
            Self::Adrd => 0x1e,
            Self::Addr(channel) => 0x20 + channel as u64 * 2,
        }
    }

    /// Returns a stable descriptive register name.
    pub fn name(self) -> String {
        match self {
            Self::Addr(channel) => format!("addr{channel}"),
            Self::Adcsr => "adcsr".to_owned(),
            Self::Adref => "adref".to_owned(),
            Self::Adexref => "adexref".to_owned(),
            Self::Adansa0 => "adansa0".to_owned(),
            Self::Adansa1 => "adansa1".to_owned(),
            Self::Adads0 => "adads0".to_owned(),
            Self::Adads1 => "adads1".to_owned(),
            Self::Adadc => "adadc".to_owned(),
            Self::Adcer => "adcer".to_owned(),
            Self::Adstrgr => "adstrgr".to_owned(),
            Self::Adexicr => "adexicr".to_owned(),
            Self::Adansb0 => "adansb0".to_owned(),
            Self::Adansb1 => "adansb1".to_owned(),
            Self::Addbldr => "addbldr".to_owned(),
            Self::Adtsdr => "adtsdr".to_owned(),
            Self::Adocdr => "adocdr".to_owned(),
            Self::Adrd => "adrd".to_owned(),
        }
    }

    /// Resolves a native ADC140 byte offset to a named register.
    pub const fn from_offset(offset: u64) -> Option<Self> {
        match offset {
            0x00 => Some(Self::Adcsr),
            0x02 => Some(Self::Adref),
            0x03 => Some(Self::Adexref),
            0x04 => Some(Self::Adansa0),
            0x06 => Some(Self::Adansa1),
            0x08 => Some(Self::Adads0),
            0x0a => Some(Self::Adads1),
            0x0c => Some(Self::Adadc),
            0x0e => Some(Self::Adcer),
            0x10 => Some(Self::Adstrgr),
            0x12 => Some(Self::Adexicr),
            0x14 => Some(Self::Adansb0),
            0x16 => Some(Self::Adansb1),
            0x18 => Some(Self::Addbldr),
            0x1a => Some(Self::Adtsdr),
            0x1c => Some(Self::Adocdr),
            0x1e => Some(Self::Adrd),
            0x20..=0x58 if offset & 1 == 0 => Some(Self::Addr(((offset - 0x20) / 2) as u8)),
            _ => None,
        }
    }
}

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
            let accuracy = match (state.adcer >> 1) & 0x3 {
                0 => 14,
                1 => 12,
                2 => 10,
                _ => 8,
            };
            let maximum = (1_u32 << accuracy) - 1;
            for channel in selected_channels(state.adansa) {
                let input = u32::from(state.inputs[channel]);
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
            }
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

fn selected_channels(masks: [u16; 2]) -> Vec<usize> {
    let mut channels = masks
        .into_iter()
        .enumerate()
        .flat_map(|(bank, mask)| {
            (0..16).filter_map(move |bit| (mask & (1 << bit) != 0).then_some(bank * 16 + bit))
        })
        .filter(|channel| *channel < CHANNELS)
        .collect::<Vec<_>>();
    if channels.is_empty() {
        channels.push(0);
    }
    channels
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
        let register =
            RaAdcRegister::from_offset(offset).or_else(|| RaAdcRegister::from_offset(offset & !1));
        match register {
            Some(RaAdcRegister::Adcsr) => Self::read_u16(
                offset,
                width,
                state.adcsr | u16::from(state.active) * ADST,
                RaAdcRegister::Adcsr.offset(),
            ),
            Some(RaAdcRegister::Adref) => lane_u16(
                u16::from(state.adref | (u8::from(state.active) << 7)),
                offset,
                RaAdcRegister::Adref.offset(),
                width,
            ),
            Some(RaAdcRegister::Adexref) => lane_u16(
                u16::from(state.adexref),
                offset,
                RaAdcRegister::Adexref.offset(),
                width,
            ),
            Some(RaAdcRegister::Adansa0 | RaAdcRegister::Adansa1) => {
                let index = usize::from(matches!(register, Some(RaAdcRegister::Adansa1)));
                lane_u16(
                    state.adansa[index],
                    offset,
                    RaAdcRegister::Adansa0.offset() + (index as u64) * 2,
                    width,
                )
            }
            Some(RaAdcRegister::Adads0 | RaAdcRegister::Adads1) => {
                let index = usize::from(matches!(register, Some(RaAdcRegister::Adads1)));
                lane_u16(
                    state.adads[index],
                    offset,
                    RaAdcRegister::Adads0.offset() + (index as u64) * 2,
                    width,
                )
            }
            Some(RaAdcRegister::Adadc) => lane_u16(
                u16::from(state.adadc),
                offset,
                RaAdcRegister::Adadc.offset(),
                width,
            ),
            Some(RaAdcRegister::Adcer) => {
                lane_u16(state.adcer, offset, RaAdcRegister::Adcer.offset(), width)
            }
            Some(RaAdcRegister::Adstrgr) => lane_u16(
                state.adstrgr,
                offset,
                RaAdcRegister::Adstrgr.offset(),
                width,
            ),
            Some(RaAdcRegister::Adexicr) => lane_u16(
                state.adexicr,
                offset,
                RaAdcRegister::Adexicr.offset(),
                width,
            ),
            Some(RaAdcRegister::Adansb0 | RaAdcRegister::Adansb1) => {
                let index = usize::from(matches!(register, Some(RaAdcRegister::Adansb1)));
                lane_u16(
                    state.adansb[index],
                    offset,
                    RaAdcRegister::Adansb0.offset() + (index as u64) * 2,
                    width,
                )
            }
            Some(RaAdcRegister::Addbldr) => lane_u16(
                state.addbldr,
                offset,
                RaAdcRegister::Addbldr.offset(),
                width,
            ),
            Some(RaAdcRegister::Adtsdr) => {
                lane_u16(state.adtsdr, offset, RaAdcRegister::Adtsdr.offset(), width)
            }
            Some(RaAdcRegister::Adocdr) => {
                lane_u16(state.adocdr, offset, RaAdcRegister::Adocdr.offset(), width)
            }
            Some(RaAdcRegister::Adrd) => Ok(0),
            Some(RaAdcRegister::Addr(channel)) => lane_u16(
                state.data[usize::from(channel)],
                offset,
                RaAdcRegister::Addr(channel).offset(),
                width,
            ),
            None => Ok(0),
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
        let register =
            RaAdcRegister::from_offset(offset).or_else(|| RaAdcRegister::from_offset(offset & !1));
        match register {
            Some(RaAdcRegister::Adcsr) => {
                let current = state.adcsr;
                let updated =
                    merge_u16(current, offset, RaAdcRegister::Adcsr.offset(), width, value)?;
                state.adcsr = updated & !ADST;
                if updated & ADST == 0 {
                    state.active = false;
                } else if !state.active {
                    state.active = true;
                    state.started = at.ticks();
                    state.adref &= !ADF;
                }
            }
            Some(RaAdcRegister::Adref) => {
                let current = u16::from(state.adref);
                state.adref =
                    merge_u16(current, offset, RaAdcRegister::Adref.offset(), width, value)? as u8
                        & ADF;
            }
            Some(RaAdcRegister::Adexref) => {
                state.adexref = merge_u16(
                    u16::from(state.adexref),
                    offset,
                    RaAdcRegister::Adexref.offset(),
                    width,
                    value,
                )? as u8
            }
            Some(RaAdcRegister::Adansa0 | RaAdcRegister::Adansa1) => {
                let index = usize::from(matches!(register, Some(RaAdcRegister::Adansa1)));
                let base = RaAdcRegister::Adansa0.offset() + (index as u64) * 2;
                state.adansa[index] = merge_u16(state.adansa[index], offset, base, width, value)?;
            }
            Some(RaAdcRegister::Adads0 | RaAdcRegister::Adads1) => {
                let index = usize::from(matches!(register, Some(RaAdcRegister::Adads1)));
                let base = RaAdcRegister::Adads0.offset() + (index as u64) * 2;
                state.adads[index] = merge_u16(state.adads[index], offset, base, width, value)?;
            }
            Some(RaAdcRegister::Adadc) => {
                state.adadc = merge_u16(
                    u16::from(state.adadc),
                    offset,
                    RaAdcRegister::Adadc.offset(),
                    width,
                    value,
                )? as u8
            }
            Some(RaAdcRegister::Adcer) => {
                state.adcer = merge_u16(
                    state.adcer,
                    offset,
                    RaAdcRegister::Adcer.offset(),
                    width,
                    value,
                )?
            }
            Some(RaAdcRegister::Adstrgr) => {
                state.adstrgr = merge_u16(
                    state.adstrgr,
                    offset,
                    RaAdcRegister::Adstrgr.offset(),
                    width,
                    value,
                )?
            }
            Some(RaAdcRegister::Adexicr) => {
                state.adexicr = merge_u16(
                    state.adexicr,
                    offset,
                    RaAdcRegister::Adexicr.offset(),
                    width,
                    value,
                )?
            }
            Some(RaAdcRegister::Adansb0 | RaAdcRegister::Adansb1) => {
                let index = usize::from(matches!(register, Some(RaAdcRegister::Adansb1)));
                let base = RaAdcRegister::Adansb0.offset() + (index as u64) * 2;
                state.adansb[index] = merge_u16(state.adansb[index], offset, base, width, value)?;
            }
            Some(
                RaAdcRegister::Addbldr
                | RaAdcRegister::Adtsdr
                | RaAdcRegister::Adocdr
                | RaAdcRegister::Adrd
                | RaAdcRegister::Addr(_),
            )
            | None => {}
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
    fn register_ids_cover_fixed_and_result_windows() {
        assert_eq!(RaAdcRegister::Adcsr.offset(), 0x00);
        assert_eq!(RaAdcRegister::Adcsr.name(), "adcsr");
        assert_eq!(
            RaAdcRegister::from_offset(0x06),
            Some(RaAdcRegister::Adansa1)
        );
        assert_eq!(
            RaAdcRegister::from_offset(0x58),
            Some(RaAdcRegister::Addr(28))
        );
        assert_eq!(RaAdcRegister::Addr(28).name(), "addr28");
        assert_eq!(RaAdcRegister::from_offset(0x59), None);
    }

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

    #[test]
    fn single_scan_updates_multiple_channels_in_both_selection_banks() {
        let (mut adc, handle) = RaAdc::new("adc0");
        handle.set_input(0, 0x1000).unwrap();
        handle.set_input(16, 0x3000).unwrap();
        adc.write(
            RaAdcRegister::Adansa0.offset(),
            AccessWidth::HalfWord,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            RaAdcRegister::Adansa1.offset(),
            AccessWidth::HalfWord,
            1,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            RaAdcRegister::Adcsr.offset(),
            AccessWidth::HalfWord,
            (1_u64 << 15) | u64::from(ADIE),
            SimTime::ZERO,
        )
        .unwrap();
        assert!(handle.poll(SimTime::from_ticks(CONVERSION_TICKS)));
        assert_eq!(handle.sample(0), Some(0x1000));
        assert_eq!(handle.sample(16), Some(0x3000));
        assert_eq!(
            adc.read(
                RaAdcRegister::Addr(16).offset() + 1,
                AccessWidth::Byte,
                SimTime::ZERO,
            )
            .unwrap(),
            0x30
        );
    }

    #[test]
    fn clearing_adst_aborts_an_in_progress_scan() {
        let (mut adc, handle) = RaAdc::new("adc0");
        handle.set_input(28, 0x3fff).unwrap();
        adc.write(
            RaAdcRegister::Adansa1.offset(),
            AccessWidth::HalfWord,
            1 << 12,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            RaAdcRegister::Adcsr.offset(),
            AccessWidth::HalfWord,
            1 << 15,
            SimTime::ZERO,
        )
        .unwrap();
        adc.write(
            RaAdcRegister::Adcsr.offset(),
            AccessWidth::HalfWord,
            0,
            SimTime::from_ticks(2),
        )
        .unwrap();
        assert!(!handle.poll(SimTime::from_ticks(CONVERSION_TICKS)));
        assert_eq!(handle.sample(28), Some(0));
    }
}
