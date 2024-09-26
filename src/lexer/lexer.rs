use crate::lexer::tokens::Token;
static NOT_IMPLEMENTED: &str = "Not implemented";

pub fn tokenizer(file: &String) -> Result<Vec<Token>, (&'static str, String)> {
    Err((NOT_IMPLEMENTED, String::new()))
}
