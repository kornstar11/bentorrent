use async_trait::async_trait;
use std::io::SeekFrom;
use std::{env::temp_dir, time::SystemTime};
use super::error::PeerError;
use anyhow::Result;
use tokio::fs::File;
use tokio::fs::create_dir_all;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::io::AsyncWriteExt;
use sha1::{Digest, Sha1};

use crate::{model::V1Torrent, peer::{TorrentAllocation}};

#[async_trait]
pub trait TorrentWriter: Send {
    //fn write_piece(&'static mut self, index: u32, begin: u32, block: Vec<u8>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + Sync>>;

    //async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<bool>;
    async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<bool>;
}

///
/// Tracks the incoming slices, so that we know when the file is complete.
#[derive(Debug)]
struct Slices {
    slices: Vec<(u32, u32)>,
    expected_size: usize,
}

impl Slices {
    fn new(expected_size: usize) -> Self {
        Self {
            slices: vec![],
            expected_size
        }
    }

    ///
    /// Takes the slices, and will return true when the entire piece is complete.
    fn insert(&mut self, begin: u32, len: u32) -> bool {
        let end_pos = begin + len;
        let slice = (begin, end_pos);
        let insert_pos = self.slices.binary_search_by_key(&slice.0, |&(start, _)| start);
        match insert_pos {
            Ok(existing_pos) => { // attempt to grow the length
                let curr = &mut self.slices[existing_pos];
                let new_end = curr.1.max(slice.1);
                *curr = (curr.0, new_end); // Grow the existing
            },
            Err(missing) => { //start is different, but something is there
                let mut start = begin;
                let mut end = end_pos;
                let mut was_combined_right = false;
                let mut was_combined_left = false; 
                if let Some(look_right) = self.slices.get_mut(missing) { 
                    // there was an element where we would go, see if there would be an overlap
                    if look_right.0 == slice.1 { //if the existing start == incomings end, its an overlap, 
                        *look_right = (slice.0, look_right.1);
                        start = slice.0;
                        end = look_right.1;
                        was_combined_right = true; // if we combine, then do not insert
                    }
                }
                if missing > 0 {
                    if let Some(look_left) = self.slices.get_mut(missing - 1) {
                        if look_left.1 >= start {
                            *look_left = (look_left.0, end);
                            was_combined_left = true;
                        }
                    }
                }
                if !was_combined_left && !was_combined_right { // if no combining happened, we know we must insert
                    self.slices.insert(missing, (start, end));
                } else if was_combined_left && was_combined_right { // if both were combined, we know that the end must be removed, all other cases already happened
                    let _ = self.slices.remove(missing);
                }
                // // other case
            }
        }
        self.is_complete()
    }

    fn is_complete(&self) -> bool {
        self.slices.len() == 1 && self.slices[0].0 == 0 &&self.slices[0].1 == self.expected_size as u32
    }
}

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

pub struct FileWriter {
    file: File,
    piece_to_file: Vec<FilePieceState>,
    allocation: TorrentAllocation,
}

impl FileWriter {
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

        for idx  in 0..allocation.total_pieces {
            let piece = &torrent.info.pieces[idx];
            let piece_len = allocation.piece_len(idx);
            let start = piece_len * idx;
            let file_piece = FilePieceState::new( piece_len, start, piece.hash.clone()).await?;
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
impl TorrentWriter for FileWriter {
    async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<bool> {

        if let Some(piece) = self.piece_to_file.get_mut(index as usize) {
            if piece.slices.is_complete() {
                return Err(crate::peer::error::PeerError::FileAlreadyComplete(index).into())
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
                    Err(PeerError::PieceHashMismatch(index, hex::encode(&computed_hash), hex::encode(&piece.hash)).into())
                }
            } else {
                Ok(false)
            }
        } else {
            Err(PeerError::BadPieceIdx(index as _).into())
        }
    }
}

#[derive(Debug)]
pub struct MemoryTorrentWriter {
    memory: Vec<Vec<u8>>,
}

impl MemoryTorrentWriter {
    pub async fn new(torrent: V1Torrent) -> Self {
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
#[async_trait]
impl TorrentWriter for MemoryTorrentWriter {
    async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<bool> {
        if let Some(piece) = self.memory.get_mut(index as usize) {
            log::debug!("Writing {}", piece.len());
            let end_pos = begin as usize + block.len();
            if end_pos > piece.len() {
                return Err(PeerError::BadBounds(index as _, end_pos).into());
            }
            for (idx, ele) in block.into_iter().enumerate() {
                let piece_idx = idx + begin as usize;
                piece[piece_idx] = ele;
            }

            Ok(false)
        } else {
            Err(PeerError::BadPieceIdx(index as _).into())
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn slices_returns_full_sequential() {
        let mut slices = Slices::new(15);
        assert!(slices.insert(0, 5) == false);
        assert!(slices.insert(5, 5) == false);
        assert!(slices.insert(10, 5) == true);
    }

    #[test]
    fn slices_returns_full_reverse() {
        let mut slices = Slices::new(15);
        assert!(slices.insert(10, 5) == false);
        assert!(slices.insert(5, 5) == false);
        assert!(slices.insert(0, 5) == true);
    }

    #[test]
    fn slices_returns_full_single_gap() {
        let mut slices = Slices::new(15);
        assert!(slices.insert(10, 5) == false);
        assert!(slices.insert(0, 5) == false);
        assert!(slices.insert(5, 5) == true);
    }

    #[test]
    fn slices_returns_full_double_gap() {
        let mut slices = Slices::new(20);
        assert!(slices.insert(15, 5) == false);
        assert!(slices.insert(0, 5) == false);
        assert!(slices.insert(5, 5) == false);
        assert!(slices.insert(10, 5) == true);
    }

    #[test]
    fn slices_returns_full_double_gap_rev() {
        let mut slices = Slices::new(20);
        assert!(slices.insert(15, 5) == false);
        assert!(slices.insert(5, 5) == false);
        assert!(slices.insert(0, 5) == false);
        assert!(slices.insert(10, 5) == true);
    }

    #[test]
    fn slices_returns_full_double_gap_rev_2() {
        let mut slices = Slices::new(20);
        assert!(slices.insert(10, 5) == false);
        assert!(slices.insert(5, 5) == false);
        assert!(slices.insert(0, 5) == false);
        assert!(slices.insert(15, 5) == true);
    }
}