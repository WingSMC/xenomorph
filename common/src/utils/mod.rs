use std::hash::{DefaultHasher, Hash, Hasher};

use crate::TokenData;

/// Just slices the value of the token to remove the comment boundries '/**' and '*/',
/// and trims the result to remove any leading or trailing whitespace.
pub fn extract_documentation<'src>(token: &TokenData<'src>) -> &'src str {
    let len = token.v.len();
    &token.v[3..len - 2].trim()
}

pub fn calculate_hash<T: Hash>(t: &T) -> u64 {
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}
