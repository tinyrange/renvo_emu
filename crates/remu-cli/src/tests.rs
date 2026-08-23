use super::*;
use clap::CommandFactory;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn clap_definition_is_internally_consistent() {
    Cli::command().debug_assert();
}

#[test]
fn corpus_arguments_require_explicit_separator_for_compiler_flags() {
    let parsed = Cli::try_parse_from([
        "remu",
        "corpus",
        "build",
        "--toolchain",
        "toolchain.toml",
        "--source",
        "src",
        "--output",
        "out",
        "--target",
        "rv32ec",
        "--artifact",
        "artifact.json",
        "--",
        "-O2",
    ])
    .unwrap();
    let Command::Corpus {
        command: CorpusCommand::Build(arguments),
    } = parsed.command
    else {
        panic!("expected corpus build");
    };
    assert_eq!(arguments.arguments, ["-O2"]);
}

#[test]
fn firmware_boot_accepts_repeatable_pin_stimulus() {
    let parsed = Cli::try_parse_from([
        "remu",
        "firmware",
        "boot",
        "--target",
        "rp2040",
        "--image",
        "firmware.uf2",
        "--pin",
        "0=1@0",
        "--pin",
        "1=z@42",
    ])
    .unwrap();
    let Command::Firmware {
        command: FirmwareCommand::Boot(arguments),
    } = parsed.command
    else {
        panic!("expected firmware boot");
    };
    assert_eq!(arguments.pins, ["0=1@0", "1=z@42"]);
}

#[test]
fn firmware_boot_accepts_radio_input_replay_and_peer() {
    let parsed = Cli::try_parse_from([
        "remu",
        "firmware",
        "boot",
        "--target",
        "esp32c6",
        "--image",
        "firmware.bin",
        "--boot-rom",
        "rom.elf",
        "--radio-input",
        "input.json",
        "--radio-replay",
        "replay.json",
        "--radio-script",
        "peer.star",
        "--radio-repl",
    ])
    .unwrap();
    let Command::Firmware {
        command: FirmwareCommand::Boot(arguments),
    } = parsed.command
    else {
        panic!("expected firmware boot");
    };
    assert_eq!(arguments.radio_input, Some(PathBuf::from("input.json")));
    assert_eq!(arguments.radio_replay, Some(PathBuf::from("replay.json")));
    assert_eq!(arguments.radio_script, Some(PathBuf::from("peer.star")));
    assert!(arguments.radio_repl);
}

#[test]
fn firmware_boot_rejects_radio_repl_without_peer() {
    assert!(
        Cli::try_parse_from([
            "remu",
            "firmware",
            "boot",
            "--target",
            "esp32c6",
            "--image",
            "firmware.bin",
            "--radio-repl",
        ])
        .is_err()
    );
}

#[test]
fn direct_run_accepts_bus_log_artifact() {
    let parsed = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "ch32v003",
        "--elf",
        "firmware.elf",
        "--bus-log",
        "accesses.json",
        "--bus-log-region",
        "esp32c6.ble-baseband-registers",
        "--bus-log-region",
        "esp32c6.ble-control-registers",
    ])
    .unwrap();
    let Command::Run(arguments) = parsed.command else {
        panic!("expected direct run");
    };
    assert_eq!(arguments.bus_log, Some(PathBuf::from("accesses.json")));
    assert_eq!(
        arguments.bus_log_region,
        [
            "esp32c6.ble-baseband-registers",
            "esp32c6.ble-control-registers"
        ]
    );
}

#[test]
fn direct_run_accepts_interrupt_transition_log() {
    let parsed = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "esp32c6",
        "--elf",
        "firmware.elf",
        "--interrupt-log",
        "interrupts.json",
    ])
    .unwrap();
    let Command::Run(arguments) = parsed.command else {
        panic!("expected direct run");
    };
    assert_eq!(
        arguments.interrupt_log,
        Some(PathBuf::from("interrupts.json"))
    );
}

#[test]
fn direct_run_rejects_bus_log_region_without_bus_log() {
    let result = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "esp32c6",
        "--elf",
        "firmware.elf",
        "--bus-log-region",
        "esp32c6.ble-baseband-registers",
    ]);
    assert!(result.is_err());
}

#[test]
fn direct_run_accepts_radio_replay_artifact() {
    let parsed = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "esp32c6",
        "--elf",
        "firmware.elf",
        "--radio-replay",
        "radio.json",
    ])
    .unwrap();
    let Command::Run(arguments) = parsed.command else {
        panic!("expected direct run");
    };
    assert_eq!(arguments.radio_replay, Some(PathBuf::from("radio.json")));
}

#[test]
fn direct_run_accepts_radio_input_artifact() {
    let parsed = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "esp32s3",
        "--elf",
        "firmware.elf",
        "--radio-input",
        "radio-input.json",
    ])
    .unwrap();
    let Command::Run(arguments) = parsed.command else {
        panic!("expected direct run");
    };
    assert_eq!(
        arguments.radio_input,
        Some(PathBuf::from("radio-input.json"))
    );
}

