use std::env;

mod lexer {
    pub mod tokens;
    pub mod lexer;
}

fn main() {
    let args: Vec<String> = env::args().collect();
    dbg!(args);

    let file = "asd".to_string();
    let result = lexer::lexer::tokenizer(&file);

    match result {
        Err((e, _)) => {
            println!("Error: {}", e);
        }

        Ok(tokens) => {
            for token in tokens {
                println!("{:?}", token);
            }
        }
    }
}
