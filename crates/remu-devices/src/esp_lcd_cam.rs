use super::*;

const REGISTER_BYTES: usize = 0x100;

const LCD_CMD: u32 = 1 << 26;
const LCD_START: u32 = 1 << 27;
const LCD_RESET: u32 = 1 << 28;
const LCD_UPDATE: u32 = 1 << 20;
const LCD_AFIFO_RESET: u32 = 1 << 27;
const CAM_LINE_INT_ENABLE: u32 = 1 << 7;
const CAM_UPDATE: u32 = 1 << 4;
const CAM_START: u32 = 1 << 29;
const CAM_RESET: u32 = 1 << 30;
const CAM_AFIFO_RESET: u32 = 1 << 31;
const INTERRUPT_MASK: u32 = 0x0f;
const LCD_TRANS_DONE_INTERRUPT: u32 = 1 << 1;
const CAMERA_VSYNC_INTERRUPT: u32 = 1 << 2;
const CAMERA_HS_INTERRUPT: u32 = 1 << 3;

/// Native register identifiers for the functional ESP32-S3 LCD_CAM aperture.
///
/// The enum deliberately contains the documented register surface used by
/// the functional model. Reserved offsets are rejected instead of silently
/// turning the peripheral into an unstructured RAM window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum Esp32S3LcdCamRegister {
    /// LCD clock configuration (`LCD_CAM_LCD_CLOCK_REG`).
    LcdClock = 0x00,
    /// Camera clock/control configuration (`LCD_CAM_CAM_CTRL_REG`).
    CamCtrl = 0x04,
    /// Camera capture configuration and start strobes (`LCD_CAM_CAM_CTRL1_REG`).
    CamCtrl1 = 0x08,
    /// Camera RGB/YUV conversion configuration (`LCD_CAM_CAM_RGB_YUV_REG`).
    CamRgbYuv = 0x0c,
    /// LCD RGB/YUV conversion configuration (`LCD_CAM_LCD_RGB_YUV_REG`).
    LcdRgbYuv = 0x10,
    /// LCD sequence configuration and start strobes (`LCD_CAM_LCD_USER_REG`).
    LcdUser = 0x14,
    /// LCD asynchronous FIFO and CD configuration (`LCD_CAM_LCD_MISC_REG`).
    LcdMisc = 0x18,
    /// LCD frame timing configuration (`LCD_CAM_LCD_CTRL_REG`).
    LcdCtrl = 0x1c,
    /// LCD horizontal/vertical width configuration (`LCD_CAM_LCD_CTRL1_REG`).
    LcdCtrl1 = 0x20,
    /// LCD sync pulse configuration (`LCD_CAM_LCD_CTRL2_REG`).
    LcdCtrl2 = 0x24,
    /// LCD command value latch (`LCD_CAM_LCD_CMD_VAL_REG`).
    LcdCmdVal = 0x28,
    /// LCD signal delay configuration (`LCD_CAM_LCD_DLY_MODE_REG`).
    LcdDlyMode = 0x30,
    /// LCD data delay configuration (`LCD_CAM_LCD_DATA_DOUT_MODE_REG`).
    LcdDataDoutMode = 0x38,
    /// LCD/CAM DMA interrupt enables (`LCD_CAM_LC_DMA_INT_ENA_REG`).
    LcDmaIntEna = 0x64,
    /// LCD/CAM DMA raw interrupts (`LCD_CAM_LC_DMA_INT_RAW_REG`).
    LcDmaIntRaw = 0x68,
    /// LCD/CAM DMA masked interrupt status (`LCD_CAM_LC_DMA_INT_ST_REG`).
    LcDmaIntStatus = 0x6c,
    /// LCD/CAM DMA interrupt clear strobes (`LCD_CAM_LC_DMA_INT_CLR_REG`).
    LcDmaIntClear = 0x70,
    /// LCD/CAM version/date register (`LCD_CAM_LC_REG_DATE_REG`).
    LcDate = 0xfc,
}

