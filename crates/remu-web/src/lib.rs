//! WASI component boundary for running Renvo Emulator in JavaScript hosts.

use remu_core::{RunLimits, SimTime};
use remu_image::{FirmwareArchitecture, FirmwareImage, IntelHexImage, ProgramWordEndianness};
use remu_machines::{
    ArmMachine, ArmMcuMachine, AvrMcuMachine, Mcs51McuMachine, Msp430McuMachine, Pic16McuMachine,
    PinStimulus, RiscVMachine, RunResult, TargetId, XtensaMachine, target_manifests,
    verify_esp_radio_rom,
};
use remu_radio::{RadioProtocol, ReplayArtifact, Spectrum};
use remu_signals::Logic;
use serde::{Deserialize, Serialize};

#[cfg(all(target_arch = "wasm32", target_os = "wasi"))]
mod component;

/// Four-state input value accepted by the portable component API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WebLogic {
    /// Driven digital low.
    Zero,
    /// Driven digital high.
    One,
    /// High impedance or released input.
    HighZ,
    /// Unknown or conflicting input.
    Unknown,
}

impl From<WebLogic> for Logic {
    fn from(value: WebLogic) -> Self {
        match value {
            WebLogic::Zero => Self::Zero,
            WebLogic::One => Self::One,
            WebLogic::HighZ => Self::Z,
            WebLogic::Unknown => Self::X,
        }
    }
}

/// One timestamped GPIO stimulus supplied by JavaScript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebPinStimulus {
    /// Abstract simulation tick at which the value changes.
    pub at: u64,
    /// Zero-based target pin number.
    pub pin: u8,
    /// Four-state value to apply.
    pub value: WebLogic,
}

/// Bounded deterministic execution options shared by native and WASI callers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebRunOptions {
    /// Maximum interpreted instructions or architectural actions.
    pub max_instructions: u64,
    /// Optional inclusive abstract-tick deadline.
    pub deadline_ticks: Option<u64>,
    /// Timestamped GPIO inputs.
    #[serde(default)]
    pub stimuli: Vec<WebPinStimulus>,
}

impl Default for WebRunOptions {
    fn default() -> Self {
        Self {
            max_instructions: 1_000_000,
            deadline_ticks: None,
            stimuli: Vec::new(),
        }
    }
}

impl WebRunOptions {
    fn limits(&self) -> RunLimits {
        RunLimits {
            instructions: Some(self.max_instructions),
            deadline: self.deadline_ticks.map(SimTime::from_ticks),
        }
    }

    fn machine_stimuli(&self) -> Vec<PinStimulus> {
        self.stimuli
            .iter()
            .map(|stimulus| PinStimulus {
                at: SimTime::from_ticks(stimulus.at),
                pin: stimulus.pin,
                value: stimulus.value.into(),
            })
            .collect()
    }
}

/// Radio protocol accepted by the portable JavaScript/WASI API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WebRadioProtocol {
    /// IEEE 802.11 Wi-Fi.
    Wifi,
    /// Bluetooth Low Energy.
    BluetoothLe,
    /// IEEE 802.15.4.
    Ieee802154,
}

impl From<WebRadioProtocol> for RadioProtocol {
    fn from(value: WebRadioProtocol) -> Self {
        match value {
            WebRadioProtocol::Wifi => Self::Wifi,
            WebRadioProtocol::BluetoothLe => Self::BluetoothLe,
            WebRadioProtocol::Ieee802154 => Self::Ieee802154,
        }
    }
}

/// One explicitly scheduled, host-isolated RF frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebRadioFrame {
    /// Simulation timestamp at which transmission begins.
    pub at: u64,
    /// Protocol decoder selected for the frame.
    pub protocol: WebRadioProtocol,
    /// Center frequency in integer kHz.
    pub center_khz: u32,
    /// Occupied bandwidth in integer kHz.
    pub bandwidth_khz: u32,
    /// Stable PHY label such as `wifi-ht20`, `ble-1m`, or `ieee802154-oqpsk-250k`.
    pub phy: String,
    /// Complete protocol PDU/PSDU bytes.
    #[serde(default)]
    pub bytes: Vec<u8>,
    /// Ordered Wi-Fi MPDUs carried by one native A-MPDU transmission.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mpdus: Vec<Vec<u8>>,
    /// Host transmitter power in integer dBm.
    pub power_dbm: i16,
}

