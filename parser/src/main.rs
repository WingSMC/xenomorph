use lexer::lexer::Lexer;
use parser::parser::Parser;
use semantic::analyzer::analyze;
use std::fs;
use xenomorph_common::{
    config::{load_config_key, workdir_path},
    plugins::load_plugins,
};

mod lexer;
mod parser;
mod semantic;

fn main() {
    let parser_config = load_config_key("parser");
    let file_path = parser_config.get("path").unwrap().as_str().unwrap();
    let filepath = workdir_path(file_path);
    let contents = fs::read_to_string(filepath);

    let plugins = load_plugins(&vec!["test".to_string()]);

    if parser_config.get("plugins").unwrap().as_bool().unwrap() {
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
            if parser_config.get("tokens").unwrap().as_bool().unwrap() {
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
            if parser_config.get("ast").unwrap().as_bool().unwrap() {
                print!("{:?}\n\n", ast)
            }
            ast
        }
    };

    analyze(&ast);
}
