use super::*;
use crate::access_output::DirectAccessOutput;
use crate::run_command::{
    run_loaded_recorded, run_mcs51_program_recorded, run_pic_program_recorded,
};

pub(super) fn boot_native_image(
    arguments: &FirmwareBootArgs,
    target: TargetId,
    bytes: &[u8],
    stimuli: &[PinStimulus],
) -> Result<RunResult, Box<dyn Error>> {
    let format = native_format(arguments.format, bytes);
    let limits = RunLimits {
        instructions: Some(arguments.max_instructions),
        deadline: arguments.deadline.map(SimTime::from_ticks),
    };
    let access_output = DirectAccessOutput::new(arguments.bus_log.as_deref(), &[], false)?;
    let control = DirectRunControl {
        access_observer: access_output.observer(),
        esp32c6_mmu_page_size: None,
        esp32c6_flash_image: None,
        esp32c6_boot_image: None,
        esp32s3_boot_image: None,
        esp_boot_rom: None,
        radio_replay: None,
        radio_input: None,
        radio_script: None,
        radio_repl: false,
        agent_script: None,
        agent_repl: false,
        breakpoints: &[],
        watchpoints: &[],
        signal_stops: &[],
    };
    let output = arguments.vcd.as_ref().map(File::create).transpose()?;
    let result = match target {
        TargetId::Pic16f15376 => {
            if format != FirmwareFormatArg::IntelHex {
                return Err("PIC16F15376 native firmware must be Intel HEX".into());
            }
            let image = IntelHexImage::parse(bytes)?;
            let program = image.program_words(14, ProgramWordEndianness::Little)?;
            if let Some(output) = output {
                let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
                run_pic_program_recorded(
                    target,
                    &program,
                    limits,
                    stimuli,
                    Some(&mut writer),
                    control,
                )?
            } else {
                run_pic_program_recorded(target, &program, limits, stimuli, None, control)?
            }
        }
        TargetId::Efm8bb52f32g => {
            if format != FirmwareFormatArg::IntelHex {
                return Err("EFM8BB52F32G native firmware must be Intel HEX".into());
            }
            let image = IntelHexImage::parse(bytes)?;
            if let Some(output) = output {
                let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
                run_mcs51_program_recorded(
                    target,
                    &image,
                    limits,
                    stimuli,
                    Some(&mut writer),
                    control,
                )?
            } else {
                run_mcs51_program_recorded(target, &image, limits, stimuli, None, control)?
            }
        }
        _ => {
            let image = byte_addressed_image(target, format, bytes)?;
            if let Some(output) = output {
                let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
                run_loaded_recorded(target, &image, limits, stimuli, Some(&mut writer), control)?
            } else {
                run_loaded_recorded(target, &image, limits, stimuli, None, control)?
            }
        }
    };
    access_output.finish()?;
    Ok(result)
}

fn native_format(requested: FirmwareFormatArg, bytes: &[u8]) -> FirmwareFormatArg {
    if !matches!(requested, FirmwareFormatArg::Auto) {
        return requested;
    }
    if bytes.first() == Some(&b':') {
        FirmwareFormatArg::IntelHex
    } else {
        FirmwareFormatArg::RawBin
    }
}

fn byte_addressed_image(
    target: TargetId,
    format: FirmwareFormatArg,
    bytes: &[u8],
) -> Result<FirmwareImage, Box<dyn Error>> {
    let architecture = match target {
        TargetId::Ch32v003 | TargetId::Ch32v006 => FirmwareArchitecture::RiscV32,
        TargetId::Atsamd21e18 | TargetId::Stm32l432kc | TargetId::R7fa4m1ab3cfm => {
            FirmwareArchitecture::Arm
        }
        TargetId::Atmega328pb => FirmwareArchitecture::Avr8,
        TargetId::Msp430fr2433 => FirmwareArchitecture::Msp430X,
        _ => {
            return Err(
                format!("target {target} does not use a byte-addressed native image").into(),
            );
        }
    };
    let flash_base = target_manifest(target)
        .memory
        .iter()
        .find(|region| region.kind == remu_machines::MemoryKind::Flash && region.executable)
        .ok_or_else(|| format!("target {target} has no executable flash region"))?
        .start;
    let (entry, segments) = match format {
        FirmwareFormatArg::RawBin => (
            flash_base,
            vec![remu_image::FirmwareSegment {
                address: flash_base,
                load_address: None,
                initialized_size: bytes.len(),
                data: bytes.to_vec(),
                executable: true,
                writable: false,
                alignment: 1,
            }],
        ),
        FirmwareFormatArg::IntelHex => {
            let image = IntelHexImage::parse(bytes)?;
            let segments = image
                .segments
                .into_iter()
                .map(|segment| remu_image::FirmwareSegment {
                    address: u64::from(segment.address),
                    load_address: None,
                    initialized_size: segment.data.len(),
                    data: segment.data,
                    executable: true,
                    writable: false,
                    alignment: 1,
                })
                .collect();
            (
                u64::from(image.entry.unwrap_or(u32::try_from(flash_base)?)),
                segments,
            )
        }
        _ => return Err("native byte-addressed targets require Intel HEX or raw binary".into()),
    };
    Ok(FirmwareImage {
        architecture,
        entry,
        segments,
        symbols: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_detects_intel_hex_and_raw_binary() {
        assert_eq!(
            native_format(FirmwareFormatArg::Auto, b":00000001FF\n"),
            FirmwareFormatArg::IntelHex
        );
        assert_eq!(
            native_format(FirmwareFormatArg::Auto, &[0, 1, 2, 3]),
            FirmwareFormatArg::RawBin
        );
    }

    #[test]
    fn roots_raw_images_at_each_targets_flash_base() {
        for (target, expected) in [
            (TargetId::Ch32v003, 0),
            (TargetId::Ch32v006, 0),
            (TargetId::Atsamd21e18, 0),
            (TargetId::Stm32l432kc, 0x0800_0000),
            (TargetId::R7fa4m1ab3cfm, 0),
            (TargetId::Atmega328pb, 0),
            (TargetId::Msp430fr2433, 0xc000),
        ] {
            let image = byte_addressed_image(target, FirmwareFormatArg::RawBin, &[1, 2]).unwrap();
            assert_eq!(image.entry, expected);
            assert_eq!(image.segments[0].address, expected);
        }
    }
}
