use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Missing torrent values {0}")]
    Missing(String),
    #[error("V1 Torrent file has a bad length ! % 20")]
    V1PiecesByteLenWrong,
    #[error("Wrong Bencode type")]
    WrongType
}

impl Error {
    pub fn missing(s: &str) -> Error {
        Error::Missing(s.to_string())
    }
    
}