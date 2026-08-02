use super::*;

const REGISTER_BYTES: usize = 0x100;
const CAM_CTRL: usize = 0x04;
const CAM_CTRL1: usize = 0x08;
const LCD_USER: usize = 0x14;
const LCD_MISC: usize = 0x18;
const LCD_CMD_VAL: usize = 0x28;
const INT_ENA: usize = 0x64;
const INT_RAW: usize = 0x68;
const INT_ST: usize = 0x6c;
const INT_CLR: usize = 0x70;
const DATE: usize = 0xfc;

const LCD_START: u32 = 1 << 27;
const CAM_START: u32 = 1 << 29;
const CAM_LINE_INT_EN: u32 = 1 << 7;
const INT_MASK: u32 = 0x0f;
const LCD_TRANS_DONE_INT: u32 = 1 << 1;
const CAM_VSYNC_INT: u32 = 1 << 2;
const CAM_HS_INT: u32 = 1 << 3;

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
    fn register(&self, offset: usize) -> u32 {
        self.registers[offset / 4]
    }

    fn set_register(&mut self, offset: usize, value: u32) {
        self.registers[offset / 4] = value;
    }

    fn refresh_interrupt_status(&mut self) {
        self.registers[INT_ST / 4] = self.register(INT_RAW) & self.register(INT_ENA);
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

    fn start_lcd(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let mut emitted = Vec::new();
        // LCD_CMD_VAL is the native command latch. Treat it as the first
        // transmitted word when LCD_CMD is enabled, then consume the bounded
        // host DMA queue as the data phase.
        if self.register(LCD_USER) & (1 << 26) != 0 {
            emitted.push(self.register(LCD_CMD_VAL));
        }
        emitted.extend(self.lcd_input.drain(..));
        if let Some(last) = emitted.last().copied() {
            self.lcd_output.extend(emitted);
            self.publish(0, last, at)?;
        }
        self.registers[INT_RAW / 4] |= LCD_TRANS_DONE_INT;
        self.refresh_interrupt_status();
        Ok(())
    }

    fn start_camera(&mut self, at: SimTime) -> Result<(), DeviceError> {
        let captured: Vec<_> = self.camera_input.drain(..).collect();
        if let Some(last) = captured.last().copied() {
            self.camera_output.extend(captured);
            self.publish(1, last, at)?;
        }
        self.registers[INT_RAW / 4] |= CAM_VSYNC_INT;
        if self.register(CAM_CTRL) & CAM_LINE_INT_EN != 0 {
            self.registers[INT_RAW / 4] |= CAM_HS_INT;
        }
        self.refresh_interrupt_status();
        Ok(())
    }

    fn reset(&mut self) {
        self.registers.fill(0);
        self.lcd_input.clear();
        self.lcd_output.clear();
        self.camera_input.clear();
        self.camera_output.clear();
        self.registers[LCD_MISC / 4] = 17 << 1;
        self.registers[DATE / 4] = 33_566_752;
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
}

impl Device for Esp32S3LcdCam {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&mut self, offset: u64, width: AccessWidth, _at: SimTime) -> Result<u64, DeviceError> {
        if width != AccessWidth::Word || offset & 3 != 0 {
            return Err(DeviceError::new(
                "ESP32-S3 LCD_CAM requires aligned word access",
            ));
        }
        let offset = usize::try_from(offset).expect("LCD_CAM offset fits usize");
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
                "ESP32-S3 LCD_CAM requires aligned word access",
            ));
        }
        let offset = usize::try_from(offset).expect("LCD_CAM offset fits usize");
        if offset >= REGISTER_BYTES {
            return Err(DeviceError::new(format!(
                "{} write at {offset:#x}",
                self.name
            )));
        }
        let value = u32::try_from(value & u64::from(u32::MAX)).expect("masked value fits u32");
        let mut state = self.state.borrow_mut();
        match offset {
            LCD_USER => {
                state.set_register(LCD_USER, value & !LCD_START);
                if value & LCD_START != 0 {
                    state.start_lcd(at)?;
                }
            }
            CAM_CTRL1 => {
                state.set_register(CAM_CTRL1, value & !CAM_START);
                if value & CAM_START != 0 {
                    state.start_camera(at)?;
                }
            }
            INT_RAW | INT_ST => {}
            INT_ENA => {
                state.set_register(INT_ENA, value & INT_MASK);
                state.refresh_interrupt_status();
            }
            INT_CLR => {
                state.registers[INT_RAW / 4] &= !(value & INT_MASK);
                state.set_register(INT_CLR, 0);
                state.refresh_interrupt_status();
            }
            LCD_MISC if value & (1 << 27) != 0 => {
                state.lcd_input.clear();
                state.lcd_output.clear();
                state.set_register(LCD_MISC, value & !(1 << 27));
            }
            DATE => state.set_register(DATE, value & 0x0fff_ffff),
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
    fn lcd_start_consumes_command_and_dma_words_and_sets_done_interrupt() {
        let hub = SignalHub::new();
        let (mut lcd, handle) = Esp32S3LcdCam::new("lcd-cam", hub).unwrap();
        handle.queue_lcd_words([0x1122_3344, 0x5566_7788]);
        lcd.write(INT_ENA as u64, AccessWidth::Word, 1 << 1, SimTime::ZERO)
            .unwrap();
        lcd.write(
            LCD_CMD_VAL as u64,
            AccessWidth::Word,
            0x00ab_cdef,
            SimTime::ZERO,
        )
        .unwrap();
        lcd.write(
            LCD_USER as u64,
            AccessWidth::Word,
            u64::from((1 << 26) | LCD_START),
            SimTime::from_ticks(2),
        )
        .unwrap();
        assert_eq!(
            handle.take_lcd_words(),
            vec![0x00ab_cdef, 0x1122_3344, 0x5566_7788]
        );
        assert_eq!(
            lcd.read(INT_ST as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            1 << 1
        );
        lcd.write(INT_CLR as u64, AccessWidth::Word, 1 << 1, SimTime::ZERO)
            .unwrap();
        assert_eq!(
            lcd.read(INT_ST as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0
        );
    }

    #[test]
    fn camera_start_captures_host_frame_and_line_interrupt() {
        let hub = SignalHub::new();
        let (mut lcd, handle) = Esp32S3LcdCam::new("lcd-cam", hub).unwrap();
        handle.queue_camera_words([0xdead_beef, 0xcafe_babe]);
        lcd.write(INT_ENA as u64, AccessWidth::Word, 0x0c, SimTime::ZERO)
            .unwrap();
        lcd.write(
            CAM_CTRL as u64,
            AccessWidth::Word,
            CAM_LINE_INT_EN as u64,
            SimTime::ZERO,
        )
        .unwrap();
        lcd.write(
            CAM_CTRL1 as u64,
            AccessWidth::Word,
            CAM_START as u64,
            SimTime::ZERO,
        )
        .unwrap();
        assert_eq!(handle.take_camera_words(), vec![0xdead_beef, 0xcafe_babe]);
        assert_eq!(
            lcd.read(INT_ST as u64, AccessWidth::Word, SimTime::ZERO)
                .unwrap(),
            0x0c
        );
    }
}
