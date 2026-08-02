use remu_bus::{Device, DeviceError};
use remu_core::{AccessWidth, ResetKind, SimTime};
use std::sync::{Arc, Mutex};

fn width_bytes(width: AccessWidth) -> usize {
    usize::from(width.bytes())
}

fn read_le(bytes: &[u8], offset: usize, width: AccessWidth) -> Result<u64, DeviceError> {
    let end = offset
        .checked_add(width_bytes(width))
        .ok_or_else(|| DeviceError::new("ADC register access overflow"))?;
    let slice = bytes
        .get(offset..end)
        .ok_or_else(|| DeviceError::new("ADC register access exceeds register"))?;
    Ok(slice
        .iter()
        .enumerate()
        .fold(0_u64, |value, (shift, byte)| {
            value | (u64::from(*byte) << (shift * 8))
        }))
}

fn write_le(
    bytes: &mut [u8],
    offset: usize,
    width: AccessWidth,
    value: u64,
) -> Result<(), DeviceError> {
    let end = offset
        .checked_add(width_bytes(width))
        .ok_or_else(|| DeviceError::new("ADC register access overflow"))?;
    let slice = bytes
        .get_mut(offset..end)
        .ok_or_else(|| DeviceError::new("ADC register access exceeds register"))?;
    for (shift, byte) in slice.iter_mut().enumerate() {
        *byte = (value >> (shift * 8)) as u8;
    }
    Ok(())
}

/// Native ATSAMD21 ADC register identifiers.
///
/// The offsets and widths follow the SAM D21/DA1 family data sheet, section
/// 33.7. Keeping this list typed makes firmware fixtures and device tests
/// self-documenting and avoids scattering integer register IDs through the
/// machine model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Samd21AdcRegister {
    /// Control A, offset `0x00`.
    Ctrla,
    /// Reference selection, offset `0x01`.
    Refctrl,
    /// Oversampling/averaging control, offset `0x02`.
    Avgctrl,
    /// Sampling-time control, offset `0x03`.
    Sampctrl,
    /// Control B, offset `0x04`.
    Ctrlb,
    /// Window monitor mode, offset `0x08`.
    Winctrl,
    /// Software start/flush trigger, offset `0x0c`.
    Swtrig,
    /// Positive/negative input selection, offset `0x10`.
    Inputctrl,
    /// Event input/output selection, offset `0x14`.
    Evctrl,
    /// Interrupt enable clear alias, offset `0x16`.
    Intenclr,
    /// Interrupt enable set alias, offset `0x17`.
    Intenset,
    /// Interrupt flags, offset `0x18`.
    Intflag,
    /// Synchronization status, offset `0x19`.
    Status,
    /// Conversion result, offset `0x1a`.
    Result,
    /// Window lower threshold, offset `0x1c`.
    Winlt,
    /// Window upper threshold, offset `0x20`.
    Winut,
    /// Gain correction, offset `0x24`.
    Gaincorr,
    /// Offset correction, offset `0x26`.
    Offsetcorr,
    /// Factory calibration, offset `0x28`.
    Calib,
    /// Debug-run control, offset `0x2a`.
    Dbgctrl,
}

impl Samd21AdcRegister {
    /// Returns the native byte offset of this register.
    pub const fn offset(self) -> usize {
        match self {
            Self::Ctrla => 0x00,
            Self::Refctrl => 0x01,
            Self::Avgctrl => 0x02,
            Self::Sampctrl => 0x03,
            Self::Ctrlb => 0x04,
            Self::Winctrl => 0x08,
            Self::Swtrig => 0x0c,
            Self::Inputctrl => 0x10,
            Self::Evctrl => 0x14,
            Self::Intenclr => 0x16,
            Self::Intenset => 0x17,
            Self::Intflag => 0x18,
            Self::Status => 0x19,
            Self::Result => 0x1a,
            Self::Winlt => 0x1c,
            Self::Winut => 0x20,
            Self::Gaincorr => 0x24,
            Self::Offsetcorr => 0x26,
            Self::Calib => 0x28,
            Self::Dbgctrl => 0x2a,
        }
    }

