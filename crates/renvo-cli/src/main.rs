//! Renvo command-line entry point.

use clap::{Args, Parser, Subcommand, ValueEnum};
use renvo_core::{AccessKind, CpuSnapshot, RunLimits, SimTime, StopReason};
use renvo_corpus::{
    BuildArtifact, BuildRequest, CaseReductionResult, CompilerMatrix, DockerCompiler, DockerLimits,
    NamedObservation, ReductionCandidate, ToolchainSpec, compare_observations, reduce_case,
};
use renvo_gdb::{
    DebugArchitecture, DebugStop, DebugTarget, ServerConfig, SessionReport, serve_once,
};
use renvo_image::{
    EspFlashImage, FirmwareArchitecture, FirmwareImage, OfficialFirmwareSuite, Uf2Image,
};
use renvo_machines::{
    ArmMachine, HOST_SCRIPT_COMPLETE_MARKER, PinStimulus, RiscVMachine, RunResult, SignalEdge,
    TargetId, XtensaMachine, target_manifests,
};
use renvo_signals::Logic;
use renvo_starlark::evaluate_script;
use renvo_trace::{Timescale, TraceSink, VcdWriter};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "renvo",
    version,
    about = "Deterministic microcontroller emulation and compiler testing"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Lists the six initial targets and their current fidelity.
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

#[derive(Debug, Subcommand)]
enum FirmwareCommand {
    /// Verifies an official firmware cache against a pinned manifest.
    Verify(FirmwareVerifyArgs),
    /// Parses and summarizes a UF2 or merged ESP flash image.
    Inspect(FirmwareInspectArgs),
    /// Runs an official UF2 through the target's reset/boot boundary.
    Boot(FirmwareBootArgs),
    /// Reconstructs the contiguous payload from a UF2 for diagnostics.
    ExtractUf2(FirmwareExtractUf2Args),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum FirmwareFormatArg {
    /// Detect from file magic.
    Auto,
    /// Microsoft UF2.
    Uf2,
    /// Espressif merged flash binary.
    EspBin,
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
    /// UF2 or merged ESP flash image.
    image: PathBuf,
    /// Container format, or automatic magic detection.
    #[arg(long, value_enum, default_value_t = FirmwareFormatArg::Auto)]
    format: FirmwareFormatArg,
}

#[derive(Debug, Args)]
struct FirmwareBootArgs {
    /// Raspberry Pi target receiving the official UF2.
    #[arg(long)]
    target: String,
    /// Processor architecture selected by a dual-ISA target.
    #[arg(long, value_enum, default_value_t = FirmwareCpuArg::Arm)]
    cpu: FirmwareCpuArg,
    /// Official UF2 image.
    #[arg(long)]
    image: PathBuf,
    /// Official merged ESP image supplying bootloader/partitions for an app-only UF2.
    #[arg(long)]
    esp_base_image: Option<PathBuf>,
    /// Complete chip boot-ROM image, when the firmware uses ROM-resident runtime tables.
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
    /// One of: ch32v003, ch32v006, rp2350, esp32c6.
    #[arg(long)]
    target: String,
    /// Compiler-produced ELF32 firmware.
    #[arg(long)]
    elf: PathBuf,
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
    /// Optional JSON record of completed memory and MMIO operations.
    #[arg(long)]
    bus_log: Option<PathBuf>,
    /// Write deterministic instruction-fetch coverage as JSON.
    #[arg(long)]
    coverage: Option<PathBuf>,
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

#[derive(Clone, Copy, Debug, Default)]
struct DirectRunControl<'a> {
    record_accesses: bool,
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
    /// Tab-separated expected-result manifest from renvo-casegen.
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
        eprintln!("renvo: {error}");
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

fn script(arguments: &ScriptArgs) -> Result<(), Box<dyn Error>> {
    let source = fs::read_to_string(&arguments.file)?;
    let mut datasets = BTreeMap::new();
    let mut dataset_artifacts = Vec::new();
    for dataset in &arguments.datasets {
        let (name, path) = dataset
            .split_once('=')
            .ok_or("Starlark dataset must use NAME=PATH")?;
        let bytes = fs::read(path)?;
        let value = serde_json::from_slice(&bytes)?;
        if datasets.insert(name.to_owned(), value).is_some() {
            return Err(format!("duplicate Starlark dataset {name:?}").into());
        }
        dataset_artifacts.push(ScriptDatasetArtifact {
            name: name.to_owned(),
            path: path.to_owned(),
            sha256: hex::encode(Sha256::digest(&bytes)),
        });
    }
    let value = evaluate_script(&arguments.file.display().to_string(), &source, &datasets)?;
    let artifact = ScriptArtifact {
        schema: "renvo.starlark-assertion.v1",
        script: arguments.file.display().to_string(),
        script_sha256: hex::encode(Sha256::digest(source.as_bytes())),
        datasets: dataset_artifacts,
        value,
        result: "pass",
    };
    if let Some(parent) = arguments.artifact.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&arguments.artifact, serde_json::to_vec_pretty(&artifact)?)?;
    println!(
        "Starlark assertions passed; artifact: {}",
        arguments.artifact.display()
    );
    Ok(())
}

enum CliDebugMachine {
    RiscV(Box<RiscVMachine>),
    Arm(Box<ArmMachine>),
    Xtensa(Box<XtensaMachine>),
}

impl CliDebugMachine {
    fn new(target: TargetId, image: &FirmwareImage) -> Result<Self, Box<dyn Error>> {
        match image.architecture {
            FirmwareArchitecture::RiscV32 => {
                let mut machine = RiscVMachine::new(target)?;
                machine.load_firmware(image)?;
                Ok(Self::RiscV(Box::new(machine)))
            }
            FirmwareArchitecture::Arm => {
                let mut machine = ArmMachine::new(target)?;
                machine.load_firmware(image)?;
                Ok(Self::Arm(Box::new(machine)))
            }
            FirmwareArchitecture::Xtensa => {
                let mut machine = XtensaMachine::new(target)?;
                machine.load_firmware(image)?;
                Ok(Self::Xtensa(Box::new(machine)))
            }
        }
    }

