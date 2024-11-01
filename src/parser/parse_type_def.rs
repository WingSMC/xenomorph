use super::{parser::Parser, parser_expr::Expr};
use crate::tokens::TokenVariant;

impl<'src> Parser<'src> {
	pub fn parse_type_definition(&mut self) -> Result<Expr, String> {
		let name_opt = self.next();
		let name = match name_opt {
			Some((TokenVariant::Identifier, d)) => d,
			_ => return Err(format!("Expected identifier after {:?}", self.prev())),
		};
		

        
    }
}