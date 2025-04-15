use lexer::lexer::Lexer;
use parser::parser::Parser;
use semantic::analyzer::analyze;
use std::fs;
use xenomorph_common::{config::Config, plugins::load_plugins};

mod lexer;
mod parser;
mod semantic;

fn main() {
    let config = Config::get();
    let dbg_config = &config.debug;
    let contents = fs::read_to_string(config.workdir.join(&config.parser.path));

    let plugins = load_plugins();

    if dbg_config.plugins {
        dbg!(&plugins);
        dbg!((plugins[0].provide)());
    }

    let c = match contents {
        Err(e) => return println!("Error: {}", e),
        Ok(s) => s,
    };

    let tokens = match Lexer::new(&c).tokenize() {
        Err((e, loc)) => return println!("Lexer error: {} at location [{}]", e, loc),
        Ok(tokens) => {
            if dbg_config.tokens {
                print!("{:?}\n\n", tokens)
            }
            tokens
        }
    };

    let ast = match Parser::new(&tokens).parse() {
        Err(e) => {
            println!("Parser error: {}", e);
            return;
        }
        Ok(ast) => {
            if dbg_config.ast {
                print!("{:?}\n\n", ast)
            }
            ast
        }
    };

    analyze(&ast);
}
