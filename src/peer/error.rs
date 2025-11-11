use thiserror::Error;

#[derive(Error, Debug)]
pub enum PeerError {
    #[error("Decoder error {0}")]
    Decode(String),
    #[error("Bad Piece Bounds: {0} {1}")]
    BadBounds(usize, usize),
    #[error("Bad Piece Index: {0}")]
    BadPieceIdx(usize),
}

impl PeerError {
    pub fn decode(s: &str) -> PeerError {
        PeerError::Decode(s.to_string())
    }
}
