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
        match self {
            Self::SarReader1Ctrl => (1 << 29) | (1 << 28) | (0xff << 19) | (1 << 18) | 0xff,
            Self::SarReader1Status => u32::MAX,
            Self::SarMeas1Ctrl1 => 0xff00_0000,
            Self::SarMeas1Ctrl2 => u32::MAX,
            Self::SarReader2Ctrl => {
                (1 << 30) | (1 << 29) | (0xff << 19) | (1 << 18) | (3 << 16) | 0xff
            }
            Self::SarReader2Status => u32::MAX,
            Self::SarMeas2Ctrl1 => u32::MAX,
            Self::SarMeas2Ctrl2 => u32::MAX,
            Self::TsensCtrl => 0x01ff_f1ff,
            Self::TsensCtrl2 => 0x0000_7fff,
            Self::SarCocpuIntRaw | Self::SarCocpuIntStatus => 0x0000_0fff,
            Self::SarCocpuIntEna => 0x0000_0fff,
            Self::SarCocpuIntClear => 0,
            Self::SarPeriClockGate => (1 << 31) | (1 << 30) | (1 << 29) | (1 << 27),
            Self::SarPeriReset => (1 << 30) | (1 << 29) | (1 << 27) | (1 << 25),
            Self::SarDate => 0x0fff_ffff,
        }
    }

    fn write_mask(self) -> u32 {
        match self {
            Self::SarReader1Ctrl => Self::SarReader1Ctrl.read_mask(),
            Self::SarReader1Status => 0,
            Self::SarMeas1Ctrl1 => 0xff00_0000,
            Self::SarMeas1Ctrl2 => {
                (1 << 31) | (0xfff << 19) | SAR_MEAS_START_FORCE | SAR_MEAS_START
            }
            Self::SarReader2Ctrl => Self::SarReader2Ctrl.read_mask(),
            Self::SarReader2Status => 0,
            Self::SarMeas2Ctrl1 => 0xffff_fff8,
            Self::SarMeas2Ctrl2 => {
                (1 << 31) | (0xfff << 19) | SAR_MEAS_START_FORCE | SAR_MEAS_START
            }
            Self::TsensCtrl => 0x01ff_f000,
            Self::TsensCtrl2 => 0x0000_7fff,
            Self::SarCocpuIntRaw | Self::SarCocpuIntStatus => 0,
            Self::SarCocpuIntEna => 0x0000_0fff,
            Self::SarCocpuIntClear => 0x0000_0fff,
            Self::SarPeriClockGate => Self::SarPeriClockGate.read_mask(),
            Self::SarPeriReset => Self::SarPeriReset.read_mask(),
            Self::SarDate => 0x0fff_ffff,
        }
    }

    fn reset_value(self) -> u32 {
        match self {
            Self::SarReader1Ctrl => (1 << 29) | (1 << 18) | 2,
            Self::SarReader1Status
            | Self::SarMeas1Ctrl1
            | Self::SarMeas1Ctrl2
            | Self::SarReader2Status
            | Self::SarMeas2Ctrl2
            | Self::SarCocpuIntRaw
            | Self::SarCocpuIntEna
            | Self::SarCocpuIntStatus
            | Self::SarCocpuIntClear
            | Self::SarPeriClockGate
            | Self::SarPeriReset => 0,
            Self::SarReader2Ctrl => (1 << 30) | (1 << 18) | (1 << 16) | 2,
            Self::SarMeas2Ctrl1 => (7 << 24) | (2 << 16) | (2 << 8),
            Self::TsensCtrl => (6 << 14) | TSENS_INT_ENABLE,
            Self::TsensCtrl2 => (1 << 14) | 2,
            Self::SarDate => 0x0210_1180,
        }
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
}

struct Esp32S3TsensState {
    registers: Vec<u32>,
    raw: u8,
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

