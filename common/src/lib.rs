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
    pub l: u32,
    /** The column number of the token (0 indexed) */
    pub c: u32,
}

#[derive(Clone, Debug)]
pub struct XenoError<'src> {
    pub location: TokenData<'src>,
    pub message: String,
}
