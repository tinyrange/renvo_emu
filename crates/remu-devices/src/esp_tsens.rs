use super::*;

const REGISTER_BYTES: usize = 0x200;

const TSENS_RAW_MASK: u32 = 0xff;
const TSENS_READY: u32 = 1 << 8;
const TSENS_INVERT: u32 = 1 << 13;
const TSENS_INT_ENABLE: u32 = 1 << 12;
const TSENS_POWER_UP: u32 = 1 << 22;
const TSENS_POWER_UP_FORCE: u32 = 1 << 23;
const TSENS_INTERRUPT: u32 = 1 << 5;
const SAR_MEAS_START_FORCE: u32 = 1 << 18;
const SAR_MEAS_START: u32 = 1 << 17;
const SAR_MEAS_DONE: u32 = 1 << 16;
const TOUCH_DONE_INTERRUPT: u32 = 1;
const TOUCH_INACTIVE_INTERRUPT: u32 = 1 << 1;
const TOUCH_ACTIVE_INTERRUPT: u32 = 1 << 2;
const TOUCH_SCAN_DONE_INTERRUPT: u32 = 1 << 11;

// Generated from ESP-IDF sens_reg.h at f992ff36f68a783d786d83178e5f85e9a9c76ead.
// Header SHA-256: f3d7fee900f7cdf5d063d1d339e95811f300e35eae6cd96a64d35aacebea2522.
#[derive(Clone, Copy)]
struct RegisterSpec {
    offset: u16,
    reset: u32,
    read_mask: u32,
    write_mask: u32,
}

const fn spec(offset: u16, reset: u32, read_mask: u32, write_mask: u32) -> RegisterSpec {
    RegisterSpec {
        offset,
        reset,
        read_mask,
        write_mask,
    }
}

