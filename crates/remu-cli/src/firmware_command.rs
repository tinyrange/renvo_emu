use super::*;

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
        partitions: Vec<remu_image::EspPartition>,
        partition_table_has_md5: bool,
        application_partition: remu_image::EspPartition,
        application: EspExecutableSummary,
    },
    IntelHex {
        entry: Option<u32>,
        records: usize,
        segments: Vec<ImageSegmentSummary>,
    },
    RawBin {
        size: usize,
        sha256: String,
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

pub(super) fn firmware(command: FirmwareCommand) -> Result<(), Box<dyn Error>> {
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
                FirmwareFormatArg::Auto if bytes.first() == Some(&b':') => {
                    FirmwareFormatArg::IntelHex
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
                FirmwareFormatArg::IntelHex => {
                    let image = IntelHexImage::parse(&bytes)?;
                    FirmwareInspection::IntelHex {
                        entry: image.entry,
                        records: image.records.len(),
                        segments: image
                            .segments
                            .into_iter()
                            .map(|segment| ImageSegmentSummary {
                                address: segment.address,
                                size: segment.data.len(),
                                flash_offset: None,
                                not_main_flash: None,
                            })
                            .collect(),
                    }
                }
                FirmwareFormatArg::RawBin => FirmwareInspection::RawBin {
                    size: bytes.len(),
                    sha256: hex::encode(Sha256::digest(&bytes)),
                },
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
        const CHUNK_MARKER: &str = "# REMU_CHUNK";
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
    if !matches!(
        target,
        TargetId::Rp2040 | TargetId::Rp2350 | TargetId::Esp32c6 | TargetId::Esp32s3
    ) {
        if usb_input.is_some()
            || arguments.esp_base_image.is_some()
            || arguments.boot_rom.is_some()
            || arguments.flash_state.is_some()
        {
            return Err(
                "USB input, --esp-base-image, --boot-rom, and --flash-state are not valid for this native image target"
                    .into(),
            );
        }
        let result =
            crate::native_firmware::boot_native_image(arguments, target, &bytes, &stimuli)?;
        return write_run_result(&result, arguments.result.as_deref());
    }
    if matches!(target, TargetId::Esp32c6 | TargetId::Esp32s3) {
        match arguments.format {
            FirmwareFormatArg::IntelHex | FirmwareFormatArg::RawBin => {
                return Err(
                    "ESP targets require --format esp-bin or an ESP application UF2".into(),
                );
            }
            FirmwareFormatArg::Uf2 if !bytes.starts_with(b"UF2\n") => {
                return Err("--format uf2 does not match the supplied ESP artifact".into());
            }
            FirmwareFormatArg::EspBin if bytes.starts_with(b"UF2\n") => {
                return Err("--format esp-bin does not match the supplied UF2 artifact".into());
            }
            _ => {}
        }
        let boot_rom_path = arguments.boot_rom.as_ref().ok_or_else(|| {
            format!("{target} native firmware boot requires --boot-rom with the matching real mask-ROM image")
        })?;
        let boot_rom_bytes = fs::read(boot_rom_path)?;
        remu_machines::verify_esp_radio_rom(target, &boot_rom_bytes)?;
        let boot_rom = FirmwareImage::parse_addressed_sections(&boot_rom_bytes)?;
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
            let payload_image = remu_image::EspExecutableImage::parse(&payload)?;
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
        let expected_chip_id = match target {
            TargetId::Esp32c6 => 13,
            TargetId::Esp32s3 => 9,
            _ => unreachable!(),
        };
        if image.application.header.chip_id != expected_chip_id {
            return Err(format!(
                "ESP application chip ID {} does not match target {target}; expected {expected_chip_id}",
                image.application.header.chip_id
            )
            .into());
        }
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
                machine.load_boot_rom(&boot_rom)?;
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
                machine.load_boot_rom(&boot_rom)?;
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
    if !matches!(
        arguments.format,
        FirmwareFormatArg::Auto | FirmwareFormatArg::Uf2
    ) {
        return Err("RP2040 and RP2350 firmware boot requires UF2".into());
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

pub(super) fn write_access_log(
    path: Option<&Path>,
    accesses: &[remu_bus::BusAccessRecord],
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

fn summarize_esp_executable(image: &remu_image::EspExecutableImage) -> EspExecutableSummary {
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