impl Esp32S3LcdCamRegister {
    /// Returns the native register offset within the LCD_CAM window.
    pub const fn offset(self) -> u64 {
        self as u64
    }

    fn from_offset(offset: u64) -> Option<Self> {
        Some(match offset {
            0x00 => Self::LcdClock,
            0x04 => Self::CamCtrl,
            0x08 => Self::CamCtrl1,
            0x0c => Self::CamRgbYuv,
            0x10 => Self::LcdRgbYuv,
            0x14 => Self::LcdUser,
            0x18 => Self::LcdMisc,
            0x1c => Self::LcdCtrl,
            0x20 => Self::LcdCtrl1,
            0x24 => Self::LcdCtrl2,
            0x28 => Self::LcdCmdVal,
            0x30 => Self::LcdDlyMode,
            0x38 => Self::LcdDataDoutMode,
            0x64 => Self::LcDmaIntEna,
            0x68 => Self::LcDmaIntRaw,
            0x6c => Self::LcDmaIntStatus,
            0x70 => Self::LcDmaIntClear,
            0xfc => Self::LcDate,
            _ => return None,
        })
    }

    fn read_mask(self) -> u32 {
        match self {
            Self::LcdClock
            | Self::LcdCtrl
            | Self::LcdCtrl1
            | Self::LcdCmdVal
            | Self::LcdDataDoutMode => u32::MAX,
            Self::CamCtrl => 0x7fff_ffff,
            Self::CamCtrl1 => 0x3fff_ffff,
            Self::CamRgbYuv => 0xffe0_0000,
            Self::LcdRgbYuv => 0xfff0_0000,
            Self::LcdUser => 0xeff8_3fff,
            Self::LcdMisc => 0xf7ff_fffe,
            Self::LcdCtrl2 => 0xffff_03ff,
            Self::LcdDlyMode => 0xff,
            Self::LcDmaIntEna | Self::LcDmaIntRaw | Self::LcDmaIntStatus => INTERRUPT_MASK,
            Self::LcDmaIntClear => 0,
            Self::LcDate => 0x0fff_ffff,
        }
    }

    fn write_mask(self) -> u32 {
        match self {
            Self::LcdClock
            | Self::LcdCtrl
            | Self::LcdCtrl1
            | Self::LcdCmdVal
            | Self::LcdDataDoutMode => u32::MAX,
            Self::CamCtrl => Self::CamCtrl.read_mask(),
            Self::CamCtrl1 => u32::MAX,
            Self::CamRgbYuv => Self::CamRgbYuv.read_mask(),
            Self::LcdRgbYuv => Self::LcdRgbYuv.read_mask(),
            Self::LcdUser => 0xfff8_3fff,
            Self::LcdMisc => 0xffff_fffe,
            Self::LcdCtrl2 => Self::LcdCtrl2.read_mask(),
            Self::LcdDlyMode => Self::LcdDlyMode.read_mask(),
            Self::LcDmaIntEna => INTERRUPT_MASK,
            Self::LcDmaIntRaw | Self::LcDmaIntStatus => 0,
            Self::LcDmaIntClear => INTERRUPT_MASK,
            Self::LcDate => 0x0fff_ffff,
        }
    }

    fn reset_value(self) -> u32 {
        match self {
            Self::LcdMisc => 17 << 1,
            Self::LcDate => 33_566_752,
            Self::LcdClock
            | Self::CamCtrl
            | Self::CamCtrl1
            | Self::CamRgbYuv
            | Self::LcdRgbYuv
            | Self::LcdUser
            | Self::LcdCtrl
            | Self::LcdCtrl1
            | Self::LcdCtrl2
            | Self::LcdCmdVal
            | Self::LcdDlyMode
            | Self::LcdDataDoutMode
            | Self::LcDmaIntEna
            | Self::LcDmaIntRaw
            | Self::LcDmaIntStatus
            | Self::LcDmaIntClear => 0,
        }
    }
}