/// Radio-aware execution options for the WASI component.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebRadioRunOptions {
    /// Ordinary bounded execution and GPIO settings.
    pub run: WebRunOptions,
    /// Timestamped RF input frames; no host sockets are opened.
    #[serde(default)]
    pub radio_frames: Vec<WebRadioFrame>,
}

/// Stable JSON envelope returned by radio-aware execution.
#[derive(Clone, Debug, Serialize)]
pub struct WebRadioRunOutput {
    /// Ordinary machine execution result.
    pub run: RunResult,
    /// Versioned packet/reception/coexistence replay evidence.
    pub radio: ReplayArtifact,
}

/// Serializes the supported target manifests for the JavaScript API.
pub fn list_targets_json() -> Result<String, String> {
    serde_json::to_string(target_manifests()).map_err(|error| error.to_string())
}

/// Parses an ELF and returns its architecture, segments, symbols, and entry as JSON.
pub fn inspect_elf_json(firmware: &[u8]) -> Result<String, String> {
    let image = FirmwareImage::parse(firmware).map_err(|error| error.to_string())?;
    serde_json::to_string(&image).map_err(|error| error.to_string())
}

/// Runs a little-endian 32-bit ELF and returns the stable [`RunResult`] JSON.
pub fn run_elf_json(
    target: &str,
    firmware: &[u8],
    options: &WebRunOptions,
) -> Result<String, String> {
    let target = target.parse::<TargetId>()?;
    let image = FirmwareImage::parse(firmware).map_err(|error| error.to_string())?;
    let result = run_elf_image(target, &image, options)?;
    serde_json::to_string(&result).map_err(|error| error.to_string())
}

/// Runs C6 or S3 ELF firmware with its required real mask ROM, timestamped RF
/// input, and replay evidence.
pub fn run_radio_elf_json(
    target: &str,
    firmware: &[u8],
    boot_rom: &[u8],
    options: &WebRadioRunOptions,
) -> Result<String, String> {
    let target = target.parse::<TargetId>()?;
    verify_esp_radio_rom(target, boot_rom).map_err(|error| error.to_string())?;
    let image = FirmwareImage::parse(firmware).map_err(|error| error.to_string())?;
    let boot_rom = FirmwareImage::parse_addressed_sections(boot_rom)
        .map_err(|error| format!("invalid {target} mask-ROM ELF: {error}"))?;
    let output = match target {
        TargetId::Esp32c6 if image.architecture == FirmwareArchitecture::RiscV32 => {
            let mut machine = RiscVMachine::new(target).map_err(|error| error.to_string())?;
            machine
                .load_firmware(&image)
                .map_err(|error| error.to_string())?;
            machine
                .load_boot_rom(&boot_rom)
                .map_err(|error| error.to_string())?;
            for frame in &options.radio_frames {
                let result = if frame.mpdus.is_empty() {
                    machine.inject_radio_frame_at(
                        SimTime::from_ticks(frame.at),
                        frame.protocol.into(),
                        Spectrum::new(frame.center_khz, frame.bandwidth_khz),
                        frame.phy.clone(),
                        frame.bytes.clone(),
                        frame.power_dbm,
                    )
                } else if frame.protocol == WebRadioProtocol::Wifi && frame.bytes.is_empty() {
                    machine.inject_wifi_ampdu_at(
                        SimTime::from_ticks(frame.at),
                        Spectrum::new(frame.center_khz, frame.bandwidth_khz),
                        frame.mpdus.clone(),
                        frame.power_dbm,
                    )
                } else {
                    return Err(
                        "aggregate radio input requires Wi-Fi protocol and an empty bytes field"
                            .to_owned(),
                    );
                };
                result.map_err(|error| error.to_string())?;
            }
            let run = machine
                .run_with_stimuli(options.run.limits(), &options.run.machine_stimuli(), None)
                .map_err(|error| error.to_string())?;
            let radio = machine
                .radio_replay_artifact()
                .ok_or_else(|| "ESP32-C6 radio subsystem is unavailable".to_owned())?;
            WebRadioRunOutput { run, radio }
        }
        TargetId::Esp32s3 if image.architecture == FirmwareArchitecture::Xtensa => {
            let mut machine = XtensaMachine::new(target).map_err(|error| error.to_string())?;
            machine
                .load_boot_rom(&boot_rom)
                .map_err(|error| error.to_string())?;
            machine
                .load_firmware(&image)
                .map_err(|error| error.to_string())?;
            for frame in &options.radio_frames {
                let result = if frame.mpdus.is_empty() {
                    machine.inject_radio_frame_at(
                        SimTime::from_ticks(frame.at),
                        frame.protocol.into(),
                        Spectrum::new(frame.center_khz, frame.bandwidth_khz),
                        frame.phy.clone(),
                        frame.bytes.clone(),
                        frame.power_dbm,
                    )
                } else if frame.protocol == WebRadioProtocol::Wifi && frame.bytes.is_empty() {
                    machine.inject_wifi_ampdu_at(
                        SimTime::from_ticks(frame.at),
                        Spectrum::new(frame.center_khz, frame.bandwidth_khz),
                        frame.mpdus.clone(),
                        frame.power_dbm,
                    )
                } else {
                    return Err(
                        "aggregate radio input requires Wi-Fi protocol and an empty bytes field"
                            .to_owned(),
                    );
                };
                result.map_err(|error| error.to_string())?;
            }
            let run = machine
                .run_with_stimuli(options.run.limits(), &options.run.machine_stimuli(), None)
                .map_err(|error| error.to_string())?;
            let radio = machine.radio_replay_artifact();
            WebRadioRunOutput { run, radio }
        }
        TargetId::Esp32c6 | TargetId::Esp32s3 => {
            return Err(format!(
                "firmware architecture {:?} does not match radio target {target}",
                image.architecture
            ));
        }
        _ => return Err(format!("target {target} has no supported radio subsystem")),
    };
    serde_json::to_string(&output).map_err(|error| error.to_string())
}

