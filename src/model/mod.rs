mod peer_context;
mod torrent;
mod tracker;

use std::sync::Arc;

pub use peer_context::PeerContext;
pub use torrent::{V1Piece, V1Torrent, V1TorrentInfo};
pub use tracker::{TrackerPeer, TrackerResponse};

pub type PeerId = Vec<u8>;
pub type InternalPeerId = Arc<PeerId>;

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct PeerRequestedPiece {
    pub peer_id: InternalPeerId,
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}