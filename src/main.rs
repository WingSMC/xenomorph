use std::env;
use std::fs;

mod lexer;
mod parser;
mod tokens;

fn main() {
    let mut filepath = env::current_exe().unwrap();
    filepath.pop();
    filepath.push("../../tests/examples/lexer/1.xen");
    let contents = fs::read_to_string(filepath);

    let c = match contents {
        Err(e) => return println!("Error: {}", e),
        Ok(s) => s,
    };

    let mut lexer = lexer::Lexer::new(&c);
    let result = lexer.tokenize();

    match result {
        Err((e, loc)) => {
            println!("Error: {}", e);
            println!("Location: {:?}", loc);
        }
        Ok(tokens) => {
            dbg!(tokens);
        }
    }
}
