use clap::Parser as ArgParser;

use lexer::lexer::Lexer;
use parser::parser::Parser;
use semantic::analyzer::analyze;
use std::env;
use std::fs;

mod lexer;
mod parser;
mod semantic;

#[derive(ArgParser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "../../tests/examples/parser/p1.xen")]
    path: String,

    #[arg(short, long)]
    tokens: bool,

    #[arg(short, long)]
    ast: bool,
}

fn main() {
    let args = Args::parse();
    let filepath = env::current_exe().unwrap().with_file_name(args.path);
    let contents = fs::read_to_string(filepath);
    let c = match contents {
        Err(e) => return println!("Error: {}", e),
        Ok(s) => s,
    };

    let tokens = match Lexer::new(&c).tokenize() {
        Err((e, loc)) => {
            println!("Lexer error: {} at location [{}]", e, loc);
            return;
        }
        Ok(tokens) => {
            if args.tokens {
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
            if args.ast {
                print!("{:?}\n\n", ast)
            }
            ast
        }
    };

    analyze(&ast);
}
