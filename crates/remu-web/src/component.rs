use super::{
    WebLogic, WebPinStimulus, WebRadioFrame, WebRadioProtocol, WebRadioRunOptions, WebRunOptions,
    inspect_elf_json, list_targets_json, run_elf_json, run_intel_hex_json, run_radio_elf_json,
};

// wit-bindgen's generated canonical-ABI adapter necessarily contains unsafe
// pointer operations. Hand-written component code remains under `unsafe_code = deny`.
#[allow(unsafe_code)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit/renvo.wit",
        world: "renvo",
    });

    use super::RenvoComponent;
    export!(RenvoComponent);
}

struct RenvoComponent;

impl bindings::exports::renvo::emulator::api::Guest for RenvoComponent {
    fn list_targets() -> Result<String, String> {
        list_targets_json()
    }

    fn inspect_elf(firmware: Vec<u8>) -> Result<String, String> {
        inspect_elf_json(&firmware)
    }

    fn run_elf(
        target: String,
        firmware: Vec<u8>,
        options: bindings::exports::renvo::emulator::api::RunOptions,
    ) -> Result<String, String> {
        run_elf_json(&target, &firmware, &convert_options(options))
    }

    fn run_intel_hex(
        target: String,
        firmware: Vec<u8>,
        options: bindings::exports::renvo::emulator::api::RunOptions,
    ) -> Result<String, String> {
        run_intel_hex_json(&target, &firmware, &convert_options(options))
    }

    fn run_radio_elf(
        target: String,
        firmware: Vec<u8>,
        boot_rom: Vec<u8>,
        options: bindings::exports::renvo::emulator::api::RadioRunOptions,
    ) -> Result<String, String> {
        run_radio_elf_json(
            &target,
            &firmware,
            &boot_rom,
            &convert_radio_options(options),
        )
    }
}

fn convert_options(options: bindings::exports::renvo::emulator::api::RunOptions) -> WebRunOptions {
    WebRunOptions {
        max_instructions: options.max_instructions,
        deadline_ticks: options.deadline_ticks,
        stimuli: options
            .stimuli
            .into_iter()
            .map(|stimulus| WebPinStimulus {
                at: stimulus.at,
                pin: stimulus.pin,
                value: match stimulus.value {
                    bindings::exports::renvo::emulator::api::Logic::Zero => WebLogic::Zero,
                    bindings::exports::renvo::emulator::api::Logic::One => WebLogic::One,
                    bindings::exports::renvo::emulator::api::Logic::HighZ => WebLogic::HighZ,
                    bindings::exports::renvo::emulator::api::Logic::Unknown => WebLogic::Unknown,
                },
            })
            .collect(),
    }
}

fn convert_radio_options(
    options: bindings::exports::renvo::emulator::api::RadioRunOptions,
) -> WebRadioRunOptions {
    WebRadioRunOptions {
        run: convert_options(options.run),
        radio_frames: options
            .radio_frames
            .into_iter()
            .map(|frame| WebRadioFrame {
                at: frame.at,
                protocol: match frame.protocol {
                    bindings::exports::renvo::emulator::api::RadioProtocol::Wifi => {
                        WebRadioProtocol::Wifi
                    }
                    bindings::exports::renvo::emulator::api::RadioProtocol::BluetoothLe => {
                        WebRadioProtocol::BluetoothLe
                    }
                    bindings::exports::renvo::emulator::api::RadioProtocol::Ieee802154 => {
                        WebRadioProtocol::Ieee802154
                    }
                },
                center_khz: frame.center_khz,
                bandwidth_khz: frame.bandwidth_khz,
                phy: frame.phy,
                bytes: frame.bytes,
                mpdus: frame.mpdus,
                power_dbm: frame.power_dbm,
            })
            .collect(),
    }
}