    const fn size(self) -> usize {
        match self {
            Self::Ctrlb
            | Self::Result
            | Self::Winlt
            | Self::Winut
            | Self::Gaincorr
            | Self::Offsetcorr
            | Self::Calib => 2,
            Self::Inputctrl => 4,
            _ => 1,
        }
    }

    fn locate(offset: u64, width: AccessWidth) -> Result<(Self, usize), DeviceError> {
        let offset = usize::try_from(offset)
            .map_err(|_| DeviceError::new("ADC register offset overflow"))?;
        let requested = width_bytes(width);
        let registers = [
            Self::Ctrla,
            Self::Refctrl,
            Self::Avgctrl,
            Self::Sampctrl,
            Self::Ctrlb,
            Self::Winctrl,
            Self::Swtrig,
            Self::Inputctrl,
            Self::Evctrl,
            Self::Intenclr,
            Self::Intenset,
            Self::Intflag,
            Self::Status,
            Self::Result,
            Self::Winlt,
            Self::Winut,
            Self::Gaincorr,
            Self::Offsetcorr,
            Self::Calib,
            Self::Dbgctrl,
        ];
        registers
            .into_iter()
            .find_map(|register| {
                let start = register.offset();
                let end = start.checked_add(register.size())?;
                let access_end = offset.checked_add(requested)?;
                (offset >= start && access_end <= end).then_some((register, offset - start))
            })
            .ok_or_else(|| DeviceError::new(format!("unmodeled ADC access at {offset:#x}")))
    }
}

const INT_RES_RDY: u8 = 1 << 0;
const INT_OVERRUN: u8 = 1 << 1;
const INT_WINMON: u8 = 1 << 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AdcRegisters {
    ctrla: u8,
    refctrl: u8,
    avgctrl: u8,
    sampctrl: u8,
    ctrlb: u16,
    winctrl: u8,
    evctrl: u8,
    interrupt_enable: u8,
    interrupt_flags: u8,
    status: u8,
    result: u16,
    winlt: u16,
    winut: u16,
    gaincorr: u16,
    offsetcorr: u16,
    calib: u16,
    dbgctrl: u8,
    inputctrl: u32,
}

impl Default for AdcRegisters {
    fn default() -> Self {
        Self {
            ctrla: 0,
            refctrl: 0,
            avgctrl: 0,
            sampctrl: 0,
            ctrlb: 0,
            winctrl: 0,
            evctrl: 0,
            interrupt_enable: 0,
            interrupt_flags: 0,
            status: 0,
            result: 0,
            winlt: 0,
            winut: 0,
            gaincorr: 0,
            offsetcorr: 0,
            calib: 0,
            dbgctrl: 0,
            inputctrl: 0,
        }
    }
}

#[derive(Default)]
struct AdcState {
    registers: AdcRegisters,
    samples: [u16; 32],
    result_valid: bool,
}

/// Host-facing handle for deterministic external ADC channel stimulus.
#[derive(Clone)]
pub struct Samd21AdcHandle(Arc<Mutex<AdcState>>);

impl Samd21AdcHandle {
    /// Supplies a 12-bit sample for a positive input mux channel.
    pub fn inject_sample(&self, channel: u8, value: u16) -> Result<(), DeviceError> {
        let mut state = self.0.lock().expect("ADC lock poisoned");
        let Some(sample) = state.samples.get_mut(usize::from(channel)) else {
            return Err(DeviceError::new(format!(
                "ADC channel {channel} is out of range"
            )));
        };
        *sample = value & 0x0fff;
        Ok(())
    }

    /// Returns the most recently latched result.
    pub fn result(&self) -> u16 {
        self.0.lock().expect("ADC lock poisoned").registers.result
    }

    /// Returns whether an enabled ADC interrupt source is pending.
    pub fn interrupt_pending(&self) -> bool {
        let state = self.0.lock().expect("ADC lock poisoned");
        state.registers.interrupt_flags & state.registers.interrupt_enable != 0
    }

