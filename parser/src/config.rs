use std::io::{self, Write};

use xenomorph_common::config::inspect_merged_config;

pub fn run_config(inspect: bool) -> Result<(), String> {
    if !inspect {
        return Ok(());
    }

    let current_dir = std::env::current_dir()
        .map_err(|error| format!("Unable to determine current directory: {error}"))?;
    let output = inspect_merged_config(&current_dir)?;
    io::stdout()
        .write_all(output.as_bytes())
        .map_err(|error| format!("Unable to write merged config: {error}"))
}
