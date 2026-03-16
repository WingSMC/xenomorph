use crate::TokenData;

/**
Just slices the value of the token to remove the comment boundries '/**' and '*/',
*/
pub fn extract_documentation<'src>(token: &TokenData<'src>) -> &'src str {
    let len = token.v.len();
    &token.v[3..len - 2].trim()
}
