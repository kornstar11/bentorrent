mod bitfield;
mod error;
mod io;
mod net;
mod protocol;
mod state;
mod tracker;

use anyhow::Result;
use sha1::Digest;
use std::{sync::Arc, time::SystemTime};

use reqwest::Client;
pub use tracker::TrackerClient;

use crate::{
    config::Config,
    model::{InternalPeerId, V1Torrent},
    peer::{
        io::{FileTorrentIO, IoHandler, MemoryTorrentIO, TorrentIO},
        net::connect_torrent_peers,
        state::TorrentProcessor,
    },
};

pub const PIECE_BLOCK_SIZE: usize = 16_384;

#[derive(Debug)]
pub struct TorrentAllocation {
    total_pieces: usize,
    max_piece_size: usize,
    last_piece_size: usize,
}

impl TorrentAllocation {
    fn allocate_torrent(torrent: &V1Torrent) -> Self {
        let total_pieces = torrent.info.pieces.len();

        let max_piece_size = torrent.info.piece_length as usize;
        let last_piece_size = torrent.info.length as usize - ((total_pieces - 1) * max_piece_size); //torrent.info.length as usize % total_pieces;

        Self {
            total_pieces,
            max_piece_size,
            last_piece_size,
        }
    }

    pub fn piece_len(&self, piece_idx: usize) -> usize {
        if piece_idx == self.total_pieces - 1 {
            self.last_piece_size
        } else {
            self.max_piece_size
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

// pub async fn start_processing(torrent: V1Torrent, config: Config) -> Result<()> {
// }

async fn inner_start_processing(
    torrent: V1Torrent,
    torrent_processor: TorrentProcessor,
    our_id: InternalPeerId,
    config: Config,
) -> Result<()> {
    let client = Client::new();
    log::info!(
        "Torrent start: our_id={}, torrent_len={}, piece_len={}",
        hex::encode(our_id.as_ref()),
        torrent.info.length,
        torrent.info.piece_length
    );

    let (conn_tx, conn_rx) = tokio::sync::mpsc::channel(1);

    let torrent_processor_task =
        tokio::spawn(async move { torrent_processor.start(conn_rx).await });

    let tracker_client = TrackerClient::new(torrent.clone(), client, Arc::clone(&our_id));
    let tracker_response = tracker_client.get_announce().await?;
    log::info!(
        "Tracker responds: peers_availiable={}",
        tracker_response.peers.len()
    );

    let connections = connect_torrent_peers(torrent, our_id, tracker_response, conn_tx, config);

    tokio::select! {
        _ = torrent_processor_task => {
            log::info!("Torrent processor done...");
        }
        _ = connections => {
            log::info!("Connection processor done...");
        }
    }

    Ok(())
}

pub async fn start_processing(torrent: V1Torrent, config: Config) -> Result<()> {
    let our_id = make_peer_id();
    let io: Box<dyn TorrentIO> = if config.use_file_writer {
        Box::new(FileTorrentIO::new(torrent.clone()).await?)
    } else {
        Box::new(MemoryTorrentIO::new(torrent.clone()).await)
    };
    let io = IoHandler::new(config.clone(), io).await?;
    let torrent_processor = TorrentProcessor::new(config.clone(), Arc::clone(&our_id), torrent.clone(), io);
    inner_start_processing(torrent, torrent_processor, our_id, config).await
}

#[cfg(test)]
mod test {
    use crate::model::{V1Piece, V1Torrent, V1TorrentInfo};

    pub fn torrent_fixture_impl(info_hash: Vec<u8>, pieces_len: usize, piece_length: i64, length: i64) -> V1Torrent {
        let pieces = (1..= pieces_len)
            .into_iter()
            .map(|i| {
                let id = (i * 11) as u8;
                V1Piece { hash: vec![id; 20] }

        }).collect();
        V1Torrent {
            info: V1TorrentInfo {
                length,
                piece_length,
                name: "test.txt".to_string(),
                pieces,
                info_hash,
            },
            announce: String::new(),
            announce_list: vec![],
        }
    }

    pub fn torrent_fixture(info_hash: Vec<u8>) -> V1Torrent {
        torrent_fixture_impl(info_hash, 2, 5_120_000, 10_240_000)
    }
}