    fn run_for(&mut self, instructions: u64) -> Result<RunResult, String> {
        let limits = RunLimits {
            instructions: Some(instructions),
            deadline: None,
        };
        match self {
            Self::RiscV(machine) => machine.run(limits, None).map_err(|error| error.to_string()),
            Self::Arm(machine) => machine.run(limits, None).map_err(|error| error.to_string()),
            Self::Xtensa(machine) => machine.run(limits, None).map_err(|error| error.to_string()),
        }
    }
}

impl DebugTarget for CliDebugMachine {
    fn architecture(&self) -> DebugArchitecture {
        match self {
            Self::RiscV(_) => DebugArchitecture::RiscV32,
            Self::Arm(_) => DebugArchitecture::Arm,
            Self::Xtensa(_) => DebugArchitecture::Xtensa,
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        match self {
            Self::RiscV(machine) => machine.debug_snapshot(),
            Self::Arm(machine) => machine.debug_snapshot(),
            Self::Xtensa(machine) => machine.debug_snapshot(),
        }
    }

    fn read_memory(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        match self {
            Self::RiscV(machine) => machine.debug_read_memory(address, length),
            Self::Arm(machine) => machine.debug_read_memory(address, length),
            Self::Xtensa(machine) => machine.debug_read_memory(address, length),
        }
    }

    fn write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::RiscV(machine) => machine.debug_write_memory(address, bytes),
            Self::Arm(machine) => machine.debug_write_memory(address, bytes),
            Self::Xtensa(machine) => machine.debug_write_memory(address, bytes),
        }
    }

    fn add_breakpoint(&mut self, address: u64) {
        match self {
            Self::RiscV(machine) => machine.add_breakpoint(address),
            Self::Arm(machine) => machine.add_breakpoint(address),
            Self::Xtensa(machine) => machine.add_breakpoint(address),
        }
    }

    fn remove_breakpoint(&mut self, address: u64) {
        match self {
            Self::RiscV(machine) => machine.remove_breakpoint(address),
            Self::Arm(machine) => machine.remove_breakpoint(address),
            Self::Xtensa(machine) => machine.remove_breakpoint(address),
        }
    }

    fn step(&mut self) -> Result<DebugStop, String> {
        self.run_for(1).map(|result| debug_stop(&result))
    }

    fn continue_run(&mut self, max_instructions: u64) -> Result<DebugStop, String> {
        self.run_for(max_instructions)
            .map(|result| debug_stop(&result))
    }
}

fn debug_stop(result: &RunResult) -> DebugStop {
    if let Some(code) = result.exit_code {
        return DebugStop::Exited(code.to_le_bytes()[0]);
    }
    match result.reason {
        StopReason::Fault(_) => DebugStop::Signal(11),
        _ => DebugStop::Signal(5),
    }
}

#[derive(Debug, Serialize)]
struct GdbReadyArtifact {
    schema: &'static str,
    address: String,
}

#[derive(Debug, Serialize)]
struct GdbSessionArtifact {
    schema: &'static str,
    target: TargetId,
    elf: String,
    elf_sha256: String,
    report: SessionReport,
    result: &'static str,
}

fn gdb(arguments: &GdbArgs) -> Result<(), Box<dyn Error>> {
    let target = arguments.target.parse::<TargetId>()?;
    let elf = fs::read(&arguments.elf)?;
    let image = FirmwareImage::parse(&elf)?;
    let mut machine = CliDebugMachine::new(target, &image)?;
    let listener = TcpListener::bind(&arguments.listen)?;
    let address = listener.local_addr()?.to_string();
    if let Some(path) = &arguments.ready {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            serde_json::to_vec_pretty(&GdbReadyArtifact {
                schema: "renvo.gdb-ready.v1",
                address: address.clone(),
            })?,
        )?;
    }
    println!("GDB remote listening on {address}");
    let report = serve_once(
        &listener,
        &mut machine,
        ServerConfig {
            max_continue_instructions: arguments.max_continue_instructions,
        },
    )?;
    let artifact = GdbSessionArtifact {
        schema: "renvo.gdb-session.v1",
        target,
        elf: arguments.elf.display().to_string(),
        elf_sha256: hex::encode(Sha256::digest(&elf)),
        report,
        result: "pass",
    };
    if let Some(parent) = arguments.artifact.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&arguments.artifact, serde_json::to_vec_pretty(&artifact)?)?;
    Ok(())
}

#[derive(Debug, Serialize)]
struct ImageSegmentSummary {
    address: u32,
    size: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    flash_offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    not_main_flash: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "format", rename_all = "kebab-case")]
enum FirmwareInspection {
    Uf2 {
        family_id: Option<u32>,
        blocks: usize,
        metadata_blocks: usize,
        segments: Vec<ImageSegmentSummary>,
    },
    EspBin {
        bootloader: EspExecutableSummary,
        partitions: Vec<renvo_image::EspPartition>,
        partition_table_has_md5: bool,
        application_partition: renvo_image::EspPartition,
        application: EspExecutableSummary,
    },
}

#[derive(Debug, Serialize)]
struct EspExecutableSummary {
    flash_offset: u32,
    entry: u32,
    chip_id: u16,
    segment_count: u8,
    checksum: u8,
    appended_sha256: Option<String>,
    end_offset: u32,
    segments: Vec<ImageSegmentSummary>,
}

