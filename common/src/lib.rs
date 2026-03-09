pub mod config;
pub mod lexer;
pub mod parser;
pub mod plugins;
pub mod semantic;
pub mod utils;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenData<'src> {
    /** The value of the token */
    pub v: &'src str,
    /** The line number of the token (0 indexed) */
    pub l: usize,
    /** The column number of the token (0 indexed) */
    pub c: usize,
}

#[derive(Clone, Debug)]
pub struct ParseError<'src> {
    pub location: TokenData<'src>,
    pub message: String,
}
