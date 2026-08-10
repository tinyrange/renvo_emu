use super::*;
use crate::access_output::{AccessSummary, DirectAccessOutput};

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
    let esp_boot_rom = if let Some(path) = &arguments.boot_rom {
        let bytes = fs::read(path)?;
        remu_machines::verify_esp_radio_rom(target, &bytes)?;
        Some(FirmwareImage::parse_addressed_sections(&bytes)?)
    } else {
        None
    };
    if esp_boot_rom.is_some() && !matches!(target, TargetId::Esp32c6 | TargetId::Esp32s3) {
        return Err("--boot-rom is supported only with ESP32-C6/ESP32-S3 direct execution".into());
    }
    let uses_native_esp_image = arguments.esp_app_image.is_some();
    let uses_radio = arguments.radio_input.is_some() || arguments.radio_replay.is_some();
    if matches!(target, TargetId::Esp32c6 | TargetId::Esp32s3)
        && (uses_native_esp_image || uses_radio)
        && esp_boot_rom.is_none()
    {
        return Err(format!(
            "{target} native/radio execution requires --boot-rom with the matching real mask-ROM image"
        )
        .into());
    }
    if arguments.radio_replay.is_some() && !matches!(target, TargetId::Esp32c6 | TargetId::Esp32s3)
    {
        return Err("--radio-replay is supported only with ESP32-C6/ESP32-S3 execution".into());
    }
    if arguments.radio_input.is_some() && !matches!(target, TargetId::Esp32c6 | TargetId::Esp32s3) {
        return Err("--radio-input is supported only with ESP32-C6/ESP32-S3 execution".into());
    }
    let (esp32c6_mmu_page_size, esp32c6_flash_image, esp32c6_boot_image, esp32s3_boot_image) =
        validate_esp_boot_image(arguments, target, &image)?;
    let limits = RunLimits {
        instructions: Some(arguments.max_instructions),
        deadline: arguments.deadline.map(SimTime::from_ticks),
    };
    let stimuli = arguments
        .pins
        .iter()
        .map(|value| parse_stimulus(value))
        .collect::<Result<Vec<_>, _>>()?;
    let access_output = DirectAccessOutput::new(
        arguments.bus_log.as_deref(),
        &arguments.bus_log_region,
        arguments.coverage.is_some(),
    )?;
    let observer = access_output.observer();
    let result = if let Some(path) = &arguments.vcd {
        let output = File::create(path)?;
        let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
        run_loaded_recorded(
            target,
            &image,
            limits,
            &stimuli,
            Some(&mut writer),
            DirectRunControl {
                access_observer: observer.clone(),
                esp32c6_mmu_page_size,
                esp32c6_flash_image: esp32c6_flash_image.clone(),
                esp32c6_boot_image: esp32c6_boot_image.clone(),
                esp32s3_boot_image: esp32s3_boot_image.clone(),
                esp_boot_rom: esp_boot_rom.clone(),
                radio_replay: arguments.radio_replay.as_deref(),
                radio_input: arguments.radio_input.as_deref(),
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
                access_observer: observer,
                esp32c6_mmu_page_size,
                esp32c6_flash_image,
                esp32c6_boot_image,
                esp32s3_boot_image,
                esp_boot_rom,
                radio_replay: arguments.radio_replay.as_deref(),
                radio_input: arguments.radio_input.as_deref(),
                breakpoints: &arguments.breakpoint,
                watchpoints: &arguments.watchpoint,
                signal_stops: &arguments.signal_stops,
            },
        )?
    };
    let access_summary = access_output.finish()?;
    write_coverage(
        arguments.coverage.as_deref(),
        target,
        image.architecture,
        Some(&image),
        access_summary,
    )?;
    write_direct_result(arguments, &result)
}

fn validate_esp_boot_image(
    arguments: &RunArgs,
    target: TargetId,
    elf: &FirmwareImage,
) -> Result<
    (
        Option<u32>,
        Option<Vec<u8>>,
        Option<(EspExecutableImage, u32)>,
        Option<(EspFlashImage, Vec<u8>)>,
    ),
    Box<dyn Error>,