const SPECS: [RegisterSpec; 71] = [
    spec(0x000, 0x20040002, 0x37fc00ff, 0x37fc00ff),
    spec(0x004, 0, 0xffffffff, 0),
    spec(0x008, 0, 0xff000000, 0xff000000),
    spec(0x00c, 0, 0xffffffff, 0xfffe0000),
    spec(0x010, 0, 0x80000000, 0x80000000),
    spec(0x014, 0xffffffff, 0xffffffff, 0xffffffff),
    spec(0x018, 0x000a000a, 0xffffffff, 0xffffffff),
    spec(0x01c, 0x000a0000, 0xffff007f, 0xffff007f),
    spec(0x020, 0x00fbb87b, 0x0fffffff, 0x0fffffff),
    spec(0x024, 0x40050002, 0x67ff00ff, 0x67ff00ff),
    spec(0x028, 0, 0xffffffff, 0),
    spec(0x02c, 0x07020200, 0xffffffff, 0xfffffff8),
    spec(0x030, 0, 0xffffffff, 0xfffe0000),
    spec(0x034, 0, 0xf0000000, 0xf0000000),
    spec(0x038, 0xffffffff, 0xffffffff, 0xffffffff),
    spec(0x03c, 0, 0xe0000000, 0xe0000000),
    spec(0x040, 0, 0x3fffffff, 0x003fffff),
    spec(0x044, 0, 0x003fffff, 0x003fffff),
    spec(0x048, 0, 0x003fffff, 0x003fffff),
    spec(0x04c, 0, 0x003fffff, 0x003fffff),
    spec(0x050, 0x00019000, 0x01fff1ff, 0x01fff000),
    spec(0x054, 0x00004002, 0x00007fff, 0x00007fff),
    spec(0x058, 0, 0x3fffffff, 0x3fffffff),
    spec(0x05c, 0xfff07fff, 0xffff7fff, 0xfff3ffff),
    spec(0x060, 0, 0x003fffff, 0),
    spec(0x064, 0, 0x003fffff, 0x003fffff),
    spec(0x068, 0, 0x003fffff, 0x003fffff),
    spec(0x06c, 0, 0x003fffff, 0x003fffff),
    spec(0x070, 0, 0x003fffff, 0x003fffff),
    spec(0x074, 0, 0x003fffff, 0x003fffff),
    spec(0x078, 0, 0x003fffff, 0x003fffff),
    spec(0x07c, 0, 0x003fffff, 0x003fffff),
    spec(0x080, 0, 0x003fffff, 0x003fffff),
    spec(0x084, 0, 0x003fffff, 0x003fffff),
    spec(0x088, 0, 0x003fffff, 0x003fffff),
    spec(0x08c, 0, 0x003fffff, 0x003fffff),
    spec(0x090, 0, 0x003fffff, 0x003fffff),
    spec(0x094, 0, 0x003fffff, 0x003fffff),
    spec(0x098, 0, 0x003fffff, 0x003fffff),
    spec(0x09c, 0, 0x80007fff, 0x3fff8000),
    spec(0x0a0, 0, 0x03ffffff, 0),
    spec(0x0a4, 0, 0xe03fffff, 0),
    spec(0x0a8, 0, 0xe03fffff, 0),
    spec(0x0ac, 0, 0xe03fffff, 0),
    spec(0x0b0, 0, 0xe03fffff, 0),
    spec(0x0b4, 0, 0xe03fffff, 0),
    spec(0x0b8, 0, 0xe03fffff, 0),
    spec(0x0bc, 0, 0xe03fffff, 0),
    spec(0x0c0, 0, 0xe03fffff, 0),
    spec(0x0c4, 0, 0xe03fffff, 0),
    spec(0x0c8, 0, 0xe03fffff, 0),
    spec(0x0cc, 0, 0xe03fffff, 0),
    spec(0x0d0, 0, 0xe03fffff, 0),
    spec(0x0d4, 0, 0xe03fffff, 0),
    spec(0x0d8, 0, 0xe03fffff, 0),
    spec(0x0dc, 0, 0xe03fffff, 0),
    spec(0x0e0, 0, 0xffffffff, 0),
    spec(0x0e4, 0, 0x7c000000, 0x02000000),
    spec(0x0e8, 0, 0x00000fff, 0),
    spec(0x0ec, 0, 0x00000fff, 0x00000fff),
    spec(0x0f0, 0, 0x00000fff, 0),
    spec(0x0f4, 0, 0, 0x00000fff),
    spec(0x0f8, 0, 0xffffffff, 0),
    spec(0x0fc, 0xa0000000, 0xf0000000, 0xf0000000),
    spec(0x100, 0, 0xffffffff, 0xffffffff),
    spec(0x104, 0, 0xe8000000, 0xe8000000),
    spec(0x108, 0, 0x6a000000, 0x6a000000),
    spec(0x10c, 0, 0, 0x00000fff),
    spec(0x110, 0, 0, 0x00000fff),
    spec(0x114, 0, 0x0000001f, 0x0000001f),
    spec(0x1fc, 0x02101180, 0x0fffffff, 0x0fffffff),
];

fn spec_for(offset: u64) -> Option<RegisterSpec> {
    let offset = u16::try_from(offset).ok()?;
    SPECS
        .binary_search_by_key(&offset, |spec| spec.offset)
        .ok()
        .map(|index| SPECS[index])
}

