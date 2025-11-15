mod bitfield;
mod state;
mod error;
mod writer;
mod protocol;
mod tracker;
mod net;

use anyhow::Result;
use std::{sync::Arc, time::SystemTime};
use sha1::Digest;

use reqwest::Client;
pub use tracker::TrackerClient;

use crate::{model::V1Torrent, peer::{state::TorrentProcessor, writer::MemoryTorrentWriter}};

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

pub fn make_peer_id() -> InternalPeerId {
    let mut sha1 = sha1::Sha1::new();
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let id = format!("ben-{}", timestamp.as_secs());
    sha1.update(id);
    return Arc::new(sha1.finalize().to_vec());
}

pub async fn start_processing(torrent: V1Torrent) -> Result<()> {
    let client = Client::new();
    let our_id = make_peer_id();
    log::info!("My peerid: {}", hex::encode(our_id.as_ref()));

    let (conn_tx, conn_rx) = tokio::sync::mpsc::channel(1);

    let torrent_writer = MemoryTorrentWriter::new(torrent.clone());
    let torrent_processor = TorrentProcessor::new(Arc::clone(&our_id), torrent.clone(), torrent_writer);

    let torrent_processor_task = tokio::spawn(async move {
        torrent_processor.start(conn_rx)
    });

    let tracker_client = TrackerClient::new(torrent, client, our_id);
    let tracker_response = tracker_client.get_announce().await?;
    log::info!("Tracker responds:  peers_availiable={}", tracker_response.peers.len());

    tokio::select! {
        _ = torrent_processor_task => {
            log::info!("Torrent processor done...");
        }

    }

    Ok(())

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