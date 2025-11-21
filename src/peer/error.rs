use thiserror::Error;

#[derive(Error, Debug)]
pub enum PeerError {
    #[error("Bad Piece Bounds: {0} {1}")]
    BadBounds(usize, usize),
    #[error("Bad Piece Index: {0}")]
    BadPieceIdx(usize),
    #[error("File was already finished, and more is being added: piece_id={0}")]
    FileAlreadyComplete(u32),
    #[error("Piece has a mismatched hash: piece_id={0}, computed={1}, expected={2}")]
    PieceHashMismatch(u32, String, String),
    #[error("Error: {0}")]
    Other(String)
}

impl PeerError {
    pub fn other(msg: &str) -> PeerError {
        PeerError::Other(msg.to_string())
    }
}