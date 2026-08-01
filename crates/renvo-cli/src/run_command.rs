use super::*;

pub(super) fn list_targets(json: bool) -> Result<(), Box<dyn Error>> {
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

pub(super) fn inspect(path: &PathBuf) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let image = FirmwareImage::parse(&bytes)?;
    println!("{}", serde_json::to_string_pretty(&image)?);
    Ok(())
}

pub(super) fn run(arguments: &RunArgs) -> Result<(), Box<dyn Error>> {
    let target = arguments.target.parse::<TargetId>()?;
    if let Some(path) = &arguments.hex {
        return run_hex(arguments, target, path);
    }
    let elf = arguments
        .elf
        .as_ref()
        .ok_or("one of --elf or --hex is required")?;
    let bytes = fs::read(elf)?;
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
    write_coverage(
        arguments.coverage.as_deref(),
        target,
        FirmwareArchitecture::from(image.architecture),
        Some(&image),
        &accesses,
    )?;
    write_direct_result(arguments, &result)
}

fn run_hex(arguments: &RunArgs, target: TargetId, path: &Path) -> Result<(), Box<dyn Error>> {
    if !matches!(target, TargetId::Pic16f15376 | TargetId::Efm8bb52f32g) {
        return Err(
            format!("Intel HEX direct execution is not yet valid for target {target}").into(),
        );
    }
    let image = IntelHexImage::parse(&fs::read(path)?)?;
    let limits = RunLimits {
        instructions: Some(arguments.max_instructions),
        deadline: arguments.deadline.map(SimTime::from_ticks),
    };
    let stimuli = arguments
        .pins
        .iter()
        .map(|value| parse_stimulus(value))
        .collect::<Result<Vec<_>, _>>()?;
    let control = DirectRunControl {
        record_accesses: arguments.bus_log.is_some() || arguments.coverage.is_some(),
        breakpoints: &arguments.breakpoint,
        watchpoints: &arguments.watchpoint,
        signal_stops: &arguments.signal_stops,
    };
    let (result, accesses, architecture) = match target {
        TargetId::Pic16f15376 => {
            let program = image.program_words(14, ProgramWordEndianness::Little)?;
            let output = arguments.vcd.as_ref().map(File::create).transpose()?;
            let result = if let Some(output) = output {
                let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
                run_pic_program_recorded(
                    target,
                    &program,
                    limits,
                    &stimuli,
                    Some(&mut writer),
                    control,
                )?
            } else {
                run_pic_program_recorded(target, &program, limits, &stimuli, None, control)?
            };
            (result.0, result.1, FirmwareArchitecture::Pic16Enhanced)
        }
        TargetId::Efm8bb52f32g => {
            let output = arguments.vcd.as_ref().map(File::create).transpose()?;
            let result = if let Some(output) = output {
                let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
                run_mcs51_program_recorded(
                    target,
                    &image,
                    limits,
                    &stimuli,
                    Some(&mut writer),
                    control,
                )?
            } else {
                run_mcs51_program_recorded(target, &image, limits, &stimuli, None, control)?
            };
            (result.0, result.1, FirmwareArchitecture::Mcs51)
        }
        _ => unreachable!("target validity checked above"),
    };
    write_access_log(arguments.bus_log.as_deref(), &accesses)?;
    write_coverage(
        arguments.coverage.as_deref(),
        target,
        architecture,
        None,
        &accesses,
    )?;
    write_direct_result(arguments, &result)
}

fn run_mcs51_program_recorded(
    target: TargetId,
    image: &IntelHexImage,
    limits: RunLimits,
    stimuli: &[PinStimulus],
    trace: Option<&mut dyn TraceSink>,
    control: DirectRunControl<'_>,
) -> Result<(RunResult, Vec<renvo_bus::BusAccessRecord>), Box<dyn Error>> {
    let mut machine = Mcs51McuMachine::new(target)?;
    machine.load_program(image)?;
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
    Ok((result, machine.access_log()))
}

fn write_direct_result(arguments: &RunArgs, result: &RunResult) -> Result<(), Box<dyn Error>> {
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
    architecture: FirmwareArchitecture,
    image: Option<&FirmwareImage>,
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
                .and_then(|image| image.symbolicate(address))
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
        architecture,
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

fn run_pic_program_recorded(
    target: TargetId,
    image: &renvo_image::ProgramWordImage,
    limits: RunLimits,
    stimuli: &[PinStimulus],
    trace: Option<&mut dyn TraceSink>,
    control: DirectRunControl<'_>,
) -> Result<(RunResult, Vec<renvo_bus::BusAccessRecord>), Box<dyn Error>> {
    let mut machine = Pic16McuMachine::new(target)?;
    machine.load_program(image)?;
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
    Ok((result, machine.access_log()))
}

pub(super) fn run_loaded(
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
            if matches!(
                target,
                TargetId::Atsamd21e18 | TargetId::Stm32l432kc | TargetId::R7fa4m1ab3cfm
            ) {
                let mut machine = ArmMcuMachine::new(target)?;
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
            } else {
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
        FirmwareArchitecture::Avr8 => {
            let mut machine = AvrMcuMachine::new(target)?;
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
        FirmwareArchitecture::Msp430X => {
            let mut machine = Msp430McuMachine::new(target)?;
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
        FirmwareArchitecture::Pic16Enhanced | FirmwareArchitecture::Mcs51 => Err(format!(
            "architecture {:?} does not have a runnable machine yet",
            image.architecture
        )
        .into()),
    }
}

pub(super) fn parse_stimulus(value: &str) -> Result<PinStimulus, Box<dyn Error>> {
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

pub(super) fn parse_address(value: &str) -> Result<u64, String> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).map_err(|error| error.to_string())
    } else {
        value.parse::<u64>().map_err(|error| error.to_string())
    }
}

pub(super) fn parse_signal_stop(value: &str) -> Result<SignalStopArg, String> {
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