/// Host-side frame/DMA endpoint for the ESP32-S3 LCD/camera controller.
#[derive(Clone)]
pub struct Esp32S3LcdCamHandle {
    state: Rc<RefCell<Esp32S3LcdCamState>>,
}

impl Esp32S3LcdCamHandle {
    /// Queues words that the functional LCD transmitter consumes on start.
    pub fn queue_lcd_words(&self, words: impl IntoIterator<Item = u32>) {
        self.state.borrow_mut().lcd_input.extend(words);
    }

    /// Drains words emitted by the LCD transmitter.
    pub fn take_lcd_words(&self) -> Vec<u32> {
        self.state.borrow_mut().lcd_output.drain(..).collect()
    }

    /// Queues words representing one host camera frame.
    pub fn queue_camera_words(&self, words: impl IntoIterator<Item = u32>) {
        self.state.borrow_mut().camera_input.extend(words);
    }

    /// Drains words captured by the camera receiver.
    pub fn take_camera_words(&self) -> Vec<u32> {
        self.state.borrow_mut().camera_output.drain(..).collect()
    }
}

struct Esp32S3LcdCamState {
    registers: Vec<u32>,
    lcd_input: VecDeque<u32>,
    lcd_output: VecDeque<u32>,
    camera_input: VecDeque<u32>,
    camera_output: VecDeque<u32>,
    hub: SignalHub,
    signals: [SignalId; 2],
}

impl Esp32S3LcdCamState {
    fn register(&self, register: Esp32S3LcdCamRegister) -> u32 {
        self.registers[register as usize / 4]
    }

    fn set_register(&mut self, register: Esp32S3LcdCamRegister, value: u32) {
        self.registers[register as usize / 4] = value & register.read_mask();
    }

    fn write_register(&mut self, register: Esp32S3LcdCamRegister, value: u32) {
        let writable = register.write_mask();
        self.set_register(
            register,
            (self.register(register) & !writable) | (value & writable),
        );
    }

    fn refresh_interrupt_status(&mut self) {
        let raw = self.register(Esp32S3LcdCamRegister::LcDmaIntRaw);
        let enabled = self.register(Esp32S3LcdCamRegister::LcDmaIntEna);
        self.set_register(Esp32S3LcdCamRegister::LcDmaIntStatus, raw & enabled);
    }

    fn publish(&self, signal: usize, value: u32, at: SimTime) -> Result<(), DeviceError> {
        self.hub
            .set(
                self.signals[signal],
                SignalValue::from_u64(u64::from(value), 32)
                    .expect("fixed LCD/CAM signal width is valid"),
                at,
            )
            .map_err(|error| DeviceError::new(error.to_string()))
    }

    fn reset_lcd_fifo(&mut self) {
        self.lcd_input.clear();
        self.lcd_output.clear();
    }

    fn reset_camera_fifo(&mut self) {
        self.camera_input.clear();
        self.camera_output.clear();
    }

    fn start_lcd(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let mut emitted = Vec::new();
        // LCD_CMD_VAL is the native command latch. Treat it as the first
        // transmitted word when LCD_CMD is enabled, then consume the bounded
        // host DMA queue as the data phase.
        if self.register(Esp32S3LcdCamRegister::LcdUser) & LCD_CMD != 0 {
            emitted.push(self.register(Esp32S3LcdCamRegister::LcdCmdVal));
        }
        emitted.extend(self.lcd_input.drain(..));
        if let Some(last) = emitted.last().copied() {
            self.lcd_output.extend(emitted);
            self.publish(0, last, at)?;
        }
        let raw = self.register(Esp32S3LcdCamRegister::LcDmaIntRaw) | LCD_TRANS_DONE_INTERRUPT;
        self.set_register(Esp32S3LcdCamRegister::LcDmaIntRaw, raw);
        self.refresh_interrupt_status();
        Ok(())
    }

