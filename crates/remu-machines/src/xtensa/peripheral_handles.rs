use super::*;

impl XtensaMachine {
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
