use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Decoder error {0}")]
    Decode(String),
}

impl Error {
    pub fn decode(s: &str) -> Error {
        Error::Decode(s.to_string())
    }
}