use std::{collections::HashSet, sync::Arc};

use crate::{model::V1Torrent, peer::{InternalPeerId, PIECE_BLOCK_SIZE, TorrentAllocation}};

#[derive(Debug, PartialEq, Eq)]
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
            let last_piece_id = torrent.info.pieces.len() as u32 - 1;
            let piece_size = if piece_id == last_piece_id { // is this the last piece?
                allocation.last_piece_size
            } else {
                allocation.max_piece_size
            };
            println!("Allocatoion: {:?}", allocation);
            for begin in (0..piece_size).step_by(PIECE_BLOCK_SIZE) {
                let mut length = PIECE_BLOCK_SIZE;
                if begin + length > piece_size {
                    length = piece_size - begin;
                }

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
    use super::*;
    use crate::peer::test::torrent_fixture;

    #[test]
    fn create_correct_request_first_piece() {
        let torrent = torrent_fixture(vec![1; 20]);
        let peer_id: InternalPeerId = Arc::new(vec![2; 20]);
        let piece_id = 0;
        let req = PieceBlockTracking::new(piece_id, &torrent, vec![peer_id.clone()].into_iter().collect()).unwrap();
        assert_eq!(
            req.requests_to_make[0], 
            PeerRequestedPiece { peer_id: Arc::clone(&peer_id), index: piece_id, begin: 0, length: PIECE_BLOCK_SIZE as _}
        );
        assert_eq!(
            req.requests_to_make.last().unwrap(), 
            &PeerRequestedPiece { peer_id, index: piece_id, begin: 5111808, length: 8192}
        );
    }

    #[test]
    fn create_correct_request_last_piece() {
        let torrent = torrent_fixture(vec![1; 20]);
        let peer_id: InternalPeerId = Arc::new(vec![2; 20]);
        let piece_id = 1;
        let req = PieceBlockTracking::new(piece_id, &torrent, vec![peer_id.clone()].into_iter().collect()).unwrap();
        assert_eq!(
            req.requests_to_make[0], 
            PeerRequestedPiece { peer_id: Arc::clone(&peer_id), index: piece_id, begin: 0, length: PIECE_BLOCK_SIZE as _}
        );
        assert_eq!(
            req.requests_to_make.last().unwrap(), 
            &PeerRequestedPiece { peer_id, index: piece_id, begin: 5111808, length: 8192}
        );
    }
}