/// Runs Intel HEX for the targets whose native program format is not ELF.
pub fn run_intel_hex_json(
    target: &str,
    firmware: &[u8],
    options: &WebRunOptions,
) -> Result<String, String> {
    let target = target.parse::<TargetId>()?;
    let image = IntelHexImage::parse(firmware).map_err(|error| error.to_string())?;
    let limits = options.limits();
    let stimuli = options.machine_stimuli();
    let result = match target {
        TargetId::Pic16f15376 => {
            let program = image
                .program_words(14, ProgramWordEndianness::Little)
                .map_err(|error| error.to_string())?;
            let mut machine = Pic16McuMachine::new(target).map_err(|error| error.to_string())?;
            machine
                .load_program(&program)
                .map_err(|error| error.to_string())?;
            machine
                .run_with_stimuli(limits, &stimuli, None)
                .map_err(|error| error.to_string())?
        }
        TargetId::Efm8bb52f32g => {
            let mut machine = Mcs51McuMachine::new(target).map_err(|error| error.to_string())?;
            machine
                .load_program(&image)
                .map_err(|error| error.to_string())?;
            machine
                .run_with_stimuli(limits, &stimuli, None)
                .map_err(|error| error.to_string())?
        }
        _ => {
            return Err(format!(
                "Intel HEX execution is not supported for target {target}"
            ));
        }
    };
    serde_json::to_string(&result).map_err(|error| error.to_string())
}