    fn start_camera(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let captured: Vec<_> = self.camera_input.drain(..).collect();
        if let Some(last) = captured.last().copied() {
            self.camera_output.extend(captured);
            self.publish(1, last, at)?;
        }
        let mut raw = self.register(Esp32S3LcdCamRegister::LcDmaIntRaw) | CAMERA_VSYNC_INTERRUPT;
        if self.register(Esp32S3LcdCamRegister::CamCtrl) & CAM_LINE_INT_ENABLE != 0 {
            raw |= CAMERA_HS_INTERRUPT;
        }
        self.set_register(Esp32S3LcdCamRegister::LcDmaIntRaw, raw);
        self.refresh_interrupt_status();
        Ok(())
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.reset_lcd_fifo();
        self.reset_camera_fifo();
        for register in [
            Esp32S3LcdCamRegister::LcdClock,
            Esp32S3LcdCamRegister::CamCtrl,
            Esp32S3LcdCamRegister::CamCtrl1,
            Esp32S3LcdCamRegister::CamRgbYuv,
            Esp32S3LcdCamRegister::LcdRgbYuv,
            Esp32S3LcdCamRegister::LcdUser,
            Esp32S3LcdCamRegister::LcdMisc,
            Esp32S3LcdCamRegister::LcdCtrl,
            Esp32S3LcdCamRegister::LcdCtrl1,
            Esp32S3LcdCamRegister::LcdCtrl2,
            Esp32S3LcdCamRegister::LcdCmdVal,
            Esp32S3LcdCamRegister::LcdDlyMode,
            Esp32S3LcdCamRegister::LcdDataDoutMode,
            Esp32S3LcdCamRegister::LcDmaIntEna,
            Esp32S3LcdCamRegister::LcDmaIntRaw,
            Esp32S3LcdCamRegister::LcDmaIntStatus,
            Esp32S3LcdCamRegister::LcDmaIntClear,
            Esp32S3LcdCamRegister::LcDate,
        ] {
            self.set_register(register, register.reset_value());
        }
    }
}

/// Functional ESP32-S3 LCD_CAM controller.
///
/// This model follows Espressif's native `lcd_cam_reg.h` register layout. LCD
/// and camera starts complete synchronously and use host-provided bounded DMA
/// word queues, which makes command/data and frame-handling firmware testable
/// without claiming panel, camera-electrical, GDMA descriptor, or pixel-clock
/// fidelity.
pub struct Esp32S3LcdCam {
    name: String,
    state: Rc<RefCell<Esp32S3LcdCamState>>,
}

impl Esp32S3LcdCam {
    /// Creates the LCD_CAM page and its host DMA endpoint.
    pub fn new(
        name: impl Into<String>,
        hub: SignalHub,
    ) -> Result<(Self, Esp32S3LcdCamHandle), SignalError> {
        let signals = [
            hub.declare(
                "board.esp32s3.lcd_cam.lcd",
                SignalValue::from_u64(0, 32)?,
                Some("ESP32-S3 LCD last transmitted word".to_owned()),
            )?,
            hub.declare(
                "board.esp32s3.lcd_cam.camera",
                SignalValue::from_u64(0, 32)?,
                Some("ESP32-S3 camera last captured word".to_owned()),
            )?,
        ];
        let state = Rc::new(RefCell::new(Esp32S3LcdCamState {
            registers: vec![0; REGISTER_BYTES / 4],
            lcd_input: VecDeque::new(),
            lcd_output: VecDeque::new(),
            camera_input: VecDeque::new(),
            camera_output: VecDeque::new(),
            hub,
            signals,
        }));
        state.borrow_mut().reset();
        Ok((
            Self {
                name: name.into(),
                state: state.clone(),
            },
            Esp32S3LcdCamHandle { state },
        ))
    }

    fn unsupported(&self, operation: &str, offset: u64) -> DeviceError {
        DeviceError::new(format!(
            "{} {operation} at unsupported ESP32-S3 LCD_CAM offset {offset:#x}",
            self.name
        ))
    }
}

