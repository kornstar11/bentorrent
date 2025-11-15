use std::{collections::HashSet, sync::Arc};

use crate::{model::V1Torrent, peer::{InternalPeerId, PIECE_BLOCK_SIZE, TorrentAllocation}};

#[derive(Debug)]
pub struct PeerRequestedPiece {
    pub peer_id: InternalPeerId,
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}

#[derive(Debug)]
pub struct PieceBlockTracking {
    pub requests_to_make: Vec<PeerRequestedPiece>,
}

impl PieceBlockTracking {
    pub fn new(piece_id: u32, torrent: &V1Torrent, peer_ids: HashSet<InternalPeerId>) -> Option<Self> {
        if let Some(peer_id) = peer_ids.iter().next() {
            let mut requests_to_make = vec![];
            let allocation = TorrentAllocation::allocate_torrent(torrent);
            let piece_size = if piece_id == (torrent.info.pieces.len() - 1) as u32 {
                allocation.last_piece_size
            } else {
                allocation.max_piece_size
            };
            for begin in (0..piece_size).step_by(PIECE_BLOCK_SIZE) {
                let length = PIECE_BLOCK_SIZE;
                requests_to_make.push(PeerRequestedPiece{
                    peer_id: Arc::clone(&peer_id),
                    index: piece_id,
                    begin: begin as _,
                    length: length as _,

                });
            }
            Some(Self {
                requests_to_make
            })
        } else {
            None
        }
    }
}


#[cfg(test)]
mod test {
    #[test]
    fn create_correct_request_single_slice() {

    }
}