use lexer::Lexer;
use parser::parser::Parser;
use std::env;
use std::fs;

mod lexer;
mod parser;
mod tokens;

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
    let result = lexer.tokenize();

    let tokens = match result {
        Err((e, loc)) => {
            println!("Lexer error: {} at location [{}]", e, loc);
            return;
        }
        Ok(tokens) => tokens,
    };

    //dbg!(&tokens);
    let mut p = Parser::new(&tokens);
    let parser_result = p.parse();

    match parser_result {
        Err(e) => println!("Parser error: {}", e),
        Ok(ast) => drop(dbg!("{:?}", ast)),
    };
}