/// Native register identifiers for the ESP32-S3 SENS aperture.
///
/// The enum intentionally contains the functional subset implemented by the
/// emulator.  It keeps callers and tests from depending on ad-hoc integer
/// offsets, while unsupported SENS registers fail explicitly instead of
/// silently behaving like RAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Esp32S3TsensRegister {
    /// SAR ADC1 reader configuration (`SENS_SAR_READER1_CTRL_REG`).
    SarReader1Ctrl = 0x00,
    /// SAR ADC1 reader status (`SENS_SAR_READER1_STATUS_REG`).
    SarReader1Status = 0x04,
    /// SAR ADC1 measurement controller 1 (`SENS_SAR_MEAS1_CTRL1_REG`).
    SarMeas1Ctrl1 = 0x08,
    /// SAR ADC1 measurement controller 2 (`SENS_SAR_MEAS1_CTRL2_REG`).
    SarMeas1Ctrl2 = 0x0c,
    /// SAR ADC2 reader configuration (`SENS_SAR_READER2_CTRL_REG`).
    SarReader2Ctrl = 0x24,
    /// SAR ADC2 reader status (`SENS_SAR_READER2_STATUS_REG`).
    SarReader2Status = 0x28,
    /// SAR ADC2 measurement controller 1 (`SENS_SAR_MEAS2_CTRL1_REG`).
    SarMeas2Ctrl1 = 0x2c,
    /// SAR ADC2 measurement controller 2 (`SENS_SAR_MEAS2_CTRL2_REG`).
    SarMeas2Ctrl2 = 0x30,
    /// Temperature-sensor control and raw output (`SENS_SAR_TSENS_CTRL_REG`).
    TsensCtrl = 0x50,
    /// Temperature-sensor power/control 2 (`SENS_SAR_TSENS_CTRL2_REG`).
    TsensCtrl2 = 0x54,
    /// SAR co-processor interrupt raw status (`SENS_SAR_COCPU_INT_RAW_REG`).
    SarCocpuIntRaw = 0xe8,
    /// SAR co-processor interrupt enables (`SENS_SAR_COCPU_INT_ENA_REG`).
    SarCocpuIntEna = 0xec,
    /// SAR co-processor interrupt status (`SENS_SAR_COCPU_INT_ST_REG`).
    SarCocpuIntStatus = 0xf0,
    /// SAR co-processor interrupt clear (`SENS_SAR_COCPU_INT_CLR_REG`).
    SarCocpuIntClear = 0xf4,
    /// SAR/TSENS peripheral clock gate (`SENS_SAR_PERI_CLK_GATE_CONF_REG`).
    SarPeriClockGate = 0x104,
    /// SAR/TSENS peripheral reset (`SENS_SAR_PERI_RESET_CONF_REG`).
    SarPeriReset = 0x108,
    /// SENS block date register (`SENS_SARDATE_REG`).
    SarDate = 0x1fc,
}

impl Esp32S3TsensRegister {
    /// Returns the offset used by the native ESP32-S3 register map.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::SarReader1Ctrl,
            0x04 => Self::SarReader1Status,
            0x08 => Self::SarMeas1Ctrl1,
            0x0c => Self::SarMeas1Ctrl2,
            0x24 => Self::SarReader2Ctrl,
            0x28 => Self::SarReader2Status,
            0x2c => Self::SarMeas2Ctrl1,
            0x30 => Self::SarMeas2Ctrl2,
            0x50 => Self::TsensCtrl,
            0x54 => Self::TsensCtrl2,
            0xe8 => Self::SarCocpuIntRaw,
            0xec => Self::SarCocpuIntEna,
            0xf0 => Self::SarCocpuIntStatus,
            0xf4 => Self::SarCocpuIntClear,
            0x104 => Self::SarPeriClockGate,
            0x108 => Self::SarPeriReset,
            0x1fc => Self::SarDate,
            _ => return None,
        })
    }

    fn read_mask(self) -> u32 {
        spec_for(self.offset())
            .expect("enumerated SENS register has a spec")
            .read_mask
    }

    fn write_mask(self) -> u32 {
        spec_for(self.offset())
            .expect("enumerated SENS register has a spec")
            .write_mask
    }
}

/// Host-side raw-code input and observation handle for the ESP32-S3 TSENS.
#[derive(Clone)]
pub struct Esp32S3TsensHandle {
    state: Rc<RefCell<Esp32S3TsensState>>,
}

impl Esp32S3TsensHandle {
    /// Sets the deterministic eight-bit raw temperature code returned after
    /// the next TSENS power-up/start operation.
    pub fn set_raw(&self, value: u8) {
        self.state.borrow_mut().raw = value;
    }

    /// Returns the raw code supplied by the host-side sensor fixture.
    pub fn raw(&self) -> u8 {
        self.state.borrow().raw
    }

    /// Supplies a deterministic 22-bit capacitive count for touch pad 1..14.
    pub fn set_touch_raw(&self, pad: usize, value: u32) {
        assert!((1..=14).contains(&pad), "ESP32-S3 touch pad must be 1..14");
        self.state.borrow_mut().touch_raw[pad] = value & 0x003f_ffff;
    }

    /// Completes one deterministic touch scan using the configured thresholds.
    pub fn scan_touch(&self) {
        self.state.borrow_mut().complete_touch_scan();
    }

    /// Returns whether any enabled SENS co-processor event is pending.
    pub fn interrupt_pending(&self) -> bool {
        self.state.borrow().register_offset(0xf0) != 0
    }
}