fn firmware(command: FirmwareCommand) -> Result<(), Box<dyn Error>> {
    match command {
        FirmwareCommand::Verify(arguments) => {
            let suite = OfficialFirmwareSuite::read(&arguments.manifest)?;
            let verified = suite.verify_directory(&arguments.cache)?;
            let json = serde_json::to_vec_pretty(&verified)?;
            if let Some(path) = arguments.artifact {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&path, &json)?;
                println!(
                    "verified {} official artifacts; report: {}",
                    verified.len(),
                    path.display()
                );
            } else {
                println!("{}", String::from_utf8(json)?);
            }
        }
        FirmwareCommand::Inspect(arguments) => {
            let bytes = fs::read(&arguments.image)?;
            let format = match arguments.format {
                FirmwareFormatArg::Auto if bytes.starts_with(&0x0a32_4655_u32.to_le_bytes()) => {
                    FirmwareFormatArg::Uf2
                }
                FirmwareFormatArg::Auto if bytes.first() == Some(&0xe9) => {
                    FirmwareFormatArg::EspBin
                }
                FirmwareFormatArg::Auto => {
                    return Err(format!(
                        "cannot detect firmware container for {}",
                        arguments.image.display()
                    )
                    .into());
                }
                explicit => explicit,
            };
            let inspection = match format {
                FirmwareFormatArg::Uf2 => {
                    let image = Uf2Image::parse(&bytes)?;
                    FirmwareInspection::Uf2 {
                        family_id: image.family_id,
                        blocks: image.blocks.len(),
                        metadata_blocks: image.metadata_blocks.len(),
                        segments: image
                            .segments
                            .into_iter()
                            .map(|segment| ImageSegmentSummary {
                                address: segment.address,
                                size: segment.data.len(),
                                flash_offset: None,
                                not_main_flash: Some(segment.not_main_flash),
                            })
                            .collect(),
                    }
                }
                FirmwareFormatArg::EspBin => {
                    let image = EspFlashImage::parse(&bytes)?;
                    FirmwareInspection::EspBin {
                        bootloader: summarize_esp_executable(&image.bootloader),
                        partitions: image.partition_table.partitions,
                        partition_table_has_md5: image.partition_table.has_md5,
                        application_partition: image.application_partition,
                        application: summarize_esp_executable(&image.application),
                    }
                }
                FirmwareFormatArg::Auto => unreachable!("auto format was resolved above"),
            };
            println!("{}", serde_json::to_string_pretty(&inspection)?);
        }
        FirmwareCommand::Boot(arguments) => boot_official_uf2(&arguments)?,
        FirmwareCommand::ExtractUf2(arguments) => {
            let image = Uf2Image::parse(&fs::read(&arguments.image)?)?;
            if image.segments.len() != 1 {
                return Err(format!(
                    "{} has {} reconstructed segments; extraction requires exactly one",
                    arguments.image.display(),
                    image.segments.len()
                )
                .into());
            }
            if let Some(parent) = arguments.output.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&arguments.output, &image.segments[0].data)?;
            println!(
                "wrote {} bytes rooted at {:#010x} to {}",
                image.segments[0].data.len(),
                image.segments[0].address,
                arguments.output.display()
            );
        }
    }
    Ok(())
}

