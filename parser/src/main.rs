use std::fs;
use xenomorph_common::{
    config::Config, lexer::Lexer, parser::Parser, plugins::load_plugins, semantic::analyze,
};

fn main() {
    let config = Config::get();
    let plugins = load_plugins();

    let dbg_config = &config.debug;
    if dbg_config.plugins {
        dbg!(&plugins);
        // dbg!(plugins[0].provide_types.map(|p| p()));
    }

    let file_path = config.workdir.join(&config.parser.path);
    let file_name = match file_path.file_name() {
        None => {
            println!("Error: Invalid file path '{}'", file_path.display());
            return;
        }
        Some(name) => name.to_string_lossy(),
    };

    let file_contents = match fs::read_to_string(&file_path) {
        Err(e) => return println!("Error: {}", e),
        Ok(s) => s,
    };

    let tokens = match Lexer::tokenize(&file_contents) {
        Err(err) => {
            return println!(
                "[{}] Lexer error: {} At [{}]",
                file_name, err.message, err.location
            )
        }
        Ok(tokens) => {
            if dbg_config.tokens {
                print!("{:?}\n\n", tokens)
            }
            tokens
        }
    };

    let (ast, errs) = Parser::parse(&tokens);
    if !errs.is_empty() {
        for e in errs {
            println!(
                "[{}] Parser error: {} At [{}]",
                file_name, e.message, e.location
            );
        }
        return;
    }

    analyze(&ast);
}