struct Esp32S3TsensState {
    registers: Vec<u32>,
    raw: u8,
    touch_raw: [u32; 15],
    rtc_i2c: Option<Esp32S3RtcI2cHandle>,
    hub: SignalHub,
    signal: SignalId,
}

impl Esp32S3TsensState {
    fn register(&self, register: Esp32S3TsensRegister) -> u32 {
        self.registers[register as usize / 4]
    }

    fn set_register(&mut self, register: Esp32S3TsensRegister, value: u32) {
        self.registers[register as usize / 4] = value & register.read_mask();
    }

    fn register_offset(&self, offset: u64) -> u32 {
        self.registers[offset as usize / 4]
    }

    fn set_register_offset(&mut self, offset: u64, value: u32) {
        let spec = spec_for(offset).expect("internal SENS offset has a register spec");
        self.registers[offset as usize / 4] = value & spec.read_mask;
    }

    fn refresh_interrupt_status(&mut self) {
        let raw = self.register(Esp32S3TsensRegister::SarCocpuIntRaw);
        let enabled = self.register(Esp32S3TsensRegister::SarCocpuIntEna);
        self.set_register(Esp32S3TsensRegister::SarCocpuIntStatus, raw & enabled);
    }

    fn complete_touch_scan(&mut self) {
        let old_active = self.register_offset(0x09c) & 0x7fff;
        let enabled = self.register_offset(0x05c) & 0x7fff;
        let mut active = 0;
        self.set_register_offset(0x0a0, self.touch_raw[0]);
        for pad in 1..=14 {
            let sample = self.touch_raw[pad] & 0x003f_ffff;
            let threshold = self.register_offset(0x060 + pad as u64 * 4) & 0x003f_ffff;
            let touched = threshold != 0 && sample <= threshold && enabled & (1 << pad) != 0;
            if touched {
                active |= 1 << pad;
            }
            self.set_register_offset(
                0x0a0 + pad as u64 * 4,
                sample | if touched { 1 << 29 } else { 0 },
            );
        }
        self.set_register_offset(0x09c, (1 << 31) | active);
        self.set_register_offset(0x05c, self.register_offset(0x05c) | (3 << 18));
        let mut event = TOUCH_DONE_INTERRUPT | TOUCH_SCAN_DONE_INTERRUPT;
        if active & !old_active != 0 {
            event |= TOUCH_ACTIVE_INTERRUPT;
        }
        if old_active & !active != 0 {
            event |= TOUCH_INACTIVE_INTERRUPT;
        }
        self.set_register_offset(0x0e8, self.register_offset(0x0e8) | event);
        self.refresh_interrupt_status();
    }

    fn start_rtc_i2c(&mut self, control: u32) {
        if let Some(rtc_i2c) = &self.rtc_i2c {
            rtc_i2c.execute_from_sens(control);
        }
    }