fn boot_official_uf2(arguments: &FirmwareBootArgs) -> Result<(), Box<dyn Error>> {
    let target = arguments.target.parse::<TargetId>()?;
    let bytes = fs::read(&arguments.image)?;
    let stimuli = arguments
        .pins
        .iter()
        .map(|value| parse_stimulus(value))
        .collect::<Result<Vec<_>, _>>()?;
    let usb_input = if let Some(path) = &arguments.usb_input {
        Some(fs::read(path)?)
    } else if !arguments.usb_script.is_empty() {
        const CHUNK_MARKER: &str = "# RENVO_CHUNK";
        let mut payload = Vec::new();
        payload.push(0x01); // enter standard raw REPL
        for path in &arguments.usb_script {
            let source = fs::read_to_string(path)?;
            for chunk in source.split(CHUNK_MARKER) {
                let chunk = chunk.trim();
                if chunk.is_empty() {
                    continue;
                }
                payload.extend_from_slice(chunk.as_bytes());
                payload.push(b'\n');
                payload.push(0x04); // compile and execute this bounded chunk
            }
        }
        payload.extend_from_slice(b"print(\"");
        payload.extend_from_slice(HOST_SCRIPT_COMPLETE_MARKER.as_bytes());
        payload.extend_from_slice(b"\")\n\x04");
        Some(payload)
    } else {
        None
    };
    let stop_on_usb_input_complete = !arguments.usb_script.is_empty();
    if matches!(target, TargetId::Esp32c6 | TargetId::Esp32s3) {
        if arguments.boot_rom.is_some() {
            return Err("--boot-rom is not used by the ESP mask-ROM image handoff".into());
        }
        let flash = if bytes.starts_with(b"UF2\n") {
            let uf2 = Uf2Image::parse(&bytes)?;
            let payload_length = uf2
                .segments
                .iter()
                .filter(|segment| !segment.not_main_flash)
                .map(|segment| {
                    usize::try_from(segment.address)
                        .ok()
                        .and_then(|start| start.checked_add(segment.data.len()))
                })
                .collect::<Option<Vec<_>>>()
                .and_then(|ends| ends.into_iter().max())
                .ok_or("ESP UF2 has no representable main-flash payload")?;
            let payload = uf2.materialize(0, payload_length, 0xff)?;
            let payload_image = renvo_image::EspExecutableImage::parse(&payload)?;
            let base_path = arguments.esp_base_image.as_ref().ok_or(
                "an ESP application UF2 requires --esp-base-image for its official bootloader and partition table",
            )?;
            let mut base = fs::read(base_path)?;
            let base_image = EspFlashImage::parse(&base)?;
            if payload_image.header.chip_id != base_image.application.header.chip_id
                || payload_image.header.entry != base_image.application.header.entry
            {
                return Err("ESP UF2 application does not match the supplied merged image".into());
            }
            let offset = usize::try_from(base_image.application_partition.offset)?;
            let executable_length = usize::try_from(payload_image.end_offset)?;
            let comparison_end = offset
                .checked_add(executable_length)
                .ok_or("ESP UF2 comparison range overflow")?;
            if base.get(offset..comparison_end) != payload.get(..executable_length) {
                return Err(
                    "ESP UF2 application bytes differ from the supplied official merged image"
                        .into(),
                );
            }
            base.resize(16 * 1024 * 1024, 0xff);
            let overlay_end = offset
                .checked_add(payload.len())
                .ok_or("ESP UF2 overlay range overflow")?;
            if overlay_end > base.len() {
                return Err("ESP UF2 application exceeds simulated flash".into());
            }
            base[offset..overlay_end].copy_from_slice(&payload);
            base
        } else {
            if arguments.esp_base_image.is_some() {
                return Err("--esp-base-image is only valid with an ESP UF2".into());
            }
            bytes
        };
        let image = EspFlashImage::parse(&flash)?;
        let flash_state = if let Some(path) = &arguments.flash_state
            && path.exists()
        {
            fs::read(path)?
        } else {
            flash.clone()
        };
        let limits = RunLimits {
            instructions: Some(arguments.max_instructions),
            deadline: arguments.deadline.map(SimTime::from_ticks),
        };
        let result = match target {
            TargetId::Esp32c6 => {
                let mut machine = RiscVMachine::new(target)?;
                machine.set_esp_flash_image(&flash_state);
                machine.load_esp_application(&image)?;
                if let Some(payload) = &usb_input {
                    machine.queue_usb_input(payload);
                }
                machine.stop_on_usb_input_complete(stop_on_usb_input_complete);
                machine.set_access_recording(arguments.bus_log.is_some());
                let result = if let Some(path) = &arguments.vcd {
                    let output = File::create(path)?;
                    let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
                    machine.run_with_stimuli(limits, &stimuli, Some(&mut writer))?
                } else {
                    machine.run_with_stimuli(limits, &stimuli, None)?
                };
                write_access_log(arguments.bus_log.as_deref(), machine.access_log())?;
                write_flash_state(arguments.flash_state.as_deref(), machine.esp_flash_image())?;
                result
            }
            TargetId::Esp32s3 => {
                let mut machine = XtensaMachine::new(target)?;
                machine.set_esp_flash_image(&flash_state);
                machine.load_esp_application(&image)?;
                if let Some(payload) = &usb_input {
                    machine.queue_usb_input(payload);
                }
                machine.stop_on_usb_input_complete(stop_on_usb_input_complete);
                machine.set_access_recording(arguments.bus_log.is_some());
                let result = if let Some(path) = &arguments.vcd {
                    let output = File::create(path)?;
                    let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
                    machine.run_with_stimuli(limits, &stimuli, Some(&mut writer))?
                } else {
                    machine.run_with_stimuli(limits, &stimuli, None)?
                };
                write_access_log(arguments.bus_log.as_deref(), machine.access_log())?;
                write_flash_state(arguments.flash_state.as_deref(), machine.esp_flash_image())?;
                result
            }
            _ => unreachable!(),
        };
        return write_run_result(&result, arguments.result.as_deref());
    }
    let image = Uf2Image::parse(&bytes)?;
    if arguments.esp_base_image.is_some() {
        return Err("--esp-base-image is only valid for ESP targets".into());
    }
    if matches!(arguments.cpu, FirmwareCpuArg::Riscv) {
        if arguments.boot_rom.is_some() {
            return Err(
                "--boot-rom is not supported by the RP2350 RISC-V functional handoff".into(),
            );
        }
        let mut machine = RiscVMachine::new(target)?;
        if let Some(path) = &arguments.flash_state
            && path.exists()
        {
            machine.set_rp_flash_image(&fs::read(path)?)?;
        }
        machine.load_rp2350_riscv_uf2(&image)?;
        if let Some(payload) = &usb_input {
            machine.queue_usb_input(payload);
        }
        machine.stop_on_usb_input_complete(stop_on_usb_input_complete);
        machine.set_access_recording(arguments.bus_log.is_some());
        let limits = RunLimits {
            instructions: Some(arguments.max_instructions),
            deadline: arguments.deadline.map(SimTime::from_ticks),
        };
        let result = if let Some(path) = &arguments.vcd {
            let output = File::create(path)?;
            let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
            machine.run_with_stimuli(limits, &stimuli, Some(&mut writer))?
        } else {
            machine.run_with_stimuli(limits, &stimuli, None)?
        };
        if let Some(path) = &arguments.bus_log {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, serde_json::to_vec_pretty(machine.access_log())?)?;
        }
        let flash = machine.rp_flash_image()?;
        write_flash_state(arguments.flash_state.as_deref(), &flash)?;
        return write_run_result(&result, arguments.result.as_deref());
    }
    let mut machine = ArmMachine::new(target)?;
    if let Some(path) = &arguments.boot_rom {
        machine.load_rp2040_boot_rom(&fs::read(path)?)?;
    }
    if let Some(path) = &arguments.flash_state
        && path.exists()
    {
        machine.set_flash_image(&fs::read(path)?)?;
    }
    machine.load_uf2(&image)?;
    match target {
        TargetId::Rp2040 => machine.rp2040_bootrom_handoff()?,
        TargetId::Rp2350 => machine.rp2350_arm_bootrom_handoff()?,
        _ => {
            return Err(format!(
                "official UF2 boot boundary is not implemented for target {target}"
            )
            .into());
        }
    }
    if let Some(payload) = &usb_input {
        machine.queue_usb_input(payload);
    }
    machine.stop_on_usb_input_complete(stop_on_usb_input_complete);
    machine.set_access_recording(arguments.bus_log.is_some());
    let limits = RunLimits {
        instructions: Some(arguments.max_instructions),
        deadline: arguments.deadline.map(SimTime::from_ticks),
    };
    let result = if let Some(path) = &arguments.vcd {
        let output = File::create(path)?;
        let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
        machine.run_with_stimuli(limits, &stimuli, Some(&mut writer))?
    } else {
        machine.run_with_stimuli(limits, &stimuli, None)?
    };
    if let Some(path) = &arguments.bus_log {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(machine.access_log())?)?;
    }
    write_flash_state(arguments.flash_state.as_deref(), &machine.flash_image())?;
    write_run_result(&result, arguments.result.as_deref())
}

