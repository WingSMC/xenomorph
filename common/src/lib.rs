// TODO #![feature(hint_must_use)]

pub mod config;
pub mod formatter;
pub mod lexer;
pub mod module;
pub mod parser;
pub mod plugins;
pub mod semantic;
#[cfg(test)]
mod test_strings;
pub mod utils;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenData<'src> {
    /** The value of the token */
    pub v: &'src str,
    /** The line number of the token (0 indexed) */
    pub l: u32,
    /** The column number of the token (0 indexed) */
    pub c: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XenoDiagSeverity {
    Err,
    Warn,
    Info,
}

#[derive(Clone, Debug)]
pub struct XenoDiagnostic<'diag> {
    pub location: TokenData<'diag>,
    pub message: String,
    pub severity: XenoDiagSeverity,
}