    /// Returns the current interrupt flags, including disabled sources.
    pub fn interrupt_flags(&self) -> u8 {
        self.0
            .lock()
            .expect("ADC lock poisoned")
            .registers
            .interrupt_flags
    }
}

/// Functional ATSAMD21 ADC control, input selection, and conversion slice.
///
/// A conversion consumes a deterministic host-provided sample when firmware
/// writes `SWTRIG.START`. This intentionally models functional firmware
/// behavior without pretending to model analog voltage, reference impedance,
/// conversion clocks, or DMA timing.
pub struct Samd21Adc {
    name: String,
    state: Arc<Mutex<AdcState>>,
}

impl Samd21Adc {
    /// Constructs an ADC and its external-stimulus handle.
    pub fn new(name: impl Into<String>) -> (Self, Samd21AdcHandle) {
        let state = Arc::new(Mutex::new(AdcState::default()));
        (
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Samd21AdcHandle(state),
        )
    }

    fn read_register(state: &AdcState, register: Samd21AdcRegister) -> u32 {
        let registers = &state.registers;
        let value = match register {
            Samd21AdcRegister::Ctrla => u32::from(registers.ctrla & 0x06),
            Samd21AdcRegister::Refctrl => u32::from(registers.refctrl & 0x8f),
            Samd21AdcRegister::Avgctrl => u32::from(registers.avgctrl & 0x7f),
            Samd21AdcRegister::Sampctrl => u32::from(registers.sampctrl & 0x3f),
            Samd21AdcRegister::Ctrlb => u32::from(registers.ctrlb & 0x073f),
            Samd21AdcRegister::Winctrl => u32::from(registers.winctrl & 0x07),
            Samd21AdcRegister::Swtrig => 0,
            Samd21AdcRegister::Inputctrl => registers.inputctrl & 0x0fff_ffff,
            Samd21AdcRegister::Evctrl => u32::from(registers.evctrl & 0x37),
            Samd21AdcRegister::Intenclr | Samd21AdcRegister::Intenset => {
                u32::from(registers.interrupt_enable & 0x0f)
            }
            Samd21AdcRegister::Intflag => u32::from(registers.interrupt_flags & 0x0f),
            Samd21AdcRegister::Status => u32::from(registers.status & 0x80),
            Samd21AdcRegister::Result => u32::from(registers.result),
            Samd21AdcRegister::Winlt => u32::from(registers.winlt),
            Samd21AdcRegister::Winut => u32::from(registers.winut),
            Samd21AdcRegister::Gaincorr => u32::from(registers.gaincorr & 0x0fff),
            Samd21AdcRegister::Offsetcorr => u32::from(registers.offsetcorr & 0x0fff),
            Samd21AdcRegister::Calib => u32::from(registers.calib & 0x07ff),
            Samd21AdcRegister::Dbgctrl => u32::from(registers.dbgctrl & 0x01),
        };
        value
    }

    fn conversion_value(state: &mut AdcState) -> u16 {
        let muxpos = state.registers.inputctrl & 0x1f;
        let inputscan = (state.registers.inputctrl >> 16) & 0x0f;
        let inputoffset = (state.registers.inputctrl >> 20) & 0x0f;
        let channel = muxpos.saturating_add(inputoffset) as usize;
        let mut value = state.samples[channel.min(state.samples.len() - 1)] & 0x0fff;
        if inputscan != 0 {
            let next_offset = if inputoffset >= inputscan {
                0
            } else {
                inputoffset + 1
            };
            state.registers.inputctrl =
                (state.registers.inputctrl & !(0x0f << 20)) | (next_offset << 20);
        }
        let resolution = (state.registers.ctrlb >> 4) & 0x03;
        value = match resolution {
            2 => value >> 2,
            3 => value >> 4,
            _ => value,
        };
        if state.registers.ctrlb & (1 << 1) != 0 {
            value = value.checked_shl(4).unwrap_or(0);
        }
        value
    }

