use std::fs;
use xenomorph_common::{
    config::Config, parser::parser::parse, plugins::load_plugins, semantic::analyzer::analyze,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::get();
    let dbg_config = &config.debug;
    let contents = fs::read_to_string(config.workdir.join(&config.parser.path))?;
    let plugins = load_plugins();

    if dbg_config.plugins {
        dbg!(&plugins);
        dbg!((plugins[0].provide)());
    }

    let result = parse(&contents);
    analyze(&result.0);

    Ok(())
}