fn write_flash_state(path: Option<&Path>, flash: &[u8]) -> Result<(), Box<dyn Error>> {
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, flash)?;
    }
    Ok(())
}

fn write_access_log(
    path: Option<&Path>,
    accesses: &[renvo_bus::BusAccessRecord],
) -> Result<(), Box<dyn Error>> {
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(accesses)?)?;
    }
    Ok(())
}

fn write_run_result(result: &RunResult, path: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let json = serde_json::to_vec_pretty(result)?;
    if let Some(path) = path {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)?;
    } else {
        println!("{}", String::from_utf8(json)?);
    }
    Ok(())
}

fn summarize_esp_executable(image: &renvo_image::EspExecutableImage) -> EspExecutableSummary {
    EspExecutableSummary {
        flash_offset: image.flash_offset,
        entry: image.header.entry,
        chip_id: image.header.chip_id,
        segment_count: image.header.segment_count,
        checksum: image.checksum,
        appended_sha256: image.appended_sha256.clone(),
        end_offset: image.end_offset,
        segments: image
            .segments
            .iter()
            .map(|segment| ImageSegmentSummary {
                address: segment.address,
                size: segment.data.len(),
                flash_offset: Some(segment.flash_offset),
                not_main_flash: None,
            })
            .collect(),
    }
}

fn list_targets(json: bool) -> Result<(), Box<dyn Error>> {
    if json {
        println!("{}", serde_json::to_string_pretty(target_manifests())?);
        return Ok(());
    }
    println!("TARGET     CPU MODE(S)                         FIDELITY");
    for target in target_manifests() {
        let cpus = target
            .cpus
            .iter()
            .map(|cpu| cpu.name)
            .collect::<Vec<_>>()
            .join(", ");
        println!("{:<10} {:<35} {:?}", target.id, cpus, target.fidelity);
    }
    Ok(())
}

fn inspect(path: &PathBuf) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let image = FirmwareImage::parse(&bytes)?;
    println!("{}", serde_json::to_string_pretty(&image)?);
    Ok(())
}