    fn complete_conversion(state: &mut AdcState) {
        if state.result_valid {
            state.registers.interrupt_flags |= INT_OVERRUN;
        }
        let result = Self::conversion_value(state);
        state.registers.result = result;
        state.result_valid = true;
        state.registers.interrupt_flags |= INT_RES_RDY;
        let mode = state.registers.winctrl & 0x07;
        let in_window = match mode {
            1 => result > state.registers.winlt,
            2 => result < state.registers.winut,
            3 => result > state.registers.winlt && result < state.registers.winut,
            4 => !(result > state.registers.winlt && result < state.registers.winut),
            _ => false,
        };
        if in_window {
            state.registers.interrupt_flags |= INT_WINMON;
        }
    }

    fn write_register(
        state: &mut AdcState,
        register: Samd21AdcRegister,
        byte_offset: usize,
        width: AccessWidth,
        value: u64,
    ) -> Result<(), DeviceError> {
        let mut raw = Self::read_register(state, register);
        let mut bytes = [0_u8; 4];
        bytes[..register.size()].copy_from_slice(&raw.to_le_bytes()[..register.size()]);
        write_le(&mut bytes, byte_offset, width, value)?;
        raw = u32::from_le_bytes(bytes);
        let value = raw;
        match register {
            Samd21AdcRegister::Ctrla => {
                let write = value as u8;
                if write & 1 != 0 {
                    let dbgctrl = state.registers.dbgctrl;
                    state.registers = AdcRegisters::default();
                    state.registers.dbgctrl = dbgctrl;
                    state.result_valid = false;
                } else {
                    state.registers.ctrla = write & 0x06;
                }
            }
            Samd21AdcRegister::Refctrl => state.registers.refctrl = value as u8 & 0x8f,
            Samd21AdcRegister::Avgctrl => state.registers.avgctrl = value as u8 & 0x7f,
            Samd21AdcRegister::Sampctrl => state.registers.sampctrl = value as u8 & 0x3f,
            Samd21AdcRegister::Ctrlb => state.registers.ctrlb = value as u16 & 0x073f,
            Samd21AdcRegister::Winctrl => state.registers.winctrl = value as u8 & 0x07,
            Samd21AdcRegister::Swtrig => {
                let write = value as u8;
                if write & 1 != 0 {
                    state.result_valid = false;
                }
                if write & 2 != 0 && state.registers.ctrla & 2 != 0 {
                    Self::complete_conversion(state);
                }
            }
            Samd21AdcRegister::Inputctrl => state.registers.inputctrl = value & 0x0fff_ffff,
            Samd21AdcRegister::Evctrl => state.registers.evctrl = value as u8 & 0x37,
            Samd21AdcRegister::Intenclr => {
                state.registers.interrupt_enable &= !(value as u8 & 0x0f);
            }
            Samd21AdcRegister::Intenset => {
                state.registers.interrupt_enable |= value as u8 & 0x0f;
            }
            Samd21AdcRegister::Intflag => {
                state.registers.interrupt_flags &= !(value as u8 & 0x0f);
                if value as u8 & INT_RES_RDY != 0 {
                    state.result_valid = false;
                }
            }
            Samd21AdcRegister::Status => {}
            Samd21AdcRegister::Result => {}
            Samd21AdcRegister::Winlt => state.registers.winlt = value as u16,
            Samd21AdcRegister::Winut => state.registers.winut = value as u16,
            Samd21AdcRegister::Gaincorr => state.registers.gaincorr = value as u16 & 0x0fff,
            Samd21AdcRegister::Offsetcorr => state.registers.offsetcorr = value as u16 & 0x0fff,
            Samd21AdcRegister::Calib => state.registers.calib = value as u16 & 0x07ff,
            Samd21AdcRegister::Dbgctrl => state.registers.dbgctrl = value as u8 & 0x01,
        }
        Ok(())
    }
}

