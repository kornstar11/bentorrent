mod file;
mod state;
use super::error::PeerError;
use anyhow::Result;
use async_trait::async_trait;
pub use file::FileTorrentIO;
pub use state::IoHandler;

use crate::{model::V1Torrent, peer::TorrentAllocation};

#[async_trait]
pub trait TorrentIO: Send {
    //fn write_piece(&'static mut self, index: u32, begin: u32, block: Vec<u8>) -> Pin<Box<dyn Future<Output = Result<()>> + Send + Sync>>;

    //async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<bool>;
    async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<bool>;
    async fn read_piece(&mut self, index: u32, begin: u32, length: u32) -> Result<Vec<u8>>;
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
            expected_size,
        }
    }

    ///
    /// Takes the slices, and will return true when the entire piece is complete.
    fn insert(&mut self, begin: u32, len: u32) -> bool {
        let end_pos = begin + len;
        let slice = (begin, end_pos);
        let insert_pos = self
            .slices
            .binary_search_by_key(&slice.0, |&(start, _)| start);
        match insert_pos {
            Ok(existing_pos) => {
                // attempt to grow the length
                let curr = &mut self.slices[existing_pos];
                let new_end = curr.1.max(slice.1);
                *curr = (curr.0, new_end); // Grow the existing
            }
            Err(missing) => {
                //start is different, but something is there
                let mut start = begin;
                let mut end = end_pos;
                let mut was_combined_right = false;
                let mut was_combined_left = false;
                if let Some(look_right) = self.slices.get_mut(missing) {
                    // there was an element where we would go, see if there would be an overlap
                    if look_right.0 == slice.1 {
                        //if the existing start == incomings end, its an overlap,
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
                if !was_combined_left && !was_combined_right {
                    // if no combining happened, we know we must insert
                    self.slices.insert(missing, (start, end));
                } else if was_combined_left && was_combined_right {
                    // if both were combined, we know that the end must be removed, all other cases already happened
                    let _ = self.slices.remove(missing);
                }
                // // other case
            }
        }
        self.is_complete()
    }

    fn is_complete(&self) -> bool {
        self.slices.len() == 1
            && self.slices[0].0 == 0
            && self.slices[0].1 == self.expected_size as u32
    }
}

#[derive(Debug)]
struct Memory {
    memory: Vec<u8>,
    slices: Slices,
}

impl From<Vec<u8>> for Memory {
    fn from(memory: Vec<u8>) -> Self {
        Self {
            slices: Slices::new(memory.len()),
            memory,
        }
    }
}

#[derive(Debug)]
pub struct MemoryTorrentIO {
    memory: Vec<Memory>,
}

impl MemoryTorrentIO {
    pub async fn new(torrent: V1Torrent) -> Self {
        let allocation = TorrentAllocation::allocate_torrent(&torrent);
        let mut memory = vec![];
        for _ in 1..allocation.total_pieces {
            memory.push(Memory::from(vec![0; allocation.max_piece_size]));
        }
        memory.push(Memory::from(vec![0; allocation.last_piece_size]));

        Self { memory }
    }
}
#[async_trait]
impl TorrentIO for MemoryTorrentIO {
    async fn write_piece(&mut self, index: u32, begin: u32, block: Vec<u8>) -> Result<bool> {
        if let Some(Memory { memory, slices }) = self.memory.get_mut(index as usize) {
            log::debug!("Writing {}", memory.len());
            let end_pos = begin as usize + block.len();
            if end_pos > memory.len() {
                return Err(PeerError::BadBounds(index as _, end_pos).into());
            }
            let finished = slices.insert(begin, block.len() as _);
            for (idx, ele) in block.into_iter().enumerate() {
                let piece_idx = idx + begin as usize;
                memory[piece_idx] = ele;
            }

            Ok(finished)
        } else {
            Err(PeerError::BadPieceIdx(index as _).into())
        }
    }

    async fn read_piece(&mut self, index: u32, begin: u32, length: u32) -> Result<Vec<u8>> {
        if let Some(Memory { memory, slices: _ }) = self.memory.get_mut(index as usize) {
            let end_offset = begin + length;
            Ok(Vec::from(&memory[(begin as usize)..(end_offset as usize)]))
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
