use thiserror::Error;

#[derive(Error, Debug)]
pub enum PeerError {
    #[error("Bad Piece Bounds: {0} {1}")]
    BadBounds(usize, usize),
    #[error("Bad Piece Index: {0}")]
    BadPieceIdx(usize),
    #[error("Error: {0}")]
    Other(String)
}