use super::*;
use clap::CommandFactory;

#[test]
fn clap_definition_is_internally_consistent() {
    Cli::command().debug_assert();
}

#[test]
fn corpus_arguments_require_explicit_separator_for_compiler_flags() {
    let parsed = Cli::try_parse_from([
        "renvo",
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
        "renvo",
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
fn direct_run_accepts_bus_log_artifact() {
    let parsed = Cli::try_parse_from([
        "renvo",
        "run",
        "--target",
        "ch32v003",
        "--elf",
        "firmware.elf",
        "--bus-log",
        "accesses.json",
    ])
    .unwrap();
    let Command::Run(arguments) = parsed.command else {
        panic!("expected direct run");
    };
    assert_eq!(arguments.bus_log, Some(PathBuf::from("accesses.json")));
}

#[test]
fn direct_run_accepts_typed_debug_and_signal_stops() {
    let parsed = Cli::try_parse_from([
        "renvo",
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
        "renvo",
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