fn run(arguments: &RunArgs) -> Result<(), Box<dyn Error>> {
    let target = arguments.target.parse::<TargetId>()?;
    let bytes = fs::read(&arguments.elf)?;
    let image = FirmwareImage::parse(&bytes)?;
    let limits = RunLimits {
        instructions: Some(arguments.max_instructions),
        deadline: arguments.deadline.map(SimTime::from_ticks),
    };
    let stimuli = arguments
        .pins
        .iter()
        .map(|value| parse_stimulus(value))
        .collect::<Result<Vec<_>, _>>()?;
    let (result, accesses) = if let Some(path) = &arguments.vcd {
        let output = File::create(path)?;
        let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
        run_loaded_recorded(
            target,
            &image,
            limits,
            &stimuli,
            Some(&mut writer),
            DirectRunControl {
                record_accesses: arguments.bus_log.is_some() || arguments.coverage.is_some(),
                breakpoints: &arguments.breakpoint,
                watchpoints: &arguments.watchpoint,
                signal_stops: &arguments.signal_stops,
            },
        )?
    } else {
        run_loaded_recorded(
            target,
            &image,
            limits,
            &stimuli,
            None,
            DirectRunControl {
                record_accesses: arguments.bus_log.is_some() || arguments.coverage.is_some(),
                breakpoints: &arguments.breakpoint,
                watchpoints: &arguments.watchpoint,
                signal_stops: &arguments.signal_stops,
            },
        )?
    };
    write_access_log(arguments.bus_log.as_deref(), &accesses)?;
    write_coverage(arguments.coverage.as_deref(), target, &image, &accesses)?;
    let json = serde_json::to_vec_pretty(&result)?;
    if let Some(path) = &arguments.replay {
        let expected: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        let actual: serde_json::Value = serde_json::from_slice(&json)?;
        if actual != expected {
            return Err(format!("deterministic replay diverged from {}", path.display()).into());
        }
    }
    if let Some(path) = &arguments.result {
        fs::write(path, json)?;
    } else {
        println!("{}", String::from_utf8(json)?);
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct CoverageAddress {
    address: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    symbol_offset: Option<u64>,
}

#[derive(Debug, Serialize)]
struct CoverageArtifact {
    schema: &'static str,
    target: TargetId,
    architecture: FirmwareArchitecture,
    fetch_accesses: u64,
    unique_addresses: usize,
    addresses: Vec<CoverageAddress>,
    digest: String,
}

fn write_coverage(
    path: Option<&Path>,
    target: TargetId,
    image: &FirmwareImage,
    accesses: &[renvo_bus::BusAccessRecord],
) -> Result<(), Box<dyn Error>> {
    let Some(path) = path else {
        return Ok(());
    };
    let fetches = accesses
        .iter()
        .filter(|access| access.kind == AccessKind::Execute)
        .collect::<Vec<_>>();
    let addresses = fetches
        .iter()
        .map(|access| access.address)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|address| {
            let (symbol, symbol_offset) = image
                .symbolicate(address)
                .map_or((None, None), |(symbol, offset)| {
                    (Some(symbol.name.clone()), Some(offset))
                });
            CoverageAddress {
                address,
                symbol,
                symbol_offset,
            }
        })
        .collect::<Vec<_>>();
    let mut digest = Sha256::new();
    for address in &addresses {
        digest.update(address.address.to_le_bytes());
    }
    let artifact = CoverageArtifact {
        schema: "renvo.execution-coverage.v1",
        target,
        architecture: image.architecture,
        fetch_accesses: fetches.len() as u64,
        unique_addresses: addresses.len(),
        addresses,
        digest: hex::encode(digest.finalize()),
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(&artifact)?)?;
    Ok(())
}

fn run_loaded(
    target: TargetId,
    image: &FirmwareImage,
    limits: RunLimits,
    stimuli: &[PinStimulus],
    trace: Option<&mut dyn TraceSink>,
) -> Result<RunResult, Box<dyn Error>> {
    Ok(run_loaded_recorded(
        target,
        image,
        limits,
        stimuli,
        trace,
        DirectRunControl::default(),
    )?
    .0)
}

fn run_loaded_recorded(
    target: TargetId,
    image: &FirmwareImage,
    limits: RunLimits,
    stimuli: &[PinStimulus],
    trace: Option<&mut dyn TraceSink>,
    control: DirectRunControl<'_>,
) -> Result<(RunResult, Vec<renvo_bus::BusAccessRecord>), Box<dyn Error>> {
    match image.architecture {
        FirmwareArchitecture::RiscV32 => {
            let mut machine = RiscVMachine::new(target)?;
            machine.load_firmware(image)?;
            for address in control.breakpoints {
                machine.add_breakpoint(*address);
            }
            for address in control.watchpoints {
                machine.add_watchpoint(*address);
            }
            for stop in control.signal_stops {
                machine.add_signal_stop(&stop.path, stop.edge)?;
            }
            machine.set_access_recording(control.record_accesses);
            let result = machine.run_with_stimuli(limits, stimuli, trace)?;
            Ok((result, machine.access_log().to_vec()))
        }
        FirmwareArchitecture::Arm => {
            let mut machine = ArmMachine::new(target)?;
            machine.load_firmware(image)?;
            for address in control.breakpoints {
                machine.add_breakpoint(*address);
            }
            for address in control.watchpoints {
                machine.add_watchpoint(*address);
            }
            for stop in control.signal_stops {
                machine.add_signal_stop(&stop.path, stop.edge)?;
            }
            machine.set_access_recording(control.record_accesses);
            let result = machine.run_with_stimuli(limits, stimuli, trace)?;
            Ok((result, machine.access_log().to_vec()))
        }
        FirmwareArchitecture::Xtensa => {
            let mut machine = XtensaMachine::new(target)?;
            machine.load_firmware(image)?;
            for address in control.breakpoints {
                machine.add_breakpoint(*address);
            }
            for address in control.watchpoints {
                machine.add_watchpoint(*address);
            }
            for stop in control.signal_stops {
                machine.add_signal_stop(&stop.path, stop.edge)?;
            }
            machine.set_access_recording(control.record_accesses);
            let result = machine.run_with_stimuli(limits, stimuli, trace)?;
            Ok((result, machine.access_log().to_vec()))
        }
    }
}

fn parse_stimulus(value: &str) -> Result<PinStimulus, Box<dyn Error>> {
    let (pin, timed_value) = value
        .split_once('=')
        .ok_or("pin stimulus must contain '='")?;
    let (logic, tick) = timed_value
        .split_once('@')
        .ok_or("pin stimulus must contain '@'")?;
    let value = match logic.to_ascii_lowercase().as_str() {
        "0" => Logic::Zero,
        "1" => Logic::One,
        "z" => Logic::Z,
        "x" => Logic::X,
        _ => return Err(format!("invalid pin logic {logic:?}").into()),
    };
    Ok(PinStimulus {
        at: SimTime::from_ticks(tick.parse()?),
        pin: pin.parse()?,
        value,
    })
}

fn parse_address(value: &str) -> Result<u64, String> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).map_err(|error| error.to_string())
    } else {
        value.parse::<u64>().map_err(|error| error.to_string())
    }
}

fn parse_signal_stop(value: &str) -> Result<SignalStopArg, String> {
    let (path, edge) = value
        .rsplit_once('=')
        .ok_or_else(|| "signal stop must use PATH=change|rising|falling".to_owned())?;
    if path.is_empty() {
        return Err("signal stop path must not be empty".to_owned());
    }
    let edge = match edge.to_ascii_lowercase().as_str() {
        "change" => SignalEdge::Change,
        "rising" => SignalEdge::Rising,
        "falling" => SignalEdge::Falling,
        _ => return Err("signal edge must be change, rising, or falling".to_owned()),
    };
    Ok(SignalStopArg {
        path: path.to_owned(),
        edge,
    })
}

