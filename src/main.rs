use std::env;

mod lexer {
    pub mod tokens;
    pub mod lexer;
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    dbg!(args);
    lexer::lexer::tokenizer();

    let path = env::current_dir()?;
    println!("The current directory is {}", path.display());
    Ok(())
}