> {
    let Some(path) = &arguments.esp_app_image else {
        if target == TargetId::Esp32c6 {
            eprintln!(
                "warning: direct ELF execution does not prove ESP32-C6 flash bootability; \
                 pass --esp-app-image to validate an esptool application image"
            );
        }
        return Ok((None, None, None, None));
    };
    let bytes = fs::read(path)?;
    if target == TargetId::Esp32s3 {
        let flash = EspFlashImage::parse(&bytes).map_err(|error| {
            format!("ESP32-S3 --esp-app-image must be a merged flash image: {error}")
        })?;
        return Ok((None, None, None, Some((flash, bytes))));
    }
    if target != TargetId::Esp32c6 {
        return Err("--esp-app-image is supported only with ESP32-C6/ESP32-S3".into());
    }
    let (application, partition_offset, flash_image) = match EspFlashImage::parse(&bytes) {
        Ok(flash) => (
            flash.application,
            flash.application_partition.offset,
            Some(bytes),
        ),
        Err(_) => {
            let application = EspExecutableImage::parse(&bytes)?;
            let partition_offset = u32::try_from(arguments.esp_app_offset.unwrap_or(0x1_0000))
                .map_err(|_| "--esp-app-offset must fit in 32 bits")?;
            (application, partition_offset, None)
        }
    };
    RiscVMachine::validate_esp32c6_boot_image(elf, &application, partition_offset)?;
    Ok((
        Some(RiscVMachine::esp32c6_image_mmu_page_size(&application)?),
        flash_image,
        Some((application, partition_offset)),
        None,
    ))
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
    let access_output = DirectAccessOutput::new(
        arguments.bus_log.as_deref(),
        &arguments.bus_log_region,
        arguments.coverage.is_some(),
    )?;
    let control = DirectRunControl {
        access_observer: access_output.observer(),
        esp32c6_mmu_page_size: None,
        esp32c6_flash_image: None,
        esp32c6_boot_image: None,
        esp32s3_boot_image: None,
        esp_boot_rom: None,
        radio_replay: None,
        radio_input: None,
        breakpoints: &arguments.breakpoint,
        watchpoints: &arguments.watchpoint,
        signal_stops: &arguments.signal_stops,
    };
    let (result, architecture) = match target {
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
            (result, FirmwareArchitecture::Pic16Enhanced)
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
            (result, FirmwareArchitecture::Mcs51)
        }
        _ => unreachable!("target validity checked above"),
    };
    let access_summary = access_output.finish()?;
    write_coverage(
        arguments.coverage.as_deref(),
        target,
        architecture,
        None,
        access_summary,
    )?;
    write_direct_result(arguments, &result)
}

