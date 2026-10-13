use serde::{de::DeserializeOwned, Serialize};

pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    postcard::to_allocvec(value).map_err(|error| error.to_string())
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    let (value, rest) = postcard::take_from_bytes(bytes).map_err(|error| error.to_string())?;
    if !rest.is_empty() {
        return Err("Trailing bytes in plugin payload".into());
    }
    Ok(value)
}