fn run_elf_image(
    target: TargetId,
    image: &FirmwareImage,
    options: &WebRunOptions,
) -> Result<RunResult, String> {
    let limits = options.limits();
    let stimuli = options.machine_stimuli();
    match image.architecture {
        FirmwareArchitecture::RiscV32 => {
            let mut machine = RiscVMachine::new(target).map_err(|error| error.to_string())?;
            machine
                .load_firmware(image)
                .map_err(|error| error.to_string())?;
            machine
                .run_with_stimuli(limits, &stimuli, None)
                .map_err(|error| error.to_string())
        }
        FirmwareArchitecture::Arm => {
            if matches!(
                target,
                TargetId::Atsamd21e18
                    | TargetId::Atsamd51j19a
                    | TargetId::Stm32l432kc
                    | TargetId::Stm32f103c8
                    | TargetId::Stm32f411re
                    | TargetId::Nrf52840
                    | TargetId::R7fa4m1ab3cfm
            ) {
                let mut machine = ArmMcuMachine::new(target).map_err(|error| error.to_string())?;
                machine
                    .load_firmware(image)
                    .map_err(|error| error.to_string())?;
                machine
                    .run_with_stimuli(limits, &stimuli, None)
                    .map_err(|error| error.to_string())
            } else {
                let mut machine = ArmMachine::new(target).map_err(|error| error.to_string())?;
                machine
                    .load_firmware(image)
                    .map_err(|error| error.to_string())?;
                machine
                    .run_with_stimuli(limits, &stimuli, None)
                    .map_err(|error| error.to_string())
            }
        }
        FirmwareArchitecture::Xtensa => {
            let mut machine = XtensaMachine::new(target).map_err(|error| error.to_string())?;
            machine
                .load_firmware(image)
                .map_err(|error| error.to_string())?;
            machine
                .run_with_stimuli(limits, &stimuli, None)
                .map_err(|error| error.to_string())
        }
        FirmwareArchitecture::Avr8 => {
            let mut machine = AvrMcuMachine::new(target).map_err(|error| error.to_string())?;
            machine
                .load_firmware(image)
                .map_err(|error| error.to_string())?;
            machine
                .run_with_stimuli(limits, &stimuli, None)
                .map_err(|error| error.to_string())
        }
        FirmwareArchitecture::Msp430X => {
            let mut machine = Msp430McuMachine::new(target).map_err(|error| error.to_string())?;
            machine
                .load_firmware(image)
                .map_err(|error| error.to_string())?;
            machine
                .run_with_stimuli(limits, &stimuli, None)
                .map_err(|error| error.to_string())
        }
        FirmwareArchitecture::Pic16Enhanced | FirmwareArchitecture::Mcs51 => Err(format!(
            "architecture {:?} requires the Intel HEX API",
            image.architecture
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use remu_image::FirmwareSegment;

    #[test]
    fn target_inventory_is_valid_json_and_contains_esp32c6() {
        let targets: serde_json::Value =
            serde_json::from_str(&list_targets_json().unwrap()).unwrap();
        assert!(targets.as_array().unwrap().iter().any(|target| {
            target
                .get("id")
                .is_some_and(|id| id.as_str() == Some("esp32c6"))
        }));
    }

    #[test]
    fn direct_riscv_image_runs_through_the_portable_boundary() {
        let image = FirmwareImage {
            architecture: FirmwareArchitecture::RiscV32,
            entry: 0,
            segments: vec![FirmwareSegment {
                address: 0,
                load_address: None,
                data: 0x0010_0073_u32.to_le_bytes().to_vec(),
                initialized_size: 4,
                executable: true,
                writable: false,
                alignment: 4,
            }],
            symbols: Vec::new(),
        };
        let result = run_elf_image(TargetId::Ch32v003, &image, &WebRunOptions::default()).unwrap();
        assert_eq!(result.target, TargetId::Ch32v003);
        assert_eq!(result.reason, remu_core::StopReason::Halted);
    }

    #[test]
    fn portable_radio_input_accepts_ampdu_and_keeps_legacy_json_compatible() {
        let aggregate: WebRadioFrame = serde_json::from_value(serde_json::json!({
            "at": 10,
            "protocol": "wifi",
            "center_khz": 2412000,
            "bandwidth_khz": 20000,
            "phy": "wifi-ht20-ampdu",
            "mpdus": [[136, 0, 1], [136, 0, 2]],
            "power_dbm": -40
        }))
        .unwrap();
        assert!(aggregate.bytes.is_empty());
        assert_eq!(aggregate.mpdus.len(), 2);

        let legacy: WebRadioFrame = serde_json::from_value(serde_json::json!({
            "at": 10,
            "protocol": "wifi",
            "center_khz": 2412000,
            "bandwidth_khz": 20000,
            "phy": "wifi-ht20",
            "bytes": [64, 0],
            "power_dbm": -40
        }))
        .unwrap();
        assert_eq!(legacy.bytes, [64, 0]);
        assert!(legacy.mpdus.is_empty());
    }
}
