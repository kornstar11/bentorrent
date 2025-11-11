
use std::pin::Pin;

use anyhow::Result;
use futures::FutureExt;
use super::error::PeerError;

use crate::{model::V1Torrent, peer::PIECE_BLOCK_SIZE};

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
        let last_piece_size = torrent.info.length as usize % total_pieces;

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




pub trait TorrentWriter: Send + Sync {
    //fn write_piece(&'static mut self, index: u32, begin: u32, block: Vec<u8>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + Sync>>;

    async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<()>;
}

#[derive(Debug)]
pub struct MemoryTorrentWriter {
    torrent: V1Torrent,
    allocation: TorrentAllocation,
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
            torrent,
            allocation,
            memory,
        }
    }

    pub fn into(self) -> Vec<u8> {
        self.memory.into_iter().flatten().collect()
    }
}

impl TorrentWriter for MemoryTorrentWriter {
    async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<()> {
        if let Some(piece) = self.memory.get_mut(index as usize) {
            let end_pos = begin as usize + block.len();
            if end_pos >= piece.len() {
                return Err(PeerError::BadBounds(index as _, end_pos).into());
            }
            for (idx, ele) in block.into_iter().enumerate() {
                let piece_idx = idx + begin as usize;
                piece[piece_idx] = ele;
            }

            Ok(())
        } else{
            Err(PeerError::BadPieceIdx(index as _).into())
        }
    }
}

