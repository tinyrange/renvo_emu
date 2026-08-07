use super::BoardArgs;
use remu_core::{RunLimits, SimTime};
use remu_image::FirmwareImage;
use remu_machines::{run_board_scenario, run_m5sticks3_firmware_scenario};
use remu_starlark::evaluate_board_script;
use remu_trace::{Timescale, VcdWriter};
use std::error::Error;
use std::fs::{self, File};

pub(super) fn board(arguments: &BoardArgs) -> Result<(), Box<dyn Error>> {
    let source = fs::read_to_string(&arguments.file)?;
    let scenario = evaluate_board_script(
        &arguments.file.display().to_string(),
        &source,
        &arguments.load_root,
    )?;
    if let Some(parent) = arguments.artifact.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(elf) = &arguments.elf {
        let firmware = FirmwareImage::parse(&fs::read(elf)?)?;
        let limits = RunLimits {
            instructions: Some(arguments.max_instructions),
            deadline: arguments.deadline.map(SimTime::from_ticks),
        };
        let result = if let Some(path) = &arguments.vcd {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let output = File::create(path)?;
            let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
            run_m5sticks3_firmware_scenario(&scenario, &firmware, limits, Some(&mut writer))?
        } else {
            run_m5sticks3_firmware_scenario(&scenario, &firmware, limits, None)?
        };
        fs::write(&arguments.artifact, serde_json::to_vec_pretty(&result)?)?;
        println!(
            "live board firmware passed for {} ({}); artifact: {}",
            result.board,
            scenario.target,
            arguments.artifact.display()
        );
    } else {
        let result = if let Some(path) = &arguments.vcd {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let output = File::create(path)?;
            let mut writer = VcdWriter::new(output, Timescale::Nanosecond);
            run_board_scenario(&scenario, Some(&mut writer))?
        } else {
            run_board_scenario(&scenario, None)?
        };
        fs::write(&arguments.artifact, serde_json::to_vec_pretty(&result)?)?;
        println!(
            "board scenario passed for {} ({}); artifact: {}",
            result.board,
            result.target,
            arguments.artifact.display()
        );
    }
    Ok(())
}