impl Device for Samd21Adc {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        let (register, byte_offset) = Samd21AdcRegister::locate(offset, width)?;
        let mut state = self.state.lock().expect("ADC lock poisoned");
        let raw = Self::read_register(&state, register);
        let value = read_le(&raw.to_le_bytes(), byte_offset, width)?;
        if register == Samd21AdcRegister::Result {
            state.registers.interrupt_flags &= !(INT_RES_RDY | INT_WINMON);
            state.result_valid = false;
        }
        Ok(value)
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        _at: SimTime,
    ) -> Result<(), DeviceError> {
        let (register, byte_offset) = Samd21AdcRegister::locate(offset, width)?;
        let mut state = self.state.lock().expect("ADC lock poisoned");
        Self::write_register(&mut state, register, byte_offset, width, value)
    }

    fn reset(&mut self, _kind: ResetKind) {
        let mut state = self.state.lock().expect("ADC lock poisoned");
        let dbgctrl = state.registers.dbgctrl;
        *state = AdcState::default();
        state.registers.dbgctrl = dbgctrl;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_registers_match_samd21_offsets() {
        assert_eq!(Samd21AdcRegister::Ctrla.offset(), 0x00);
        assert_eq!(Samd21AdcRegister::Inputctrl.offset(), 0x10);
        assert_eq!(Samd21AdcRegister::Result.offset(), 0x1a);
        assert_eq!(Samd21AdcRegister::Dbgctrl.offset(), 0x2a);
    }

    #[test]
    fn host_sample_is_latched_by_software_start_and_read_clears_ready() {
        let (mut adc, handle) = Samd21Adc::new("adc");
        handle.inject_sample(3, 0x0abc).unwrap();
        adc.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        adc.write(0x10, AccessWidth::Word, 3, SimTime::ZERO)
            .unwrap();
        adc.write(0x17, AccessWidth::Byte, INT_RES_RDY as u64, SimTime::ZERO)
            .unwrap();
        adc.write(0x0c, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.result(), 0x0abc);
        assert_eq!(handle.interrupt_flags(), INT_RES_RDY);
        assert!(handle.interrupt_pending());
        assert_eq!(
            adc.read(0x1a, AccessWidth::HalfWord, SimTime::ZERO)
                .unwrap(),
            0x0abc
        );
        assert_eq!(handle.interrupt_flags(), 0);
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn a_second_unread_conversion_sets_overrun_and_window_match() {
        let (mut adc, handle) = Samd21Adc::new("adc");
        handle.inject_sample(0, 0x0100).unwrap();
        adc.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        adc.write(0x08, AccessWidth::Byte, 1, SimTime::ZERO)
            .unwrap();
        adc.write(0x1c, AccessWidth::HalfWord, 0x00ff, SimTime::ZERO)
            .unwrap();
        adc.write(0x0c, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        adc.write(0x0c, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            handle.interrupt_flags(),
            INT_RES_RDY | INT_OVERRUN | INT_WINMON
        );
        adc.write(0x18, AccessWidth::Byte, INT_OVERRUN as u64, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.interrupt_flags(), INT_RES_RDY | INT_WINMON);
    }

    #[test]
    fn input_scan_advances_the_positive_mux_offset_and_wraps() {
        let (mut adc, handle) = Samd21Adc::new("adc");
        handle.inject_sample(4, 0x0444).unwrap();
        handle.inject_sample(5, 0x0555).unwrap();
        adc.write(0x00, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        // MUXPOS=4, INPUTSCAN=1 selects channels 4 and 5 in turn.
        adc.write(0x10, AccessWidth::Word, 4 | (1_u64 << 16), SimTime::ZERO)
            .unwrap();
        adc.write(0x0c, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.result(), 0x0444);
        adc.read(0x1a, AccessWidth::HalfWord, SimTime::ZERO)
            .unwrap();
        adc.write(0x0c, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.result(), 0x0555);
        adc.read(0x1a, AccessWidth::HalfWord, SimTime::ZERO)
            .unwrap();
        adc.write(0x0c, AccessWidth::Byte, 2, SimTime::ZERO)
            .unwrap();
        assert_eq!(handle.result(), 0x0444);
    }
}
