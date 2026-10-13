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

pub use xenomorph_plugin_api::source::{SourceDocument, TextRange, TokenData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum XenoDiagSeverity {
    Err,
    Warn,
    Info,
}

#[derive(Clone, Debug)]
pub struct XenoDiagnostic {
    pub location: TokenData,
    pub message: String,
    pub severity: XenoDiagSeverity,
}
