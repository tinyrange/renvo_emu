//! Firmware-visible `M5StickS3` board attachment and stable state artifacts.

use remu_bus::DeviceError;
use remu_core::SimTime;
use remu_devices::{
    Bmi270Snapshot, Es8311Snapshot, Esp32s3I2cHandle, Esp32s3I2sHandle, Esp32s3RmtHandle,
    Esp32s3RmtTransfer, Esp32s3SpiHandle, Esp32s3St7789Snapshot, GpioHandle, M5Pm1Snapshot,
};
use remu_signals::Logic;
use serde::Serialize;

/// One of the two active-low buttons wired directly to the ESP32-S3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum M5StickS3Button {
    /// Front Button A on GPIO11.
    A,
    /// Side Button B on GPIO12.
    B,
}

impl M5StickS3Button {
    /// Returns the published ESP32-S3 GPIO number.
    pub const fn pin(self) -> u8 {
        match self {
            Self::A => 11,
            Self::B => 12,
        }
    }
}

/// Stable state of every modeled `M5StickS3` board component.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct M5StickS3Snapshot {
    /// ST7789 controller, framebuffer, reset, and backlight state.
    pub display: Esp32s3St7789Snapshot,
    /// Whether the LCD controller, backlight, and shared L3B rail are all active.
    pub display_active: bool,
    /// M5PM1 power-management companion state.
    pub power: M5Pm1Snapshot,
    /// BMI270 inertial sensor state and deterministic sample.
    pub imu: Bmi270Snapshot,
    /// ES8311 control and I2S microphone/speaker data-plane evidence.
    pub audio: M5StickS3AudioSnapshot,
    /// Infrared transmitter waveform and receiver input state.
    pub infrared: M5StickS3InfraredSnapshot,
    /// Grove and Hat2 expansion power/pin state.
    pub expansion: M5StickS3ExpansionSnapshot,
    /// Whether active-low Button A is currently pressed.
    pub button_a_pressed: bool,
    /// Whether active-low Button B is currently pressed.
    pub button_b_pressed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct M5StickS3AudioSnapshot {
    pub codec: Es8311Snapshot,
    pub power_domain_enabled: bool,
    pub speaker_amp_enabled: bool,
    pub speaker_frames: u64,
    pub speaker_last_sample: u32,
    pub microphone_frames: u64,
    pub microphone_last_sample: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct M5StickS3InfraredSnapshot {
    pub transmitter: Esp32s3RmtTransfer,
    pub receiver: Esp32s3RmtTransfer,
    pub receiver_high: bool,
    pub powered: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct M5StickS3ExpansionSnapshot {
    pub grove_powered: bool,
    pub grove_sda_high: bool,
    pub grove_scl_high: bool,
    /// Published Hat2 GPIO pins in connector order: 5,4,6,1,7,8,43,44,2,3.
    pub hat2_high: [bool; 10],
}

/// Live connection between one ESP32-S3 machine and its `M5StickS3` components.
#[derive(Clone)]
pub struct M5StickS3Handle {
    spi3: Esp32s3SpiHandle,
    i2c1: Esp32s3I2cHandle,
    i2s0: Esp32s3I2sHandle,
    i2s1: Esp32s3I2sHandle,
    rmt: Esp32s3RmtHandle,
    gpio: GpioHandle,
}

impl M5StickS3Handle {
    pub(crate) fn new(
        spi3: Esp32s3SpiHandle,
        i2c1: Esp32s3I2cHandle,
        speaker_i2s: Esp32s3I2sHandle,
        microphone_i2s: Esp32s3I2sHandle,
        rmt: Esp32s3RmtHandle,
        gpio: GpioHandle,
    ) -> Self {
        Self {
            spi3,
            i2c1,
            i2s0: speaker_i2s,
            i2s1: microphone_i2s,
            rmt,
            gpio,
        }
    }

    /// Supplies the next sample returned by the internal microphone I2S input.
    pub fn set_microphone_sample(&self, sample: u32) {
        self.i2s1.set_input_sample(sample);
    }

    /// Supplies one deterministic BMI270 physical sample.
    pub fn set_imu_sample(
        &self,
        accel: [i16; 3],
        gyro: [i16; 3],
        temperature: i16,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        self.i2c1.set_bmi270_sample(accel, gyro, temperature, at)
    }

    /// Drives the active-high raw IR receiver input on GPIO42.
    pub fn set_ir_receiver(&self, high: bool, at: SimTime) -> Result<(), DeviceError> {
        if high {
            self.rmt.inject_receive(
                0,
                vec![
                    rmt_item(9_000, true, 4_500, false),
                    rmt_item(560, true, 560, false),
                ],
            );
        }
        self.gpio
            .set_input(42, if high { Logic::One } else { Logic::Zero }, at)
    }

    /// Drives or releases one physical active-low button.
    pub fn set_button(
        &self,
        button: M5StickS3Button,
        pressed: bool,
        at: SimTime,
    ) -> Result<(), DeviceError> {
        self.gpio.set_input(
            button.pin(),
            if pressed { Logic::Zero } else { Logic::One },
            at,
        )
    }

    /// Captures the complete deterministic board-component state.
    pub fn snapshot(&self) -> Result<M5StickS3Snapshot, DeviceError> {
        let display = self
            .spi3
            .st7789_snapshot()
            .ok_or_else(|| DeviceError::new("M5StickS3 ST7789 is not attached"))?;
        let power = self
            .i2c1
            .m5pm1_snapshot()
            .ok_or_else(|| DeviceError::new("M5StickS3 M5PM1 is not attached"))?;
        let imu = self
            .i2c1
            .bmi270_snapshot()
            .ok_or_else(|| DeviceError::new("M5StickS3 BMI270 is not attached"))?;
        let codec = self
            .i2c1
            .es8311_snapshot()
            .ok_or_else(|| DeviceError::new("M5StickS3 ES8311 is not attached"))?;
        let speaker = self.i2s0.transfer();
        let microphone = self.i2s1.transfer();
        let infrared_tx = self.rmt.transfer(0).unwrap_or_default();
        let infrared_rx = self.rmt.receiver(0).unwrap_or_default();
        let resolved_high = |pin| self.gpio.resolved(pin).map(|value| value == Logic::One);
        let mut hat2_high = [false; 10];
        for (index, pin) in [5, 4, 6, 1, 7, 8, 43, 44, 2, 3].into_iter().enumerate() {
            hat2_high[index] = resolved_high(pin)?;
        }
        Ok(M5StickS3Snapshot {
            display_active: display.panel.display_on
                && display.backlight_on
                && !display.reset_asserted
                && power.l3b_powered,
            display,
            audio: M5StickS3AudioSnapshot {
                codec,
                power_domain_enabled: power.l3b_powered,
                speaker_amp_enabled: power.speaker_amp_enabled,
                speaker_frames: speaker.tx_frames,
                speaker_last_sample: speaker.last_tx,
                microphone_frames: microphone.rx_frames,
                microphone_last_sample: microphone.last_rx,
            },
            infrared: M5StickS3InfraredSnapshot {
                transmitter: infrared_tx,
                receiver: infrared_rx,
                receiver_high: resolved_high(42)?,
                powered: power.power_config & 0x04 != 0,
            },
            expansion: M5StickS3ExpansionSnapshot {
                grove_powered: power.power_config & 0x08 != 0,
                grove_sda_high: resolved_high(9)?,
                grove_scl_high: resolved_high(10)?,
                hat2_high,
            },
            power,
            imu,
            button_a_pressed: self.gpio.resolved(M5StickS3Button::A.pin())? == Logic::Zero,
            button_b_pressed: self.gpio.resolved(M5StickS3Button::B.pin())? == Logic::Zero,
        })
    }
}

const fn rmt_item(duration0: u32, level0: bool, duration1: u32, level1: bool) -> u32 {
    duration0 | ((level0 as u32) << 15) | (duration1 << 16) | ((level1 as u32) << 31)
}