impl Device for Esp32S3LcdCam {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || !width.is_aligned(offset) {
            return Err(DeviceError::new(
                "ESP32-S3 LCD_CAM requires aligned word access",
            ));
        }
        let register = Esp32S3LcdCamRegister::from_offset(offset)
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
                "ESP32-S3 LCD_CAM requires aligned word access",
            ));
        }
        let register = Esp32S3LcdCamRegister::from_offset(offset)
            .ok_or_else(|| self.unsupported("write", offset))?;
        let value = u32::try_from(value)
            .map_err(|_| DeviceError::new("ESP32-S3 LCD_CAM word write exceeds 32 bits"))?;
        let mut state = self.state.borrow_mut();
        match register {
            Esp32S3LcdCamRegister::LcdUser => {
                // LCD_START is a native R/W level.  Only UPDATE and RESET are
                // hardware strobes; preserve the requested start level for
                // firmware that polls or clears it explicitly.
                state.write_register(register, value & !(LCD_UPDATE | LCD_RESET));
                if value & LCD_RESET != 0 {
                    state.reset_lcd_fifo();
                }
                if value & LCD_START != 0 {
                    state.start_lcd(at)?;
                }
            }
            Esp32S3LcdCamRegister::CamCtrl => {
                state.write_register(register, value & !CAM_UPDATE);
            }
            Esp32S3LcdCamRegister::CamCtrl1 => {
                // CAM_START is a native R/W level.  RESET and AFIFO_RESET are
                // write-only strobes and must not be retained.
                state.write_register(register, value & !(CAM_RESET | CAM_AFIFO_RESET));
                if value & (CAM_RESET | CAM_AFIFO_RESET) != 0 {
                    state.reset_camera_fifo();
                }
                if value & CAM_START != 0 {
                    state.start_camera(at)?;
                }
            }
            Esp32S3LcdCamRegister::LcdMisc => {
                state.write_register(register, value & !LCD_AFIFO_RESET);
                if value & LCD_AFIFO_RESET != 0 {
                    state.reset_lcd_fifo();
                }
            }
            Esp32S3LcdCamRegister::LcDmaIntRaw
            | Esp32S3LcdCamRegister::LcDmaIntStatus
            | Esp32S3LcdCamRegister::LcDmaIntClear => {
                if register == Esp32S3LcdCamRegister::LcDmaIntClear {
                    let raw = state.register(Esp32S3LcdCamRegister::LcDmaIntRaw)
                        & !(value & register.write_mask());
                    state.set_register(Esp32S3LcdCamRegister::LcDmaIntRaw, raw);
                    state.refresh_interrupt_status();
                }
            }
            Esp32S3LcdCamRegister::LcDmaIntEna => {
                state.write_register(register, value);
                state.refresh_interrupt_status();
            }
            _ => state.write_register(register, value),
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

    fn read(device: &mut Esp32S3LcdCam, register: Esp32S3LcdCamRegister) -> u64 {
        device
            .read(register.offset(), AccessWidth::Word, SimTime::ZERO)
            .unwrap()
    }

    fn write(
        device: &mut Esp32S3LcdCam,
        register: Esp32S3LcdCamRegister,
        value: u64,
    ) -> Result<(), DeviceError> {
        device.write(register.offset(), AccessWidth::Word, value, SimTime::ZERO)
    }

    #[test]
    fn register_enum_matches_native_offsets_and_reserved_accesses_fail() {
        assert_eq!(Esp32S3LcdCamRegister::LcdClock.offset(), 0);
        assert_eq!(Esp32S3LcdCamRegister::LcDate.offset(), 0xfc);
        let hub = SignalHub::new();
        let (mut lcd, _) = Esp32S3LcdCam::new("lcd-cam", hub).unwrap();
        assert!(lcd.read(0x2c, AccessWidth::Word, SimTime::ZERO).is_err());
        assert!(
            lcd.write(0x2c, AccessWidth::Word, 0, SimTime::ZERO)
                .is_err()
        );
    }

    #[test]
    fn reset_values_and_write_masks_follow_native_header() {
        let hub = SignalHub::new();
        let (mut lcd, _) = Esp32S3LcdCam::new("lcd-cam", hub).unwrap();
        assert_eq!(read(&mut lcd, Esp32S3LcdCamRegister::LcdMisc), 17 << 1);
        assert_eq!(read(&mut lcd, Esp32S3LcdCamRegister::LcDate), 33_566_752);
        write(
            &mut lcd,
            Esp32S3LcdCamRegister::LcdCtrl2,
            u64::from(u32::MAX),
        )
        .unwrap();
        assert_eq!(read(&mut lcd, Esp32S3LcdCamRegister::LcdCtrl2), 0xffff_03ff);
        write(
            &mut lcd,
            Esp32S3LcdCamRegister::LcdUser,
            u64::from(u32::MAX),
        )
        .unwrap();
        assert_eq!(read(&mut lcd, Esp32S3LcdCamRegister::LcdUser), 0xefe8_3fff);
        assert!(
            write(
                &mut lcd,
                Esp32S3LcdCamRegister::LcdUser,
                u64::from(u32::MAX) + 1,
            )
            .is_err()
        );
    }

    #[test]
    fn lcd_and_camera_start_levels_and_fifo_resets_follow_native_semantics() {
        let hub = SignalHub::new();
        let (mut lcd, handle) = Esp32S3LcdCam::new("lcd-cam", hub).unwrap();
        handle.queue_lcd_words([0x11, 0x22]);
        write(
            &mut lcd,
            Esp32S3LcdCamRegister::LcdUser,
            u64::from(LCD_CMD | LCD_START),
        )
        .unwrap();
        assert_eq!(handle.take_lcd_words(), vec![0, 0x11, 0x22]);
        assert_eq!(
            read(&mut lcd, Esp32S3LcdCamRegister::LcdUser) & u64::from(LCD_START),
            u64::from(LCD_START)
        );
        handle.queue_camera_words([0x33]);
        write(
            &mut lcd,
            Esp32S3LcdCamRegister::CamCtrl1,
            u64::from(CAM_START | CAM_RESET),
        )
        .unwrap();
        assert!(handle.take_camera_words().is_empty());
        assert_eq!(
            read(&mut lcd, Esp32S3LcdCamRegister::CamCtrl1) & u64::from(CAM_START),
            u64::from(CAM_START)
        );
    }

    #[test]
    fn raw_interrupts_are_read_only_and_clear_routes_through_status() {
        let hub = SignalHub::new();
        let (mut lcd, handle) = Esp32S3LcdCam::new("lcd-cam", hub).unwrap();
        write(
            &mut lcd,
            Esp32S3LcdCamRegister::LcDmaIntEna,
            u64::from(INTERRUPT_MASK),
        )
        .unwrap();
        handle.queue_lcd_words([0x44]);
        write(
            &mut lcd,
            Esp32S3LcdCamRegister::LcdUser,
            u64::from(LCD_START),
        )
        .unwrap();
        assert_eq!(read(&mut lcd, Esp32S3LcdCamRegister::LcDmaIntRaw), 1 << 1);
        assert_eq!(
            read(&mut lcd, Esp32S3LcdCamRegister::LcDmaIntStatus),
            1 << 1
        );
        write(
            &mut lcd,
            Esp32S3LcdCamRegister::LcDmaIntRaw,
            u64::from(INTERRUPT_MASK),
        )
        .unwrap();
        assert_eq!(read(&mut lcd, Esp32S3LcdCamRegister::LcDmaIntRaw), 1 << 1);
        write(
            &mut lcd,
            Esp32S3LcdCamRegister::LcDmaIntClear,
            u64::from(1u32 << 1),
        )
        .unwrap();
        assert_eq!(read(&mut lcd, Esp32S3LcdCamRegister::LcDmaIntStatus), 0);
    }
}