#[test]
fn direct_run_accepts_radio_script_and_repl() {
    let parsed = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "esp32c6",
        "--elf",
        "firmware.elf",
        "--radio-script",
        "peer.star",
        "--radio-repl",
    ])
    .unwrap();
    let Command::Run(arguments) = parsed.command else {
        panic!("expected direct run");
    };
    assert_eq!(arguments.radio_script, Some(PathBuf::from("peer.star")));
    assert!(arguments.radio_repl);
}

#[test]
fn direct_run_accepts_agent_script_and_scoped_repl() {
    let cli = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "esp32-c6",
        "--elf",
        "firmware.elf",
        "--boot-rom",
        "rom.elf",
        "--agent-script",
        "drive.star",
        "--agent-repl",
        "--agent-artifact",
        "agent.json",
    ])
    .unwrap();
    let Command::Run(arguments) = cli.command else {
        panic!("expected run command");
    };
    assert_eq!(arguments.agent_script, Some(PathBuf::from("drive.star")));
    assert!(arguments.agent_repl);
    assert_eq!(arguments.agent_artifact, Some(PathBuf::from("agent.json")));
}

#[test]
fn direct_run_rejects_agent_artifact_without_script() {
    let error = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "esp32-c6",
        "--elf",
        "firmware.elf",
        "--boot-rom",
        "rom.elf",
        "--agent-artifact",
        "agent.json",
    ])
    .unwrap_err();
    assert!(error.to_string().contains("--agent-script"));
}

#[test]
fn agent_artifact_keeps_bulk_transport_output_out_of_the_decision_record() {
    let outcome = evaluate_agent_script(
        "artifact.star",
        "def main():\n    machine.breakpoint(0x40000000)\n    return {\"decision\": machine.run(instructions = 1)[\"reason\"]}\n",
        AgentMachine::Esp32c6(Box::new(RiscVMachine::new(TargetId::Esp32c6).unwrap())),
        false,
    )
    .unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "remu-agent-artifact-{}-{nonce}.json",
        std::process::id()
    ));
    crate::run_command::write_agent_artifact(&path, Path::new("artifact.star"), &outcome).unwrap();
    let artifact: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    fs::remove_file(path).unwrap();
    assert_eq!(artifact["schema"], "remu.agent-session.v1");
    assert_eq!(artifact["value"]["decision"], "Breakpoint");
    assert_eq!(artifact["run"]["uart_bytes"], 0);
    assert!(artifact["run"].get("uart").is_none());
    assert!(artifact["run"]["cpu"].get("registers").is_none());
}

#[test]
fn direct_run_rejects_agent_repl_without_script() {
    let error = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "esp32-c6",
        "--elf",
        "firmware.elf",
        "--boot-rom",
        "rom.elf",
        "--agent-repl",
    ])
    .unwrap_err();
    assert!(error.to_string().contains("--agent-script"));
}

#[test]
fn direct_run_rejects_radio_repl_without_script() {
    assert!(
        Cli::try_parse_from([
            "remu",
            "run",
            "--target",
            "esp32c6",
            "--elf",
            "firmware.elf",
            "--radio-repl",
        ])
        .is_err()
    );
}

#[test]
fn direct_run_accepts_esp32c6_boot_image_validation() {
    let parsed = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "esp32c6",
        "--elf",
        "firmware.elf",
        "--esp-app-image",
        "firmware.bin",
        "--esp-app-offset",
        "0x20000",
    ])
    .unwrap();
    let Command::Run(arguments) = parsed.command else {
        panic!("expected direct run");
    };
    assert_eq!(arguments.esp_app_image, Some(PathBuf::from("firmware.bin")));
    assert_eq!(arguments.esp_app_offset, Some(0x2_0000));
}

#[test]
fn direct_run_rejects_esp_offset_without_an_image() {
    let result = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "esp32c6",
        "--elf",
        "firmware.elf",
        "--esp-app-offset",
        "0x10000",
    ]);
    assert!(result.is_err());
}

#[test]
fn direct_run_accepts_typed_debug_and_signal_stops() {
    let parsed = Cli::try_parse_from([
        "remu",
        "run",
        "--target",
        "ch32v003",
        "--elf",
        "firmware.elf",
        "--breakpoint",
        "0x20",
        "--watchpoint",
        "64",
        "--stop-signal",
        "board.ch32v003.gpioc.pin1=rising",
    ])
    .unwrap();
    let Command::Run(arguments) = parsed.command else {
        panic!("expected direct run");
    };
    assert_eq!(arguments.breakpoint, [0x20]);
    assert_eq!(arguments.watchpoint, [64]);
    assert_eq!(
        arguments.signal_stops,
        [SignalStopArg {
            path: "board.ch32v003.gpioc.pin1".to_owned(),
            edge: SignalEdge::Rising,
        }]
    );
}

#[test]
fn firmware_boot_accepts_multiple_raw_repl_scripts() {
    let parsed = Cli::try_parse_from([
        "remu",
        "firmware",
        "boot",
        "--target",
        "rp2040",
        "--image",
        "firmware.uf2",
        "--usb-script",
        "timer.py",
        "--usb-script",
        "gpio.py",
    ])
    .unwrap();
    let Command::Firmware {
        command: FirmwareCommand::Boot(arguments),
    } = parsed.command
    else {
        panic!("expected firmware boot");
    };
    assert_eq!(
        arguments.usb_script,
        [PathBuf::from("timer.py"), PathBuf::from("gpio.py")]
    );
}
