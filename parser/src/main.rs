use std::fs;
use xenomorph_common::{
    config::Config,
    lexer::Lexer,
    module::{build_declaration_cache, load_workspace},
    parser::Parser,
    plugins::load_plugins,
    semantic::analyze,
};

fn main() {
    let config = Config::get();
    let plugins = load_plugins();

    let dbg_config = &config.debug;
    if dbg_config.plugins {
        println!("{:?}", &plugins);
        println!(
            "{:?}",
            &plugins
                .iter()
                .map(|p| p.provide_types.map(|provide| provide()))
        );
    }

    let file_path = config.workdir.join(&config.parser.path);
    let file_name = match file_path.file_name() {
        None => {
            println!("Error: Invalid file path '{}'", file_path.display());
            return;
        }
        Some(name) => name.to_string_lossy(),
    };

    // Load the full module graph starting from the entry file
    let abs_entry = match file_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            println!("Error: Cannot resolve entry file '{}': {}", file_path.display(), e);
            return;
        }
    };

    let (registry, module_errors) = load_workspace(&abs_entry);
    for me in &module_errors {
        println!("[module] {}", me);
    }

    // Build the declaration cache across all modules
    let decl_cache = build_declaration_cache(&registry);
    if dbg_config.ast {
        println!("Declaration cache:");
        for (name, info) in &decl_cache {
            println!("  {} (from {})", name, info.module_path);
        }
    }

    // Also run the single-file pipeline for the entry file for backwards compat
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

    if config.debug.ast {
        print!("{:#?}\n\n", ast)
    }

    let semantic_errors = analyze(&ast);
    for e in &semantic_errors {
        println!(
            "[{}] Semantic error: {} At [{}]",
            file_name, e.message, e.location
        );
    }
}
