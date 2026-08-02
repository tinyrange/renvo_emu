use super::*;

const REGISTER_BYTES: usize = 0x200;
const SAR_READER1_STATUS: usize = 0x04;
const SAR_MEAS1_CTRL2: usize = 0x0c;
const SAR_READER2_STATUS: usize = 0x28;
const SAR_MEAS2_CTRL2: usize = 0x30;
const TSENS_CTRL: usize = 0x50;
const TSENS_CTRL2: usize = 0x54;
const INT_RAW: usize = 0xe8;
const INT_ENA: usize = 0xec;
const INT_ST: usize = 0xf0;
const INT_CLR: usize = 0xf4;
const DATE: usize = 0x1fc;

const TSENS_RAW_MASK: u32 = 0xff;
const TSENS_READY: u32 = 1 << 8;
const TSENS_INT: u32 = 1 << 5;
const TSENS_POWER_UP: u32 = 1 << 22;
const TSENS_POWER_UP_FORCE: u32 = 1 << 23;

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

    /// Returns the raw code currently exposed by the functional sensor.
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
    fn register(&self, offset: usize) -> u32 {
        self.registers[offset / 4]
    }

    fn set_register(&mut self, offset: usize, value: u32) {
        self.registers[offset / 4] = value;
    }

    fn refresh_interrupt_status(&mut self) {
        self.registers[INT_ST / 4] = self.register(INT_RAW) & self.register(INT_ENA);
    }

    fn publish(&self, at: SimTime) -> Result<(), DeviceError> {
        self.hub
            .set(
                self.signal,
                SignalValue::from_u64(u64::from(self.raw), 8)
                    .expect("fixed ESP32-S3 TSENS signal width is valid"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn complete_measurement(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let value = (self.register(TSENS_CTRL) & !((TSENS_RAW_MASK) | TSENS_READY))
            | u32::from(self.raw)
            | TSENS_READY;
        self.set_register(TSENS_CTRL, value);
        self.registers[INT_RAW / 4] |= TSENS_INT;
        self.refresh_interrupt_status();
        self.publish(at)
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.raw = 128;
        // SENS_TSENS_CTRL reset fields from the ESP32-S3 register header:
        // an enabled interrupt, nominal divider six, and powered-down sensor.
        self.registers[TSENS_CTRL / 4] = (6 << 14) | (1 << 12);
        self.registers[TSENS_CTRL2 / 4] = (1 << 14) | 2;
        self.registers[DATE / 4] = 0x0210_1180;
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
}

impl Device for Esp32S3Tsens {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 SENS requires aligned word access",
            ));
        }
        let offset = usize::try_from(offset).expect("SENS offset fits usize");
        if offset >= REGISTER_BYTES {
            return Err(DeviceError::new(format!(
                "{} read at {offset:#x}",
                self.name
            )));
        }
        Ok(u64::from(self.state.borrow().register(offset)))
    }

    fn write(
        &mut self,
        offset: u64,
        width: AccessWidth,
        value: u64,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 SENS requires aligned word access",
            ));
        }
        let offset = usize::try_from(offset).expect("SENS offset fits usize");
        if offset >= REGISTER_BYTES {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits u32");
        let mut state = self.state.borrow_mut();
        match offset {
            TSENS_CTRL => {
                state.set_register(TSENS_CTRL, value & !TSENS_READY);
                if value & (TSENS_POWER_UP | TSENS_POWER_UP_FORCE) != 0 {
                    state.complete_measurement(at)?;
                } else {
                    let control = state.register(TSENS_CTRL) & !TSENS_READY;
                    state.set_register(TSENS_CTRL, control);
                }
            }
            INT_RAW | INT_ST => {}
            INT_ENA => {
                state.set_register(INT_ENA, value & TSENS_INT);
                state.refresh_interrupt_status();
            }
            INT_CLR => {
                state.registers[INT_RAW / 4] &= !(value & TSENS_INT);
                state.set_register(INT_CLR, 0);
                state.refresh_interrupt_status();
            }
            SAR_READER1_STATUS | SAR_READER2_STATUS => {}
            SAR_MEAS1_CTRL2 | SAR_MEAS2_CTRL2 => {
                let done = if offset == SAR_MEAS1_CTRL2 {
                    1 << 16
                } else {
                    1 << 16
                };
                let start = 1 << 17;
                state.set_register(offset, value & !start);
                if value & start != 0 {
                    state.set_register(offset, (value & !start) | done);
                }
            }
            DATE => state.set_register(DATE, value),
            _ => state.set_register(offset, value),
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

    #[test]
    fn power_up_exposes_raw_code_ready_and_interrupt() {
        let hub = SignalHub::new();
        let (mut tsens, handle) = Esp32S3Tsens::new("tsens", hub.clone()).unwrap();
        handle.set_raw(173);
        tsens
            .write(
                INT_ENA as u64,
                AccessWidth::Word,
                TSENS_INT as u64,
                SimTime::ZERO,
            )
            .unwrap();
        tsens
            .write(
                TSENS_CTRL as u64,
                AccessWidth::Word,
                TSENS_POWER_UP as u64,
                SimTime::from_ticks(3),
            )
            .unwrap();
        let control = tsens
            .read(TSENS_CTRL as u64, AccessWidth::Word, SimTime::ZERO)
            .unwrap();
        assert_eq!(control & u64::from(TSENS_RAW_MASK), 173);
        assert_ne!(control & u64::from(TSENS_READY), 0);
        assert_eq!(
            tsens
                .read(INT_ST as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            u64::from(TSENS_INT)
        );
        assert!(
            hub.with_registry(|registry| registry.find("board.esp32s3.tsens.temperature"))
                .is_some()
        );
        tsens
            .write(
                INT_CLR as u64,
                AccessWidth::Word,
                TSENS_INT as u64,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            tsens
                .read(INT_ST as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
    }

    #[test]
    fn sar_reader_start_sets_the_documented_done_latch() {
        let hub = SignalHub::new();
        let (mut tsens, _) = Esp32S3Tsens::new("tsens", hub).unwrap();
        tsens
            .write(
                SAR_MEAS1_CTRL2 as u64,
                AccessWidth::Word,
                1 << 17,
                SimTime::ZERO,
            )
            .unwrap();
        assert_eq!(
            tsens
                .read(SAR_MEAS1_CTRL2 as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1 << 16
        );
    }
}
