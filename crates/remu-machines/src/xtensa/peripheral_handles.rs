use super::*;

type BoardSerialHandles = (
    Vec<Esp32s3I2cHandle>,
    Vec<Esp32s3SpiHandle>,
    Vec<Esp32s3I2sHandle>,
    Esp32s3RmtHandle,
);

impl XtensaMachine {
    pub(super) fn map_board_serial_peripherals(
        bus: &mut AddressSpace,
        signals: &SignalHub,
    ) -> Result<BoardSerialHandles, XtensaMachineError> {
        let mut i2c = Vec::new();
        for (name, base) in [("i2c0", 0x6001_3000), ("i2c1", 0x6002_7000)] {
            let (device, handle) =
                Esp32s3I2c::new_with_handle(format!("esp32s3.{name}"), signals.clone())?;
            bus.map_device(format!("esp32s3.{name}"), base, 0x1000, Box::new(device))?;
            i2c.push(handle);
        }
        let mut spi = Vec::new();
        for (name, base) in [("spi2", 0x6002_4000), ("spi3", 0x6002_5000)] {
            let (device, handle) =
                Esp32s3Spi::new_with_handle(format!("esp32s3.{name}"), signals.clone())?;
            bus.map_device(format!("esp32s3.{name}"), base, 0x1000, Box::new(device))?;
            spi.push(handle);
        }
        let mut i2s = Vec::new();
        for (name, base) in [("i2s0", 0x6000_f000), ("i2s1", 0x6002_d000)] {
            let (device, handle) =
                Esp32s3I2s::new_with_handle(format!("esp32s3.{name}"), signals.clone())?;
            bus.map_device(format!("esp32s3.{name}"), base, 0x1000, Box::new(device))?;
            i2s.push(handle);
        }
        let (rmt_device, rmt) = Esp32s3Rmt::new_with_handle("esp32s3.rmt", signals.clone())?;
        bus.map_device("esp32s3.rmt", 0x6001_6000, 0x1000, Box::new(rmt_device))?;
        Ok((i2c, spi, i2s, rmt))
    }

    /// Attaches every published non-radio `M5StickS3` board peripheral.
    pub fn attach_m5sticks3(&self) -> Result<crate::M5StickS3Handle, XtensaMachineError> {
        self.spi[1].attach_st7789(
            remu_devices::St7789Config::m5stick_s3(),
            self.chip_gpio.clone(),
            remu_devices::Esp32s3St7789Pins::m5stick_s3(),
        )?;
        self.i2c[1].attach_m5pm1()?;
        self.i2c[1].bind_m5pm1_irq(self.chip_gpio.clone(), 13, self.now)?;
        self.i2c[1].attach_bmi270()?;
        self.i2c[1].attach_es8311()?;
        let handle = crate::M5StickS3Handle::new(
            self.spi[1].clone(),
            self.i2c[1].clone(),
            self.i2s[0].clone(),
            self.i2s[1].clone(),
            self.rmt.clone(),
            self.chip_gpio.clone(),
        );
        handle.set_button(crate::M5StickS3Button::A, false, self.now)?;
        handle.set_button(crate::M5StickS3Button::B, false, self.now)?;
        self.chip_gpio.set_input(9, Logic::One, self.now)?;
        self.chip_gpio.set_input(10, Logic::One, self.now)?;
        Ok(handle)
    }

    /// Drives or releases one GPIO pin.
    pub fn set_pin(&self, pin: u8, value: Logic) -> Result<(), XtensaMachineError> {
        if usize::from(pin) < self.gpio.pin_count() {
            self.gpio.set_input(pin, value, self.now)?;
        }
        self.chip_gpio.set_input(pin, value, self.now)?;
        Ok(())
    }

    /// Applies a host-supplied edge to an ESP32-S3 PCNT unit.
    pub fn pulse_pcnt(
        &self,
        unit: usize,
        edge: remu_devices::EspPcntEdge,
    ) -> Result<bool, XtensaMachineError> {
        Ok(self.pcnt.pulse(unit, edge, self.now)?)
    }

    /// Returns a host-side handle for deterministic TWAI bus injection.
    pub fn twai(&self) -> EspTwaiHandle {
        self.twai.clone()
    }

    /// Returns a host-side handle for configuring and inspecting GDMA channels.
    pub fn gdma(&self) -> EspGdmaHandle {
        self.gdma.clone()
    }

    /// Returns the host-side UHCI0 framed GDMA/UART bridge handle.
    pub fn uhci(&self) -> Esp32S3UhciHandle {
        self.uhci.clone()
    }

    /// Injects a UHCI0 UART frame into the configured GDMA receive channel.
    pub fn queue_uhci_input(&self, frame: &[u8]) -> bool {
        self.uhci.receive_uart_frame(&self.gdma, frame)
    }

    /// Returns a host-side handle for deterministic SAR ADC samples.
    pub fn saradc(&self) -> Esp32S3SarAdcHandle {
        self.saradc.clone()
    }

    /// Returns a host-side handle for deterministic temperature samples.
    pub fn tsens(&self) -> Esp32S3TsensHandle {
        self.tsens.clone()
    }

    /// Returns the LCD/CAM host-side inspection handle.
    pub fn lcd_cam(&self) -> Esp32S3LcdCamHandle {
        self.lcd_cam.clone()
    }

    /// Returns the SD/MMC host-side card handle.
    pub fn sdmmc(&self) -> Esp32S3SdmmcHandle {
        self.sdmmc.clone()
    }

    /// Returns the SHA accelerator host-side inspection handle.
    pub fn sha(&self) -> Esp32S3ShaHandle {
        self.sha.clone()
    }

    /// Returns the AES accelerator host-side inspection handle.
    pub fn aes(&self) -> Esp32S3AesHandle {
        self.aes.clone()
    }

    /// Returns the RTC-control host-side wakeup and interrupt handle.
    pub fn rtc_control(&self) -> EspRtcControlHandle {
        self.rtc_control.clone()
    }

    /// Returns the IO MUX host-side pad configuration handle.
    pub fn io_mux(&self) -> Esp32S3IoMuxHandle {
        self.io_mux.clone()
    }

    /// Returns the SPI-side manual external-memory encryption result handle.
    pub fn xts_aes(&self) -> remu_devices::Esp32S3XtsAesHandle {
        self.xts_aes.clone()
    }
}
