//! Renvo Emulator command-line entry point.

use clap::{Args, Parser, Subcommand, ValueEnum};
use remu_core::{CpuSnapshot, RunLimits, SimTime, StopReason};
use remu_corpus::{
    BuildArtifact, BuildRequest, CaseReductionResult, CompilerMatrix, DockerCompiler, DockerLimits,
    NamedObservation, ReductionCandidate, ToolchainSpec, compare_observations, reduce_case,
};
use remu_gdb::{
    DebugArchitecture, DebugStop, DebugTarget, ServerConfig, SessionReport, serve_once,
};
use remu_image::{
    EspExecutableImage, EspFlashImage, FirmwareArchitecture, FirmwareImage, IntelHexImage,
    OfficialFirmwareSuite, ProgramWordEndianness, Uf2Image,
};
use remu_machines::{
    ArmMachine, ArmMcuMachine, AvrMcuMachine, HOST_SCRIPT_COMPLETE_MARKER, Mcs51McuMachine,
    Msp430McuMachine, Pic16McuMachine, PinStimulus, RiscVMachine, RunResult, SignalEdge, TargetId,
    XtensaMachine, target_manifest, target_manifests,
};
use remu_signals::Logic;
use remu_starlark::{AgentMachine, StarlarkRadioPeer, evaluate_agent_script, evaluate_script};
use remu_trace::{Timescale, TraceSink, VcdWriter};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

mod corpus_command;
use corpus_command::corpus;
mod access_output;
mod board_command;
use board_command::board;
mod debug_command;
use debug_command::{gdb, script};
mod firmware_command;
use firmware_command::firmware;
mod native_firmware;
mod run_command;
use run_command::{
    inspect, list_targets, parse_address, parse_signal_stop, parse_stimulus, run, run_loaded,
};

#[derive(Debug, Parser)]
#[command(
    name = "remu",
    version,
    about = "Renvo Emulator: deterministic microcontroller emulation and compiler testing"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Lists all supported targets and their current fidelity.
    Targets {
        /// Emit stable JSON rather than a table.
        #[arg(long)]
        json: bool,
    },
    /// Inspects a supported ELF without running it.
    Inspect {
        /// Firmware ELF.
        elf: PathBuf,
    },
    /// Runs a direct-mode RISC-V firmware ELF.
    Run(RunArgs),
    /// Official firmware artifact and container operations.
    Firmware {
        #[command(subcommand)]
        command: FirmwareCommand,
    },
    /// Docker-only compiler corpus operations.
    Corpus {
        #[command(subcommand)]
        command: CorpusCommand,
    },
    /// Evaluates bounded Starlark assertions over explicit JSON artifacts.
    Script(ScriptArgs),
    /// Runs a declarative Starlark board/component scenario.
    Board(BoardArgs),
    /// Serves one GDB remote-debugging session for a direct ELF.
    Gdb(GdbArgs),
}

#[derive(Debug, Args)]
struct GdbArgs {
    /// Microcontroller model receiving the ELF.
    #[arg(long)]
    target: String,
    /// Compiler-produced ELF32 firmware.
    #[arg(long)]
    elf: PathBuf,
    /// TCP address, including `:0` for an ephemeral port.
    #[arg(long, default_value = "127.0.0.1:3333")]
    listen: String,
    /// JSON file written after bind and before accepting a client.
    #[arg(long)]
    ready: Option<PathBuf>,
    /// JSON session report written after detach.
    #[arg(long)]
    artifact: PathBuf,
    /// Safety bound for one GDB continue packet.
    #[arg(long, default_value_t = 10_000_000)]
    max_continue_instructions: u64,
}

#[derive(Debug, Args)]
struct ScriptArgs {
    /// Starlark assertion source.
    #[arg(long)]
    file: PathBuf,
    /// JSON dataset as NAME=PATH; repeat for multiple immutable inputs.
    #[arg(long = "data")]
    datasets: Vec<String>,
    /// Stable JSON evaluation artifact.
    #[arg(long)]
    artifact: PathBuf,
}

