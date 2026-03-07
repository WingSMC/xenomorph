use crate::lexer::Token;

/**
Just slices the value of the token to remove the comment boundries '/**' and '*/',
*/
pub fn extract_documentation<'src>(token: &Token<'src>) -> &'src str {
    let value = token.1.v;
    let len = value.len();
    &value[3..len - 2]
}
