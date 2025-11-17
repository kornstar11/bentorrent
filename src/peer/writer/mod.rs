use super::error::PeerError;
use anyhow::Result;

use crate::{model::V1Torrent, peer::{TorrentAllocation}};

pub trait TorrentWriter: Send + Sync {
    //fn write_piece(&'static mut self, index: u32, begin: u32, block: Vec<u8>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + Sync>>;

    async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<()>;
}

#[derive(Debug)]
pub struct MemoryTorrentWriter {
    memory: Vec<Vec<u8>>,
}

impl MemoryTorrentWriter {
    pub fn new(torrent: V1Torrent) -> Self {
        let allocation = TorrentAllocation::allocate_torrent(&torrent);
        let mut memory = vec![];
        for _ in 1..allocation.total_pieces {
            memory.push(vec![0; allocation.max_piece_size]);
        }
        memory.push(vec![0; allocation.last_piece_size]);

        Self {
            memory,
        }
    }
}

impl TorrentWriter for MemoryTorrentWriter {
    async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<()> {
        if let Some(piece) = self.memory.get_mut(index as usize) {
            log::debug!("Writing {}", piece.len());
            let end_pos = begin as usize + block.len();
            if end_pos >= piece.len() {
                return Err(PeerError::BadBounds(index as _, end_pos).into());
            }
            for (idx, ele) in block.into_iter().enumerate() {
                let piece_idx = idx + begin as usize;
                piece[piece_idx] = ele;
            }

            Ok(())
        } else {
            Err(PeerError::BadPieceIdx(index as _).into())
        }
    }
}