#[derive(Debug, Args)]
struct BoardArgs {
    /// Starlark board test whose final expression is a board.
    #[arg(long)]
    file: PathBuf,
    /// Root used to resolve confined `load("//package:file.star", ...)` labels.
    #[arg(long, default_value = ".")]
    load_root: PathBuf,
    /// Optional Xtensa ELF to run against a live `M5StickS3` component graph.
    #[arg(long)]
    elf: Option<PathBuf>,
    /// Maximum interpreted instructions for a live firmware board run.
    #[arg(long, default_value_t = 1_000_000)]
    max_instructions: u64,
    /// Optional virtual-time deadline for a live firmware board run.
    #[arg(long)]
    deadline: Option<u64>,
    /// Stable board-simulation JSON artifact.
    #[arg(long)]
    artifact: PathBuf,
    /// Optional VCD waveform output.
    #[arg(long)]
    vcd: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum FirmwareCommand {
    /// Verifies an official firmware cache against a pinned manifest.
    Verify(FirmwareVerifyArgs),
    /// Parses and summarizes a supported native firmware container.
    Inspect(FirmwareInspectArgs),
    /// Runs a native flash image through the target's reset/boot boundary.
    Boot(FirmwareBootArgs),
    /// Reconstructs the contiguous payload from a UF2 for diagnostics.
    ExtractUf2(FirmwareExtractUf2Args),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum FirmwareFormatArg {
    /// Detect from file magic.
    Auto,
    /// Microsoft UF2.
    Uf2,
    /// Espressif merged flash binary.
    EspBin,
    /// Intel HEX with absolute flash addresses.
    IntelHex,
    /// Addressless bytes rooted at the target's primary flash base.
    RawBin,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum FirmwareCpuArg {
    /// Arm M-profile execution.
    Arm,
    /// RISC-V execution.
    Riscv,
}

#[derive(Debug, Args)]
struct FirmwareVerifyArgs {
    /// Official firmware suite TOML.
    #[arg(long)]
    manifest: PathBuf,
    /// Directory containing the downloaded artifacts.
    #[arg(long)]
    cache: PathBuf,
    /// Optional stable JSON verification report.
    #[arg(long)]
    artifact: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct FirmwareInspectArgs {
    /// UF2, merged ESP binary, Intel HEX, or raw binary.
    image: PathBuf,
    /// Container format, or automatic magic detection.
    #[arg(long, value_enum, default_value_t = FirmwareFormatArg::Auto)]
    format: FirmwareFormatArg,
}

#[derive(Debug, Args)]
struct FirmwareBootArgs {
    /// Target receiving the native flash artifact.
    #[arg(long)]
    target: String,
    /// Processor architecture selected by a dual-ISA target.
    #[arg(long, value_enum, default_value_t = FirmwareCpuArg::Arm)]
    cpu: FirmwareCpuArg,
    /// Native flash image.
    #[arg(long)]
    image: PathBuf,
    /// Native container format, or automatic detection from target and magic.
    #[arg(long, value_enum, default_value_t = FirmwareFormatArg::Auto)]
    format: FirmwareFormatArg,
    /// Official merged ESP image supplying bootloader/partitions for an app-only UF2.
    #[arg(long)]
    esp_base_image: Option<PathBuf>,
    /// Complete chip boot-ROM image; required for ESP32-C6 and ESP32-S3 native boot.
    #[arg(long)]
    boot_rom: Option<PathBuf>,
    /// Bytes to deliver after native USB enumeration, typically a REPL transcript.
    #[arg(long)]
    usb_input: Option<PathBuf>,
    /// Python source to deliver through the standard raw REPL; repeat to run
    /// multiple sources in one firmware session.
    #[arg(long, conflicts_with = "usb_input")]
    usb_script: Vec<PathBuf>,
    /// Persistent full flash state; created on first run.
    #[arg(long)]
    flash_state: Option<PathBuf>,
    /// Maximum interpreted CPU actions.
    #[arg(long, default_value_t = 1_000_000)]
    max_instructions: u64,
    /// Optional inclusive virtual-time deadline in ticks.
    #[arg(long)]
    deadline: Option<u64>,
    /// External drive as PIN=VALUE@TICK, where VALUE is 0, 1, z, or x.
    #[arg(long = "pin")]
    pins: Vec<String>,
    /// Stream GPIO changes to this VCD file.
    #[arg(long)]
    vcd: Option<PathBuf>,
    /// Write the run result to this file instead of stdout.
    #[arg(long)]
    result: Option<PathBuf>,
    /// Optional JSON record of completed memory and MMIO operations.
    #[arg(long)]
    bus_log: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct FirmwareExtractUf2Args {
    /// UF2 image to reconstruct.
    image: PathBuf,
    /// Raw payload output path.
    output: PathBuf,
}

#[derive(Debug, Args)]
struct RunArgs {
    /// One of the target identifiers reported by `remu targets`.
    #[arg(long)]
    target: String,
    /// Compiler-produced ELF32 firmware.
    #[arg(long, required_unless_present = "hex", conflicts_with = "hex")]
    elf: Option<PathBuf>,
    /// Real chip mask-ROM ELF required for ESP native/radio execution.
    #[arg(long, requires = "elf", conflicts_with = "hex")]
    boot_rom: Option<PathBuf>,
    /// Compiler-produced Intel HEX firmware (PIC16F15376 and MCS-51 targets).
    #[arg(long, required_unless_present = "elf", conflicts_with = "elf")]
    hex: Option<PathBuf>,
    /// Maximum interpreted CPU actions.
    #[arg(long, default_value_t = 1_000_000)]
    max_instructions: u64,
    /// Optional inclusive virtual-time deadline in ticks.
    #[arg(long)]
    deadline: Option<u64>,
    /// Stream GPIO changes to this VCD file.
    #[arg(long)]
    vcd: Option<PathBuf>,
    /// External drive as PIN=VALUE@TICK, where VALUE is 0, 1, z, or x.
    #[arg(long = "pin")]
    pins: Vec<String>,
    /// Write the run result to this file instead of stdout.
    #[arg(long)]
    result: Option<PathBuf>,
    /// Stream completed memory and MMIO operations to this JSON file.
    #[arg(long)]
    bus_log: Option<PathBuf>,
    /// Retain only accesses to this exact bus-region name in --bus-log; repeatable.
    #[arg(long, requires = "bus_log")]
    bus_log_region: Vec<String>,
    /// esptool application or merged flash binary for ESP32-C6/ESP32-S3.
    #[arg(long, requires = "elf", conflicts_with = "hex")]
    esp_app_image: Option<PathBuf>,
    /// Flash partition offset of --esp-app-image (default: 0x10000).
    #[arg(long, requires = "esp_app_image", value_parser = parse_address)]
    esp_app_offset: Option<u64>,
    /// Write deterministic instruction-fetch coverage as JSON.
    #[arg(long)]
    coverage: Option<PathBuf>,
    /// Write the deterministic isolated RF-medium replay artifact as JSON.
    #[arg(long)]
    radio_replay: Option<PathBuf>,
    /// Read deterministic timestamped RF input frames from a JSON artifact.
    #[arg(long)]
    radio_input: Option<PathBuf>,
    /// Run an event-driven deterministic Starlark peer on emitted RF frames.
    #[arg(long)]
    radio_script: Option<PathBuf>,
    /// Enable `repl()`/`breakpoint()` terminal sessions inside the radio script.
    #[arg(long, requires = "radio_script")]
    radio_repl: bool,
    /// Drive a live ESP32-C6/ESP32-S3 machine from a bounded Starlark `main()`.
    #[arg(long, requires = "boot_rom")]
    agent_script: Option<PathBuf>,
    /// Enable scoped `repl()` sessions inside the agent driver script.
    #[arg(long, requires = "agent_script")]
    agent_repl: bool,
    /// Require the complete result to match a prior JSON result exactly.
    #[arg(long)]
    replay: Option<PathBuf>,
    /// Stop before executing this address; accepts decimal or 0x-prefixed hex.
    #[arg(long, value_parser = parse_address)]
    breakpoint: Vec<u64>,
    /// Stop after a data access overlaps this address; accepts decimal or 0x-prefixed hex.
    #[arg(long, value_parser = parse_address)]
    watchpoint: Vec<u64>,
    /// Stop on PATH=change, PATH=rising, or PATH=falling.
    #[arg(long = "stop-signal", value_parser = parse_signal_stop)]
    signal_stops: Vec<SignalStopArg>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SignalStopArg {
    path: String,
    edge: SignalEdge,
}

#[derive(Clone, Default)]
struct DirectRunControl<'a> {
    access_observer: Option<remu_bus::SharedBusAccessObserver>,
    esp32c6_mmu_page_size: Option<u32>,
    esp32c6_flash_image: Option<Vec<u8>>,
    esp32c6_boot_image: Option<(EspExecutableImage, u32)>,
    esp32s3_boot_image: Option<(EspFlashImage, Vec<u8>)>,
    esp_boot_rom: Option<FirmwareImage>,
    radio_replay: Option<&'a Path>,
    radio_input: Option<&'a Path>,
    radio_script: Option<&'a Path>,
    radio_repl: bool,
    agent_script: Option<&'a Path>,
    agent_repl: bool,
    breakpoints: &'a [u64],
    watchpoints: &'a [u64],
    signal_stops: &'a [SignalStopArg],
}

#[derive(Debug, Subcommand)]
enum CorpusCommand {
    /// Compiles one case inside a pinned, network-isolated Docker image.
    Build(CorpusBuildArgs),
    /// Builds every compiler/optimization variant in a TOML matrix.
    Matrix(CorpusMatrixArgs),
    /// Compares selected JSON Pointer fields across run artifacts.
    Compare(CorpusCompareArgs),
    /// Runs a directory of case ELFs against an expected-result manifest.
    Run(CorpusRunArgs),
    /// Reduces a seeded compiler/emulator discrepancy across three axes.
    Reduce(CorpusReduceArgs),
    /// Verifies that the Docker daemon is reachable.
    Doctor,
}

#[derive(Debug, Args)]
struct CorpusMatrixArgs {
    /// TOML file containing a `CompilerMatrix`.
    #[arg(long)]
    matrix: PathBuf,
    /// Source directory mounted read-only for every variant.
    #[arg(long)]
    source: PathBuf,
    /// Root directory receiving one subdirectory per variant.
    #[arg(long)]
    output: PathBuf,
    /// Compiler target recorded in provenance.
    #[arg(long)]
    target: String,
    /// Root directory receiving build artifact JSON files.
    #[arg(long)]
    artifacts: PathBuf,
    /// Wall-time limit for each compiler container.
    #[arg(long, default_value_t = 120)]
    timeout_seconds: u64,
}

#[derive(Debug, Args)]
struct CorpusCompareArgs {
    /// JSON Pointer to compare; repeat for multiple observables.
    #[arg(long = "pointer", required = true)]
    pointers: Vec<String>,
    /// Two or more JSON run artifacts, with the first as baseline.
    #[arg(required = true, num_args = 2..)]
    observations: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct CorpusBuildArgs {
    /// TOML file containing a `ToolchainSpec`.
    #[arg(long)]
    toolchain: PathBuf,
    /// Source directory mounted read-only at /workspace/src.
    #[arg(long)]
    source: PathBuf,
    /// Output directory mounted read-write at /workspace/out.
    #[arg(long)]
    output: PathBuf,
    /// Compiler target recorded in provenance.
    #[arg(long)]
    target: String,
    /// JSON provenance artifact path.
    #[arg(long)]
    artifact: PathBuf,
    /// Print the Docker argv without starting a container.
    #[arg(long)]
    dry_run: bool,
    /// Wall-time limit for the compiler container.
    #[arg(long, default_value_t = 120)]
    timeout_seconds: u64,
    /// Arguments passed to the compiler/build program after `--`.
    #[arg(last = true)]
    arguments: Vec<String>,
}

#[derive(Debug, Args)]
struct CorpusRunArgs {
    /// Microcontroller model receiving every ELF.
    #[arg(long)]
    target: String,
    /// Directory containing `case_NNNN.elf` files.
    #[arg(long)]
    input: PathBuf,
    /// Tab-separated expected-result manifest from remu-casegen.
    #[arg(long)]
    manifest: PathBuf,
    /// Maximum interpreted CPU actions per case.
    #[arg(long, default_value_t = 100_000)]
    max_instructions: u64,
    /// Stable JSON conformance result artifact.
    #[arg(long)]
    artifact: PathBuf,
}

#[derive(Debug, Args)]
struct CorpusReduceArgs {
    /// Microcontroller model used for every predicate evaluation.
    #[arg(long)]
    target: String,
    /// Pinned Docker toolchain specification.
    #[arg(long)]
    toolchain: PathBuf,
    /// Stable harness directory copied into each isolated evaluation.
    #[arg(long)]
    source: PathBuf,
    /// Root receiving every build, run, and final reduction artifact.
    #[arg(long)]
    output: PathBuf,
    /// Intentionally seeded reference value that the candidate must diverge from.
    #[arg(long)]
    seed_expected: u32,
    /// Independently removable source line written to `candidate.h`.
    #[arg(long = "source-item", required = true)]
    source_items: Vec<String>,
    /// Independently removable compiler flag.
    #[arg(long = "flag-item", required = true, allow_hyphen_values = true)]
    flag_items: Vec<String>,
    /// Independently removable unsigned input value.
    #[arg(long = "input-item", required = true)]
    input_items: Vec<u32>,
    /// Maximum interpreted CPU actions per predicate evaluation.
    #[arg(long, default_value_t = 100_000)]
    max_instructions: u64,
    /// Stable JSON reduction artifact.
    #[arg(long)]
    artifact: PathBuf,
    /// Base compiler arguments, including sources, linker script, and output.
    #[arg(last = true)]
    arguments: Vec<String>,
}

fn main() {
    if let Err(error) = execute(Cli::parse()) {
        eprintln!("remu: {error}");
        std::process::exit(1);
    }
}

fn execute(cli: Cli) -> Result<(), Box<dyn Error>> {
    match cli.command {
        Command::Targets { json } => list_targets(json)?,
        Command::Inspect { elf } => inspect(&elf)?,
        Command::Run(arguments) => run(&arguments)?,
        Command::Firmware { command } => firmware(command)?,
        Command::Corpus { command } => corpus(command)?,
        Command::Script(arguments) => script(&arguments)?,
        Command::Board(arguments) => board(&arguments)?,
        Command::Gdb(arguments) => gdb(&arguments)?,
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ScriptDatasetArtifact {
    name: String,
    path: String,
    sha256: String,
}

#[derive(Debug, Serialize)]
struct ScriptArtifact {
    schema: &'static str,
    script: String,
    script_sha256: String,
    datasets: Vec<ScriptDatasetArtifact>,
    value: serde_json::Value,
    result: &'static str,
}

#[cfg(test)]
mod tests;
