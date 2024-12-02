use lexer::lexer::Lexer;
use parser::parser::Parser;
use semantic::analyzer::analyze;
use std::env;
use std::fs;

mod lexer;
mod parser;
mod semantic;

fn main() {
    let mut filepath = env::current_exe().unwrap();
    filepath.pop();
    filepath.push("../../tests/examples/parser/p1.xen");
    let contents = fs::read_to_string(filepath);

    let c = match contents {
        Err(e) => return println!("Error: {}", e),
        Ok(s) => s,
    };

    let mut lexer = Lexer::new(&c);

    let tokens = match lexer.tokenize() {
        Err((e, loc)) => {
            println!("Lexer error: {} at location [{}]", e, loc);
            return;
        }
        Ok(tokens) => tokens,
    };

    let mut p = Parser::new(&tokens);
    let ast = match p.parse() {
        Err(e) => {
            println!("Parser error: {}", e);
            return;
        }
        Ok(ast) => ast,
    };

    analyze(&ast);
}