pub(crate) fn run_mcs51_program_recorded(
    target: TargetId,
    image: &IntelHexImage,
    limits: RunLimits,
    stimuli: &[PinStimulus],
    trace: Option<&mut dyn TraceSink>,
    control: DirectRunControl<'_>,
) -> Result<RunResult, Box<dyn Error>> {
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
    machine.set_access_observer(control.access_observer);
    let result = machine.run_with_stimuli(limits, stimuli, trace)?;
    Ok(result)
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
    accesses: AccessSummary,
) -> Result<(), Box<dyn Error>> {
    let Some(path) = path else {
        return Ok(());
    };
    let addresses = accesses
        .execute_addresses
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
        schema: "remu.execution-coverage.v1",
        target,
        architecture,
        fetch_accesses: accesses.fetch_accesses,
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

pub(crate) fn run_pic_program_recorded(
    target: TargetId,
    image: &remu_image::ProgramWordImage,
    limits: RunLimits,
    stimuli: &[PinStimulus],
    trace: Option<&mut dyn TraceSink>,
    control: DirectRunControl<'_>,
) -> Result<RunResult, Box<dyn Error>> {
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
    machine.set_access_observer(control.access_observer);
    let result = machine.run_with_stimuli(limits, stimuli, trace)?;
    Ok(result)
}

pub(super) fn run_loaded(
    target: TargetId,
    image: &FirmwareImage,
    limits: RunLimits,
    stimuli: &[PinStimulus],
    trace: Option<&mut dyn TraceSink>,
) -> Result<RunResult, Box<dyn Error>> {
    run_loaded_recorded(
        target,
        image,
        limits,
        stimuli,
        trace,
        DirectRunControl::default(),
    )
}

pub(crate) fn run_loaded_recorded(
    target: TargetId,
    image: &FirmwareImage,
    limits: RunLimits,
    stimuli: &[PinStimulus],
    trace: Option<&mut dyn TraceSink>,
    control: DirectRunControl<'_>,
) -> Result<RunResult, Box<dyn Error>> {
    match image.architecture {
        FirmwareArchitecture::RiscV32 => {
            let mut machine = RiscVMachine::new(target)?;
            machine.load_firmware(image)?;
            // Direct handoff setup supplies compatibility data for the
            // functional ROM. In low-level mode, load the real ROM second so
            // its immutable code and ROM-owned interface data are authoritative.
            if let Some(boot_rom) = &control.esp_boot_rom {
                machine.load_boot_rom(boot_rom)?;
            }
            if let Some(page_size) = control.esp32c6_mmu_page_size {
                machine.configure_esp32c6_mmu_page_size(page_size)?;
            }
            if let Some(flash_image) = &control.esp32c6_flash_image {
                machine.set_esp_flash_image(flash_image);
            }
            if let Some((application, partition_offset)) = &control.esp32c6_boot_image {
                machine.configure_esp32c6_boot_mappings(application, *partition_offset)?;
            }
            for address in control.breakpoints {
                machine.add_breakpoint(*address);
            }
            for address in control.watchpoints {
                machine.add_watchpoint(*address);
            }
            for stop in control.signal_stops {
                machine.add_signal_stop(&stop.path, stop.edge)?;
            }
            for frame in read_radio_input(control.radio_input)? {
                machine.inject_radio_frame_at(
                    SimTime::from_ticks(frame.at),
                    frame.protocol.into(),
                    remu_radio::Spectrum::new(frame.center_khz, frame.bandwidth_khz),
                    frame.phy,
                    frame.bytes,
                    frame.power_dbm,
                )?;
            }
            machine.set_access_observer(control.access_observer);
            let result = machine.run_with_stimuli(limits, stimuli, trace)?;
            if let Some(path) = control.radio_replay {
                let artifact = machine
                    .radio_replay_artifact()
                    .ok_or("ESP32-C6 radio replay artifact is unavailable")?;
                write_radio_replay(path, &artifact)?;
            }
            Ok(result)
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
                machine.set_access_observer(control.access_observer);
                let result = machine.run_with_stimuli(limits, stimuli, trace)?;
                Ok(result)
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
                machine.set_access_observer(control.access_observer);
                let result = machine.run_with_stimuli(limits, stimuli, trace)?;
                Ok(result)
            }
        }
        FirmwareArchitecture::Xtensa => {
            let mut machine = XtensaMachine::new(target)?;
            if let Some(boot_rom) = &control.esp_boot_rom {
                machine.load_boot_rom(boot_rom)?;
            }
            machine.load_firmware(image)?;
            if let Some((flash, bytes)) = &control.esp32s3_boot_image {
                machine.set_esp_flash_image(bytes);
                machine.load_esp_application(flash)?;
            }
            for address in control.breakpoints {
                machine.add_breakpoint(*address);
            }
            for address in control.watchpoints {
                machine.add_watchpoint(*address);
            }
            for stop in control.signal_stops {
                machine.add_signal_stop(&stop.path, stop.edge)?;
            }
            for frame in read_radio_input(control.radio_input)? {
                machine.inject_radio_frame_at(
                    SimTime::from_ticks(frame.at),
                    frame.protocol.into(),
                    remu_radio::Spectrum::new(frame.center_khz, frame.bandwidth_khz),
                    frame.phy,
                    frame.bytes,
                    frame.power_dbm,
                )?;
            }
            machine.set_access_observer(control.access_observer);
            let result = machine.run_with_stimuli(limits, stimuli, trace)?;
            if let Some(path) = control.radio_replay {
                write_radio_replay(path, &machine.radio_replay_artifact())?;
            }
            Ok(result)
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
            machine.set_access_observer(control.access_observer);
            let result = machine.run_with_stimuli(limits, stimuli, trace)?;
            Ok(result)
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
            machine.set_access_observer(control.access_observer);
            let result = machine.run_with_stimuli(limits, stimuli, trace)?;
            Ok(result)
        }
        FirmwareArchitecture::Pic16Enhanced | FirmwareArchitecture::Mcs51 => Err(format!(
            "architecture {:?} does not have a runnable machine yet",
            image.architecture
        )
        .into()),
    }
}

fn write_radio_replay(path: &Path, artifact: &impl Serialize) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(artifact)?)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum DirectRadioProtocol {
    Wifi,
    BluetoothLe,
    Ieee802154,
}

impl From<DirectRadioProtocol> for remu_radio::RadioProtocol {
    fn from(value: DirectRadioProtocol) -> Self {
        match value {
            DirectRadioProtocol::Wifi => Self::Wifi,
            DirectRadioProtocol::BluetoothLe => Self::BluetoothLe,
            DirectRadioProtocol::Ieee802154 => Self::Ieee802154,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectRadioFrame {
    at: u64,
    protocol: DirectRadioProtocol,
    center_khz: u32,
    bandwidth_khz: u32,
    phy: String,
    bytes: Vec<u8>,
    power_dbm: i16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectRadioInput {
    schema: String,
    frames: Vec<DirectRadioFrame>,
}

fn read_radio_input(path: Option<&Path>) -> Result<Vec<DirectRadioFrame>, Box<dyn Error>> {
    let Some(path) = path else {
        return Ok(Vec::new());
    };
    let input: DirectRadioInput = serde_json::from_slice(&fs::read(path)?)?;
    if input.schema != "remu.radio-input.v1" {
        return Err(format!(
            "unsupported radio-input schema {:?} in {}",
            input.schema,
            path.display()
        )
        .into());
    }
    Ok(input.frames)
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
