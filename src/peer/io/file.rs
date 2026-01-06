use std::{env::temp_dir, io::SeekFrom, time::SystemTime};

use crate::{
    model::V1Torrent,
    peer::{
        TorrentAllocation,
        error::PeerError,
        io::{Slices, TorrentIO},
    },
};
use anyhow::Result;
use async_trait::async_trait;
use sha1::{Digest, Sha1};
use tokio::{
    fs::{File, create_dir_all},
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};

struct FilePieceState {
    hash: Vec<u8>,
    slices: Slices,
    start_idx: usize,
    size: usize,
}

impl FilePieceState {
    async fn new(expected_size: usize, start_idx: usize, hash: Vec<u8>) -> Result<Self> {
        Ok(Self {
            hash,
            start_idx,
            slices: Slices::new(expected_size),
            size: expected_size,
        })
    }

    async fn seek_offset(&self, file: &mut File, offset: u32) -> Result<()> {
        let offset = self.start_idx + (offset as usize);
        let _ = file.seek(SeekFrom::Start(offset as _)).await?;
        Ok(())
    }

    async fn compute_file_sha1_hash(&self, file: &mut File) -> Result<Vec<u8>> {
        let mut digest = Sha1::new();
        let _ = file.seek(SeekFrom::Start(self.start_idx as _)).await?;
        let mut buf = vec![0; self.size];
        let _ = file.read_exact(&mut buf).await?;
        digest.update(&buf);
        Ok(digest.finalize().to_vec())
    }
}

pub struct FileTorrentIO {
    file: File,
    piece_to_file: Vec<FilePieceState>,
    allocation: TorrentAllocation,
}

impl FileTorrentIO {
    pub async fn new(torrent: V1Torrent) -> Result<Self> {
        let allocation = TorrentAllocation::allocate_torrent(&torrent);
        let mut path = temp_dir();
        let ts = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?;
        path.push("torrent_download");
        path.push(format!("{}_{}", torrent.info.name, ts.as_secs()));
        create_dir_all(path.clone()).await?;
        path.push("download");
        log::info!("Writing to: {}", path.to_str().unwrap());

        let file = File::create_new(path).await?;
        let _ = file.set_len(torrent.info.length as _).await?;
        let mut piece_to_file = vec![];

        for idx in 0..allocation.total_pieces {
            let piece = &torrent.info.pieces[idx];
            let piece_len = allocation.piece_len(idx);
            let start = piece_len * idx;
            let file_piece = FilePieceState::new(piece_len, start, piece.hash.clone()).await?;
            piece_to_file.push(file_piece);
        }

        Ok(Self {
            file,
            piece_to_file,
            allocation,
        })
    }
}
#[async_trait]
impl TorrentIO for FileTorrentIO {
    async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<bool> {
        if let Some(piece) = self.piece_to_file.get_mut(index as usize) {
            if piece.slices.is_complete() {
                return Err(crate::peer::error::PeerError::FileAlreadyComplete(index).into());
            }
            let end_pos = begin as usize + block.len();
            if end_pos > self.allocation.piece_len(index as _) {
                return Err(PeerError::BadBounds(index as _, end_pos).into());
            }
            let _ = piece.seek_offset(&mut self.file, begin).await?;
            self.file.write_all(&block).await?;
            let finished = piece.slices.insert(begin, block.len() as _);

            if finished {
                //self.file.flush().await?;
                let computed_hash = piece.compute_file_sha1_hash(&mut self.file).await?;
                if computed_hash == piece.hash {
                    Ok(true)
                } else {
                    Err(PeerError::PieceHashMismatch(
                        index,
                        hex::encode(&computed_hash),
                        hex::encode(&piece.hash),
                    )
                    .into())
                }
            } else {
                Ok(false)
            }
        } else {
            Err(PeerError::BadPieceIdx(index as _).into())
        }
    }

    async fn read_piece(&mut self, index: u32, begin: u32, length: u32) -> Result<Vec<u8>> {
        if let Some(piece) = self.piece_to_file.get_mut(index as usize) {
            let length = length as usize;
            let end_pos = begin as usize + length;
            if end_pos > self.allocation.piece_len(index as _) {
                return Err(PeerError::BadBounds(index as _, end_pos).into());
            }

            if !piece.slices.is_complete() {
                return Err(PeerError::FilePieceNotFinished(index).into());
            }

            let _ = piece.seek_offset(&mut self.file, begin).await?;

            let mut block = vec![0; length];
            let _ = self.file.read_exact(&mut block).await?;
            Ok(block)
        } else {
            Err(PeerError::BadPieceIdx(index as _).into())
        }
    }
}