fn corpus(command: CorpusCommand) -> Result<(), Box<dyn Error>> {
    let compiler = DockerCompiler::default();
    match command {
        CorpusCommand::Doctor => {
            compiler.verify_available()?;
            println!("Docker compiler boundary is available");
        }
        CorpusCommand::Matrix(arguments) => {
            let matrix_text = fs::read_to_string(arguments.matrix)?;
            let matrix: CompilerMatrix = toml::from_str(&matrix_text)?;
            fs::create_dir_all(&arguments.output)?;
            fs::create_dir_all(&arguments.artifacts)?;
            for variant in matrix.expand() {
                if !variant
                    .id
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
                {
                    return Err(format!("unsafe matrix variant ID {:?}", variant.id).into());
                }
                let output = arguments.output.join(&variant.id);
                fs::create_dir_all(&output)?;
                let request = BuildRequest {
                    toolchain: variant.toolchain,
                    source_dir: arguments.source.clone(),
                    output_dir: output,
                    arguments: variant.arguments,
                    target: arguments.target.clone(),
                    limits: DockerLimits {
                        timeout_seconds: arguments.timeout_seconds,
                        ..DockerLimits::default()
                    },
                };
                let artifact = compiler.compile(&request)?;
                artifact.write_json(&arguments.artifacts.join(format!("{}.json", variant.id)))?;
                if !artifact.succeeded() {
                    return Err(format!(
                        "matrix variant {:?} exited with status {}",
                        variant.id, artifact.exit_code
                    )
                    .into());
                }
            }
        }
        CorpusCommand::Compare(arguments) => {
            let observations = arguments
                .observations
                .iter()
                .map(|path| {
                    let value = serde_json::from_slice(&fs::read(path)?)?;
                    Ok(NamedObservation {
                        name: path.display().to_string(),
                        value,
                    })
                })
                .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
            let comparisons = compare_observations(&observations, &arguments.pointers);
            println!("{}", serde_json::to_string_pretty(&comparisons)?);
            if comparisons
                .iter()
                .any(|comparison| !comparison.equivalent())
            {
                return Err("selected observations diverged".into());
            }
        }
        CorpusCommand::Run(arguments) => run_corpus_suite(&arguments)?,
        CorpusCommand::Reduce(arguments) => reduce_corpus_case(&compiler, &arguments)?,
        CorpusCommand::Build(arguments) => {
            let spec_text = fs::read_to_string(&arguments.toolchain)?;
            let toolchain: ToolchainSpec = toml::from_str(&spec_text)?;
            let request = BuildRequest {
                toolchain,
                source_dir: arguments.source,
                output_dir: arguments.output,
                arguments: arguments.arguments,
                target: arguments.target,
                limits: DockerLimits {
                    timeout_seconds: arguments.timeout_seconds,
                    ..DockerLimits::default()
                },
            };
            if arguments.dry_run {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&compiler.command(&request)?)?
                );
            } else {
                let artifact = compiler.compile(&request)?;
                artifact.write_json(&arguments.artifact)?;
                if !artifact.succeeded() {
                    return Err(format!(
                        "container compiler exited with status {}",
                        artifact.exit_code
                    )
                    .into());
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
struct ReductionEvaluation {
    id: u64,
    candidate: ReductionCandidate,
    build: BuildArtifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<RunResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    discrepancy: bool,
}

#[derive(Debug, Serialize)]
struct CorpusReductionArtifact {
    schema: &'static str,
    target: TargetId,
    seeded_expected: u32,
    reduction: CaseReductionResult,
    evaluations: Vec<ReductionEvaluation>,
    final_repeat_evaluations: [u64; 2],
    final_reproducible: bool,
    result: &'static str,
}

fn reduce_corpus_case(
    compiler: &DockerCompiler,
    arguments: &CorpusReduceArgs,
) -> Result<(), Box<dyn Error>> {
    let target = arguments.target.parse::<TargetId>()?;
    let toolchain: ToolchainSpec = toml::from_str(&fs::read_to_string(&arguments.toolchain)?)?;
    let original = ReductionCandidate {
        source: arguments.source_items.clone(),
        flags: arguments.flag_items.clone(),
        inputs: arguments.input_items.clone(),
    };
    fs::create_dir_all(&arguments.output)?;
    let mut evaluations = Vec::new();
    let mut next_id = 0_u64;

    let mut evaluate = |candidate: &ReductionCandidate| -> Result<bool, Box<dyn Error>> {
        let id = next_id;
        next_id = next_id.saturating_add(1);
        let evaluation_root = arguments.output.join(format!("evaluation-{id:04}"));
        let source_dir = evaluation_root.join("source");
        let output_dir = evaluation_root.join("output");
        fs::create_dir_all(&source_dir)?;
        fs::create_dir_all(&output_dir)?;
        copy_directory_contents(&arguments.source, &source_dir)?;

        let mut header = String::from("/* Generated deterministic reduction candidate. */\n");
        for fragment in &candidate.source {
            header.push_str(fragment);
            header.push('\n');
        }
        header.push_str("#define RENVO_INPUT_SUM (0u");
        for input in &candidate.inputs {
            write!(header, " + {input}u")?;
        }
        header.push_str(")\n");
        fs::write(source_dir.join("candidate.h"), header)?;

        let mut compiler_arguments = arguments.arguments.clone();
        compiler_arguments.extend(candidate.flags.iter().cloned());
        let request = BuildRequest {
            toolchain: toolchain.clone(),
            source_dir,
            output_dir: output_dir.clone(),
            arguments: compiler_arguments,
            target: arguments.target.clone(),
            limits: DockerLimits::default(),
        };
        let build = compiler.compile(&request)?;
        build.write_json(&evaluation_root.join("build.json"))?;

        let mut run = None;
        let mut error = None;
        if build.succeeded() {
            match fs::read(output_dir.join("smoke.elf"))
                .map_err(|problem| problem.to_string())
                .and_then(|bytes| {
                    FirmwareImage::parse(&bytes).map_err(|problem| problem.to_string())
                })
                .and_then(|image| {
                    run_loaded(
                        target,
                        &image,
                        RunLimits {
                            instructions: Some(arguments.max_instructions),
                            deadline: None,
                        },
                        &[],
                        None,
                    )
                    .map_err(|problem| problem.to_string())
                }) {
                Ok(result) => {
                    fs::write(
                        evaluation_root.join("run.json"),
                        serde_json::to_vec_pretty(&result)?,
                    )?;
                    run = Some(result);
                }
                Err(problem) => error = Some(problem),
            }
        } else {
            error = Some(format!("compiler exited with status {}", build.exit_code));
        }
        let discrepancy = run
            .as_ref()
            .is_some_and(|result| result.exit_code != Some(arguments.seed_expected));
        evaluations.push(ReductionEvaluation {
            id,
            candidate: candidate.clone(),
            build,
            run,
            error,
            discrepancy,
        });
        Ok(discrepancy)
    };

    if !evaluate(&original)? {
        return Err("initial reduction case does not reproduce the seeded discrepancy".into());
    }
    let reduction = reduce_case(original, &mut evaluate)?;
    if !evaluate(&reduction.minimized)? || !evaluate(&reduction.minimized)? {
        return Err("minimized discrepancy did not reproduce twice".into());
    }
    let second = evaluations.last().expect("repeat evaluation exists");
    let first = &evaluations[evaluations.len() - 2];
    let final_reproducible = first.run == second.run
        && first.build.inputs == second.build.inputs
        && first.build.outputs == second.build.outputs;
    if !final_reproducible {
        return Err("minimized build or run artifacts are not reproducible".into());
    }
    let final_repeat_evaluations = [first.id, second.id];
    let artifact = CorpusReductionArtifact {
        schema: "renvo.corpus-reduction.v1",
        target,
        seeded_expected: arguments.seed_expected,
        reduction,
        evaluations,
        final_repeat_evaluations,
        final_reproducible,
        result: "pass",
    };
    fs::write(&arguments.artifact, serde_json::to_vec_pretty(&artifact)?)?;
    println!(
        "reduced seeded discrepancy for {target}; artifact: {}",
        arguments.artifact.display()
    );
    Ok(())
}

fn copy_directory_contents(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let output = destination.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir_all(&output)?;
            copy_directory_contents(&entry.path(), &output)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), output)?;
        } else {
            return Err(format!(
                "reduction source contains unsupported entry {}",
                entry.path().display()
            )
            .into());
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ExpectedCase {
    name: String,
    category: String,
    signature: String,
    expected: u32,
    inspiration: String,
}

#[derive(Debug, Serialize)]
struct SuiteFailure {
    case_id: String,
    name: String,
    category: String,
    signature: String,
    inspiration: String,
    expected: u32,
    actual: Option<u32>,
    reason: String,
    instructions: Option<u64>,
}

#[derive(Debug, Serialize)]
struct SuiteArtifact {
    schema: &'static str,
    target: TargetId,
    input: String,
    manifest: String,
    total: usize,
    passed: usize,
    failed: usize,
    failures: Vec<SuiteFailure>,
}

fn run_corpus_suite(arguments: &CorpusRunArgs) -> Result<(), Box<dyn Error>> {
    let target = arguments.target.parse::<TargetId>()?;
    let expected = read_expected_manifest(&arguments.manifest)?;
    let mut failures = Vec::new();

    for (case_id, case) in &expected {
        let path = arguments.input.join(format!("{case_id}.elf"));
        let outcome = fs::read(&path)
            .map_err(|error| format!("{}: {error}", path.display()))
            .and_then(|bytes| {
                FirmwareImage::parse(&bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|image| {
                        run_loaded(
                            target,
                            &image,
                            RunLimits {
                                instructions: Some(arguments.max_instructions),
                                deadline: None,
                            },
                            &[],
                            None,
                        )
                        .map_err(|error| error.to_string())
                    })
            });
        match outcome {
            Ok(result) if result.exit_code == Some(case.expected) => {}
            Ok(result) => failures.push(SuiteFailure {
                case_id: case_id.clone(),
                name: case.name.clone(),
                category: case.category.clone(),
                signature: case.signature.clone(),
                inspiration: case.inspiration.clone(),
                expected: case.expected,
                actual: result.exit_code,
                reason: format!("{:?}", result.reason),
                instructions: Some(result.stats.instructions),
            }),
            Err(error) => failures.push(SuiteFailure {
                case_id: case_id.clone(),
                name: case.name.clone(),
                category: case.category.clone(),
                signature: case.signature.clone(),
                inspiration: case.inspiration.clone(),
                expected: case.expected,
                actual: None,
                reason: error,
                instructions: None,
            }),
        }
    }

    let total = expected.len();
    let artifact = SuiteArtifact {
        schema: "renvo.corpus-suite.v1",
        target,
        input: normalized_display_path(&arguments.input),
        manifest: normalized_display_path(&arguments.manifest),
        total,
        passed: total - failures.len(),
        failed: failures.len(),
        failures,
    };
    if let Some(parent) = arguments.artifact.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&arguments.artifact, serde_json::to_vec_pretty(&artifact)?)?;
    println!(
        "{}: {}/{} cases passed; artifact: {}",
        target,
        artifact.passed,
        artifact.total,
        arguments.artifact.display()
    );
    if artifact.failed != 0 {
        return Err(format!("{} corpus cases failed", artifact.failed).into());
    }
    Ok(())
}

fn read_expected_manifest(path: &Path) -> Result<BTreeMap<String, ExpectedCase>, Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let mut expected = BTreeMap::new();
    for (line_number, line) in contents.lines().enumerate() {
        if line_number == 0
            && line == "case_id\tname\tcategory\tsignature\texpected_hex\tinspiration"
        {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 6 {
            return Err(format!(
                "{}:{}: expected six TSV fields",
                path.display(),
                line_number + 1
            )
            .into());
        }
        let value = fields[4]
            .strip_prefix("0x")
            .ok_or_else(|| format!("{}:{}: expected 0x result", path.display(), line_number + 1))?;
        let parsed = u32::from_str_radix(value, 16)?;
        if expected
            .insert(
                fields[0].to_owned(),
                ExpectedCase {
                    name: fields[1].to_owned(),
                    category: fields[2].to_owned(),
                    signature: fields[3].to_owned(),
                    expected: parsed,
                    inspiration: fields[5].to_owned(),
                },
            )
            .is_some()
        {
            return Err(format!("duplicate case ID {:?}", fields[0]).into());
        }
    }
    if expected.len() != 1_000 {
        return Err(format!("expected exactly 1000 cases, found {}", expected.len()).into());
    }
    Ok(expected)
}

fn normalized_display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
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
}
