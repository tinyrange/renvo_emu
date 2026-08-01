use super::BoardArgs;
use remu_machines::run_board_scenario;
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
    if let Some(parent) = arguments.artifact.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&arguments.artifact, serde_json::to_vec_pretty(&result)?)?;
    println!(
        "board scenario passed for {} ({}); artifact: {}",
        result.board,
        result.target,
        arguments.artifact.display()
    );
    Ok(())
}