    fn publish(&self, value: u8, at: SimTime) -> Result<(), DeviceError> {
        self.hub
            .set(
                self.signal,
                SignalValue::from_u64(u64::from(value), 8)
                    .expect("fixed ESP32-S3 TSENS signal width is valid"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn complete_measurement(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let control = self.register(Esp32S3TsensRegister::TsensCtrl);
        let output = if control & TSENS_INVERT != 0 {
            !self.raw
        } else {
            self.raw
        };
        self.set_register(
            Esp32S3TsensRegister::TsensCtrl,
            (control
                & Esp32S3TsensRegister::TsensCtrl.read_mask()
                & !TSENS_RAW_MASK
                & !TSENS_READY)
                | u32::from(output)
                | TSENS_READY,
        );
        if control & TSENS_INT_ENABLE != 0 {
            let raw = self.register(Esp32S3TsensRegister::SarCocpuIntRaw) | TSENS_INTERRUPT;
            self.set_register(Esp32S3TsensRegister::SarCocpuIntRaw, raw);
        }
        self.refresh_interrupt_status();
        self.publish(output, at)
    }

    fn complete_sar_measurement(&mut self, register: Esp32S3TsensRegister, value: u32) {
        let start = SAR_MEAS_START_FORCE | SAR_MEAS_START;
        let done = SAR_MEAS_DONE;
        let writable = register.write_mask();
        let mut next = (value & writable) | (self.register(register) & !writable);
        if value & start != 0 {
            // START_* are software strobes.  The functional model completes
            // immediately and exposes the native read-only DONE latch.
            next &= !start;
            next |= done;
        }
        self.set_register(register, next);
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.raw = 128;
        self.touch_raw.fill(0);
        for spec in SPECS {
            self.registers[spec.offset as usize / 4] = spec.reset & spec.read_mask;
        }
    }
}

/// Functional ESP32-S3 SENS temperature-sensor and SAR-reader block.
///
/// The block follows Espressif's native `sens_reg.h` offsets when mapped at
/// `0x6000_8800`. TSENS power-up completes synchronously at the current
/// abstract time and exposes a deterministic eight-bit raw code. This is a
/// functional driver/firmware baseline, not an electrical or calibrated
/// temperature model; eFuse calibration coefficients are intentionally left
/// to a separate fidelity increment.
pub struct Esp32S3Tsens {
    name: String,
    state: Rc<RefCell<Esp32S3TsensState>>,
}

impl Esp32S3Tsens {
    /// Creates the native SENS page and a host raw-code handle.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Esp32S3TsensHandle), SignalError> {
        Self::new_with_rtc_i2c(name, hub, None)
    }

    /// Creates SENS coupled to the RTC-domain I2C controller used by ULP code.
    pub fn new_with_rtc_i2c(
        name: impl Into<String>,
        hub: SignalHub,
        rtc_i2c: Option<Esp32S3RtcI2cHandle>,
    ) -> Result<(Self, Esp32S3TsensHandle), SignalError> {
        let signal = hub.declare(
            "board.esp32s3.tsens.temperature",
            SignalValue::from_u64(0, 8)?,
            Some("ESP32-S3 TSENS raw temperature code".to_owned()),
        )?;
        let state = Rc::new(RefCell::new(Esp32S3TsensState {
            registers: vec![0; REGISTER_BYTES / 4],
            raw: 128,
            touch_raw: [0; 15],
            rtc_i2c,
            hub,
            signal,
        }));
        state.borrow_mut().reset();
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3TsensHandle { state },
        ))
    }

    fn unsupported(&self, operation: &str, offset: u64) -> DeviceError {
        DeviceError::new(format!(
            "{} {operation} at unsupported ESP32-S3 SENS offset {offset:#x}",
            self.name
        ))
    }
}

