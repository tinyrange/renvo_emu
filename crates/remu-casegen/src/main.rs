//! Generates the checked-in, undefined-behavior-free C conformance corpus.

use std::path::PathBuf;

mod generator;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("corpus/edge_cases"), PathBuf::from);
    generator::generate(&output)
}