    fn refresh_interrupt_status(&mut self) {
        let raw = self.register(Esp32S3TsensRegister::SarCocpuIntRaw);
        let enabled = self.register(Esp32S3TsensRegister::SarCocpuIntEna);
        self.set_register(Esp32S3TsensRegister::SarCocpuIntStatus, raw & enabled);
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
        for register in [
            Esp32S3TsensRegister::SarReader1Ctrl,
            Esp32S3TsensRegister::SarReader1Status,
            Esp32S3TsensRegister::SarMeas1Ctrl1,
            Esp32S3TsensRegister::SarMeas1Ctrl2,
            Esp32S3TsensRegister::SarReader2Ctrl,
            Esp32S3TsensRegister::SarReader2Status,
            Esp32S3TsensRegister::SarMeas2Ctrl1,
            Esp32S3TsensRegister::SarMeas2Ctrl2,
            Esp32S3TsensRegister::TsensCtrl,
            Esp32S3TsensRegister::TsensCtrl2,
            Esp32S3TsensRegister::SarCocpuIntRaw,
            Esp32S3TsensRegister::SarCocpuIntEna,
            Esp32S3TsensRegister::SarCocpuIntStatus,
            Esp32S3TsensRegister::SarCocpuIntClear,
            Esp32S3TsensRegister::SarPeriClockGate,
            Esp32S3TsensRegister::SarPeriReset,
            Esp32S3TsensRegister::SarDate,
        ] {
            self.set_register(register, register.reset_value());
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
        let signal = hub.declare(
            "board.esp32s3.tsens.temperature",
            SignalValue::from_u64(0, 8)?,
            Some("ESP32-S3 TSENS raw temperature code".to_owned()),
        )?;
        let state = Rc::new(RefCell::new(Esp32S3TsensState {
            registers: vec![0; REGISTER_BYTES / 4],
            raw: 128,
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
        let register = Esp32S3TsensRegister::from_offset(offset)
            .ok_or_else(|| self.unsupported("read", offset))?;
        Ok(u64::from(
            self.state.borrow().register(register) & register.read_mask(),
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
        let register = Esp32S3TsensRegister::from_offset(offset)
            .ok_or_else(|| self.unsupported("write", offset))?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 SENS word write exceeds 32 bits"))?;
        let mut state = self.state.borrow_mut();
        match register {
            Esp32S3TsensRegister::TsensCtrl => {
                let requested = value & register.write_mask();
                // The output and ready fields are read-only on native
                // hardware.  A configuration write must not erase the last
                // completed measurement while changing the writable bits.
                let preserved = state.register(register) & !register.write_mask();
                state.set_register(register, preserved | requested);
                if requested & (TSENS_POWER_UP | TSENS_POWER_UP_FORCE) != 0 {
                    state.complete_measurement(at)?;
                }
            }
            Esp32S3TsensRegister::SarCocpuIntRaw
            | Esp32S3TsensRegister::SarCocpuIntStatus
            | Esp32S3TsensRegister::SarReader1Status
            | Esp32S3TsensRegister::SarReader2Status
            | Esp32S3TsensRegister::SarCocpuIntClear => {
                if register == Esp32S3TsensRegister::SarCocpuIntClear {
                    let raw = state.register(Esp32S3TsensRegister::SarCocpuIntRaw)
                        & !(value & register.write_mask());
                    state.set_register(Esp32S3TsensRegister::SarCocpuIntRaw, raw);
                    state.refresh_interrupt_status();
                }
            }
            Esp32S3TsensRegister::SarCocpuIntEna => {
                state.set_register(register, value & register.write_mask());
                state.refresh_interrupt_status();
            }
            Esp32S3TsensRegister::SarMeas1Ctrl2 | Esp32S3TsensRegister::SarMeas2Ctrl2 => {
                state.complete_sar_measurement(register, value);
            }
            _ => state.set_register(register, value & register.write_mask()),
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
    fn register_enum_matches_native_offsets_and_rejects_unmodeled_accesses() {
        assert_eq!(Esp32S3TsensRegister::TsensCtrl.offset(), 0x50);
        assert_eq!(Esp32S3TsensRegister::SarDate.offset(), 0x1fc);
        let hub = SignalHub::new();
        let (mut tsens, _) = Esp32S3Tsens::new("tsens", hub).unwrap();
        assert!(tsens.read(0x100, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(
            tsens
                .write(0x100, AccessWidth::Word, 0, SimTime::ZERO)
                .is_err()
        );
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
}