impl Device for Esp32S3Tsens {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "ESP32-S3 SENS requires aligned word access",
            ));
        }
        let spec = spec_for(offset).ok_or_else(|| self.unsupported("read", offset))?;
        Ok(u64::from(
            self.state.borrow().register_offset(offset) & spec.read_mask,
        ))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "ESP32-S3 SENS requires aligned word access",
            ));
        }
        let spec = spec_for(offset).ok_or_else(|| self.unsupported("write", offset))?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 SENS word write exceeds 32 bits"))?;
        let mut state = self.state.borrow_mut();
        match offset {
            0x050 => {
                let register = Esp32S3TsensRegister::TsensCtrl;
                let requested = value & spec.write_mask;
                // The output and ready fields are read-only on native
                // hardware.  A configuration write must not erase the last
                // completed measurement while changing the writable bits.
                let preserved = state.register(register) & !register.write_mask();
                state.set_register(register, preserved | requested);
                if requested & (TSENS_POWER_UP | TSENS_POWER_UP_FORCE) != 0 {
                    state.complete_measurement(at)?;
                }
            }
            0x0f4 => {
                let raw = state.register_offset(0x0e8) & !(value & spec.write_mask);
                state.set_register_offset(0x0e8, raw);
                state.refresh_interrupt_status();
            }
            0x0ec => {
                state.set_register_offset(offset, value & spec.write_mask);
                state.refresh_interrupt_status();
            }
            0x10c => {
                let enabled = state.register_offset(0x0ec) | value;
                state.set_register_offset(0x0ec, enabled);
                state.refresh_interrupt_status();
            }
            0x110 => {
                let enabled = state.register_offset(0x0ec) & !value;
                state.set_register_offset(0x0ec, enabled);
                state.refresh_interrupt_status();
            }
            0x00c | 0x030 => {
                let register = Esp32S3TsensRegister::from_offset(offset)
                    .expect("SAR measurement controls are enumerated");
                state.complete_sar_measurement(register, value);
            }
            0x058 => {
                let next =
                    (state.register_offset(offset) & !spec.write_mask) | (value & spec.write_mask);
                state.set_register_offset(offset, next);
                if next & (3 << 28) == (3 << 28) {
                    state.start_rtc_i2c(next);
                }
            }
            0x05c => {
                let mut next =
                    (state.register_offset(offset) & !spec.write_mask) | (value & spec.write_mask);
                if value & (1 << 15) != 0 {
                    let channel_state = state.register_offset(0x09c) & !0x7fff;
                    state.set_register_offset(0x09c, channel_state);
                    next &= !(3 << 18);
                }
                state.set_register_offset(offset, next);
                if next & 0x7fff != 0 {
                    state.complete_touch_scan();
                }
            }
            0x09c => {
                let clear = (value >> 15) & 0x7fff;
                let next = state.register_offset(offset) & !clear;
                state.set_register_offset(offset, next);
            }
            _ => {
                let next =
                    (state.register_offset(offset) & !spec.write_mask) | (value & spec.write_mask);
                state.set_register_offset(offset, next);
            }
        }
        Ok(())
    }

    fn reset(&mut self, _kind: ResetKind) {
        self.state.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(device: &mut Esp32S3Tsens, register: Esp32S3TsensRegister) -> u64 {
        device
            .read(register.offset(), AccessWidth::Word, SimTime::ZERO)
            .unwrap()
    }

    fn write(
        device: &mut Esp32S3Tsens,
        register: Esp32S3TsensRegister,
        value: u64,
    ) -> Result<(), DeviceError> {
        device.write(register.offset(), AccessWidth::Word, value, SimTime::ZERO)
    }

    #[test]
    fn complete_vendor_register_contract_rejects_only_reserved_holes() {
        assert_eq!(Esp32S3TsensRegister::TsensCtrl.offset(), 0x50);
        assert_eq!(Esp32S3TsensRegister::SarDate.offset(), 0x1fc);
        let hub = SignalHub::new();
        let (mut tsens, _) = Esp32S3Tsens::new("tsens", hub).unwrap();
        let mut count = 0;
        for offset in (0..REGISTER_BYTES as u64).step_by(4) {
            if let Some(spec) = spec_for(offset) {
                count += 1;
                assert_eq!(
                    tsens
                        .read(offset, AccessWidth::Word, SimTime::ZERO)
                        .unwrap(),
                    u64::from(spec.reset & spec.read_mask)
                );
            } else {
                assert!(
                    tsens
                        .read(offset, AccessWidth::Word, SimTime::ZERO)
                        .is_err()
                );
            }
        }
        assert_eq!(count, 71);
    }

    #[test]
    fn reset_values_follow_native_header_for_modeled_controls() {
        let hub = SignalHub::new();
        let (mut tsens, _) = Esp32S3Tsens::new("tsens", hub).unwrap();
        assert_eq!(
            read(&mut tsens, Esp32S3TsensRegister::TsensCtrl),
            u64::from((6 << 14) | TSENS_INT_ENABLE)
        );
        assert_eq!(
            read(&mut tsens, Esp32S3TsensRegister::TsensCtrl2),
            u64::from((1u32 << 14) | 2)
        );
        assert_eq!(read(&mut tsens, Esp32S3TsensRegister::SarDate), 0x0210_1180);
    }

    #[test]
    fn power_up_applies_inversion_ready_and_native_interrupt_routing() {
        let hub = SignalHub::new();
        let (mut tsens, handle) = Esp32S3Tsens::new("tsens", hub.clone()).unwrap();
        handle.set_raw(173);
        write(
            &mut tsens,
            Esp32S3TsensRegister::SarCocpuIntEna,
            u64::from(TSENS_INTERRUPT),
        )
        .unwrap();
        write(
            &mut tsens,
            Esp32S3TsensRegister::TsensCtrl,
            u64::from(TSENS_POWER_UP | TSENS_INT_ENABLE | TSENS_INVERT),
        )
        .unwrap();
        let control = read(&mut tsens, Esp32S3TsensRegister::TsensCtrl);
        assert_eq!(control & u64::from(TSENS_RAW_MASK), u64::from(!173u8));
        assert_ne!(control & u64::from(TSENS_READY), 0);
        assert_eq!(
            read(&mut tsens, Esp32S3TsensRegister::SarCocpuIntStatus),
            u64::from(TSENS_INTERRUPT)
        );
        write(
            &mut tsens,
            Esp32S3TsensRegister::TsensCtrl,
            u64::from(TSENS_INT_ENABLE),
        )
        .unwrap();
        let control_after_configuration_write = read(&mut tsens, Esp32S3TsensRegister::TsensCtrl);
        assert_eq!(
            control_after_configuration_write & u64::from(TSENS_RAW_MASK),
            u64::from(!173u8)
        );
        assert_ne!(
            control_after_configuration_write & u64::from(TSENS_READY),
            0
        );
        write(
            &mut tsens,
            Esp32S3TsensRegister::SarCocpuIntClear,
            u64::from(TSENS_INTERRUPT),
        )
        .unwrap();
        assert_eq!(read(&mut tsens, Esp32S3TsensRegister::SarCocpuIntStatus), 0);
        assert!(
            hub.with_registry(|registry| registry.find("board.esp32s3.tsens.temperature"))
                .is_some()
        );
    }

    #[test]
    fn sar_reader_start_strobes_done_without_writing_ro_bits() {
        let hub = SignalHub::new();
        let (mut tsens, _) = Esp32S3Tsens::new("tsens", hub).unwrap();
        write(
            &mut tsens,
            Esp32S3TsensRegister::SarMeas1Ctrl2,
            u64::from(SAR_MEAS_START),
        )
        .unwrap();
        assert_eq!(
            read(&mut tsens, Esp32S3TsensRegister::SarMeas1Ctrl2),
            u64::from(SAR_MEAS_DONE)
        );
        assert!(
            write(
                &mut tsens,
                Esp32S3TsensRegister::SarMeas1Ctrl2,
                u64::from(u32::MAX) << 32,
            )
            .is_err()
        );
    }

    #[test]
    fn touch_scan_updates_samples_active_bitmap_and_interrupt_hierarchy() {
        let hub = SignalHub::new();
        let (mut sens, handle) = Esp32S3Tsens::new("sens", hub).unwrap();
        handle.set_touch_raw(3, 100);
        sens.write(0x06c, AccessWidth::Word, 200, SimTime::ZERO)
            .unwrap();
        sens.write(
            0x0ec,
            AccessWidth::Word,
            u64::from(TOUCH_DONE_INTERRUPT | TOUCH_ACTIVE_INTERRUPT | TOUCH_SCAN_DONE_INTERRUPT),
            SimTime::ZERO,
        )
        .unwrap();
        handle.scan_touch();
        assert_eq!(
            sens.read(0x0ac, AccessWidth::Word, SimTime::ZERO).unwrap(),
            u64::from((1u32 << 29) | 100)
        );
        assert_ne!(
            sens.read(0x09c, AccessWidth::Word, SimTime::ZERO).unwrap() & (1 << 3),
            0
        );
        assert!(handle.interrupt_pending());
        sens.write(0x0f4, AccessWidth::Word, 0xfff, SimTime::ZERO)
            .unwrap();
        assert!(!handle.interrupt_pending());
    }

    #[test]
    fn sens_ulp_i2c_start_drives_rtc_i2c_read_and_write_paths() {
        let hub = SignalHub::new();
        let (mut rtc, rtc_handle) = Esp32S3RtcI2c::new("rtc-i2c");
        let (mut sens, _) =
            Esp32S3Tsens::new_with_rtc_i2c("sens", hub, Some(rtc_handle.clone())).unwrap();
        rtc_handle.set_slave_register(0x42, 7, 0x5a);
        let read_control = (3 << 28) | (7 << 11) | 0x42;
        sens.write(0x058, AccessWidth::Word, read_control, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            rtc.read(0x034, AccessWidth::Word, SimTime::ZERO).unwrap() & 0xff,
            0x5a
        );
        let write_control = (3 << 28) | (1 << 27) | (0xa5 << 19) | (9 << 11) | 0x42;
        sens.write(0x058, AccessWidth::Word, write_control, SimTime::ZERO)
            .unwrap();
        assert_eq!(rtc_handle.slave_register(0x42, 9), 0xa5);
    }
}
