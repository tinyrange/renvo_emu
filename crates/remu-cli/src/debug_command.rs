use super::*;

pub(super) fn script(arguments: &ScriptArgs) -> Result<(), Box<dyn Error>> {
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
        schema: "remu.starlark-assertion.v1",
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
    ArmMcu(Box<ArmMcuMachine>),
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
                if matches!(
                    target,
                    TargetId::Atsamd21e18 | TargetId::Stm32l432kc | TargetId::R7fa4m1ab3cfm
                ) {
                    let mut machine = ArmMcuMachine::new(target)?;
                    machine.load_firmware(image)?;
                    Ok(Self::ArmMcu(Box::new(machine)))
                } else {
                    let mut machine = ArmMachine::new(target)?;
                    machine.load_firmware(image)?;
                    Ok(Self::Arm(Box::new(machine)))
                }
            }
            FirmwareArchitecture::Xtensa => {
                let mut machine = XtensaMachine::new(target)?;
                machine.load_firmware(image)?;
                Ok(Self::Xtensa(Box::new(machine)))
            }
            FirmwareArchitecture::Avr8
            | FirmwareArchitecture::Msp430X
            | FirmwareArchitecture::Pic16Enhanced
            | FirmwareArchitecture::Mcs51 => Err(format!(
                "architecture {:?} does not have a runnable machine yet",
                image.architecture
            )
            .into()),
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
            Self::ArmMcu(machine) => machine.run(limits, None).map_err(|error| error.to_string()),
            Self::Xtensa(machine) => machine.run(limits, None).map_err(|error| error.to_string()),
        }
    }
}

impl DebugTarget for CliDebugMachine {
    fn architecture(&self) -> DebugArchitecture {
        match self {
            Self::RiscV(_) => DebugArchitecture::RiscV32,
            Self::Arm(_) | Self::ArmMcu(_) => DebugArchitecture::Arm,
            Self::Xtensa(_) => DebugArchitecture::Xtensa,
        }
    }

    fn snapshot(&self) -> CpuSnapshot {
        match self {
            Self::RiscV(machine) => machine.debug_snapshot(),
            Self::Arm(machine) => machine.debug_snapshot(),
            Self::ArmMcu(machine) => machine.debug_snapshot(),
            Self::Xtensa(machine) => machine.debug_snapshot(),
        }
    }

    fn read_memory(&mut self, address: u64, length: usize) -> Result<Vec<u8>, String> {
        match self {
            Self::RiscV(machine) => machine.debug_read_memory(address, length),
            Self::Arm(machine) => machine.debug_read_memory(address, length),
            Self::ArmMcu(machine) => machine.debug_read_memory(address, length),
            Self::Xtensa(machine) => machine.debug_read_memory(address, length),
        }
    }

    fn write_memory(&mut self, address: u64, bytes: &[u8]) -> Result<(), String> {
        match self {
            Self::RiscV(machine) => machine.debug_write_memory(address, bytes),
            Self::Arm(machine) => machine.debug_write_memory(address, bytes),
            Self::ArmMcu(machine) => machine.debug_write_memory(address, bytes),
            Self::Xtensa(machine) => machine.debug_write_memory(address, bytes),
        }
    }

    fn add_breakpoint(&mut self, address: u64) {
        match self {
            Self::RiscV(machine) => machine.add_breakpoint(address),
            Self::Arm(machine) => machine.add_breakpoint(address),
            Self::ArmMcu(machine) => machine.add_breakpoint(address),
            Self::Xtensa(machine) => machine.add_breakpoint(address),
        }
    }

    fn remove_breakpoint(&mut self, address: u64) {
        match self {
            Self::RiscV(machine) => machine.remove_breakpoint(address),
            Self::Arm(machine) => machine.remove_breakpoint(address),
            Self::ArmMcu(machine) => machine.remove_breakpoint(address),
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

pub(super) fn gdb(arguments: &GdbArgs) -> Result<(), Box<dyn Error>> {
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
                schema: "remu.gdb-ready.v1",
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
        schema: "remu.gdb-session.v1",
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
