use super::{
    WebLogic, WebPinStimulus, WebRunOptions, inspect_elf_json, list_targets_json, run_elf_json,
    run_intel_hex_json,
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
