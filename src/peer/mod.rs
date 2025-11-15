mod bitfield;
mod state;
mod error;
mod writer;
mod protocol;
mod tracker;


use std::sync::Arc;

use reqwest::Client;
pub use tracker::TrackerClient;

use crate::model::V1Torrent;

pub type PeerId = Vec<u8>;
pub type InternalPeerId = Arc<PeerId>;
pub const PIECE_BLOCK_SIZE: usize = 16_384;


#[derive(Debug)]
pub struct TorrentAllocation {
    total_pieces: usize,
    max_piece_size: usize,
    last_piece_size: usize,
    max_blocks_per_piece: usize,
    blocks_in_last_piece: usize,

}

impl TorrentAllocation {
    fn allocate_torrent(torrent: &V1Torrent) -> Self {
        let total_pieces = torrent.info.pieces.len();

        let max_piece_size = torrent.info.length as usize / total_pieces;
        let last_piece_size = torrent.info.length as usize - ((total_pieces -1) * max_piece_size);//torrent.info.length as usize % total_pieces;

        let max_blocks_per_piece = max_piece_size.div_ceil(PIECE_BLOCK_SIZE);
        let blocks_in_last_piece = last_piece_size.div_ceil(PIECE_BLOCK_SIZE);

        Self {
            total_pieces,
            max_piece_size,
            last_piece_size,
            max_blocks_per_piece,
            blocks_in_last_piece,
        }
    }
}

pub async fn start_processing(torrent: V1Torrent) {
    let client = Client::new();
    let tracker_client = TrackerClient::new(torrent, client);
    let tracker_response = tracker_client.get_announce().await;

}


#[cfg(test)]
mod test {
    use crate::model::{V1Piece, V1Torrent, V1TorrentInfo};

    pub fn torrent_fixture(info_hash: Vec<u8>) -> V1Torrent {
        V1Torrent {
            info: V1TorrentInfo {
                length: 10_240_000,
                name: "test.txt".to_string(),
                pieces: vec![
                    V1Piece{hash: vec![11; 20]},
                    V1Piece{hash: vec![22; 20]},
                ],
                info_hash
            },
            announce: String::new(),
            announce_list: vec![]
        }
    }
}