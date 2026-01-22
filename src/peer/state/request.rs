use std::{collections::{BTreeMap, HashMap, HashSet, VecDeque}, sync::Arc, time::{Duration, Instant}};

use crate::{
    model::{InternalPeerId, PeerRequestedPiece, V1Torrent},
    peer::{PIECE_BLOCK_SIZE, TorrentAllocation}
};

// within a piece this maps the blocks using the "begin" as the key
type OutstandingBlockRequests = BTreeMap<u32, BlockDownload>;

///
/// Per piece, we need to break it into blocks that can be requested. This helps keep track of this process
#[derive(Debug)]
pub struct PieceBlockAllocation {
    pub requests_to_make: Vec<PeerRequestedPiece>,
}

impl PieceBlockAllocation {
    fn new(
        piece_id: u32,
        torrent: &V1Torrent,
        peer_ids: HashSet<InternalPeerId>,
        outstanding_requests: &OutstandingBlockRequests,
        request_capacity: &mut usize,
    ) -> Option<Self> {
        if let Some(peer_id) = peer_ids.iter().next() {
            let mut requests_to_make = vec![];
            let allocation = TorrentAllocation::allocate_torrent(torrent);
            let last_piece_id = torrent.info.pieces.len() as u32 - 1;
            let piece_size = if piece_id == last_piece_id {
                // is this the last piece?
                allocation.last_piece_size
            } else {
                allocation.max_piece_size
            };
            for begin in (0..piece_size).step_by(PIECE_BLOCK_SIZE) {
                let already_requested = outstanding_requests
                    .get(&(begin as u32))
                    .and_then(|req| { // this may not be needed
                        if req.piece_request.index == piece_id {
                            Some(req)
                        } else {
                            None
                        }
                    })
                    .is_some();
                if already_requested || *request_capacity == 0 { // skip based on the "begin" parameter and the piece_id, or no more request capacity is left.
                    continue;
                }
                (*request_capacity) -= 1;

                let mut length = PIECE_BLOCK_SIZE;
                if begin + length > piece_size {
                    length = piece_size - begin;
                }

                requests_to_make.push(PeerRequestedPiece {
                    peer_id: Arc::clone(&peer_id),
                    index: piece_id,
                    begin: begin as _,
                    length: length as _,
                });
            }
            Some(Self { requests_to_make })
        } else {
            None
        }
    }
}
#[derive(Clone, Debug)]
struct BlockDownload {
    started: Instant,
    piece_request: PeerRequestedPiece,
}

impl BlockDownload {
    fn new(piece_request: PeerRequestedPiece) -> Self {
        Self {
            started: Instant::now(),
            piece_request,
        }
    }
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, PartialOrd)]
struct PieceToBlockKey {
    piece_id: u32,
    block_begin: u32,
}
#[derive(Debug, Default)]
struct PieceToBlockMap {
    piece_to_blocks_started: HashMap<u32, OutstandingBlockRequests>,
    expirations: VecDeque<(Instant, PieceToBlockKey)>
}

impl PieceToBlockMap {
    fn insert(&mut self, started: Instant, piece_request: PeerRequestedPiece) {
        let k = PieceToBlockKey { piece_id: piece_request.index, block_begin: piece_request.begin };
        let download = BlockDownload {
            started,
            piece_request
        };
        // push to expirations queue
        self.expirations.push_back((started, k));
        let block_map = self
            .piece_to_blocks_started
            .entry(k.piece_id)
            .or_insert_with(|| BTreeMap::default());
        block_map.insert(k.block_begin, download);
    }

    fn remove_expired(&mut self, time: Instant, ttl: Duration) -> Vec<PeerRequestedPiece> {
        let mut acc = vec![];
        while let Some((entry_time, k)) = self.expirations.pop_front() {
            let lived_time = time
                .checked_duration_since(entry_time)
                .unwrap_or(Duration::ZERO);
            if lived_time >= ttl {
                if let Some(expired) = self.piece_to_blocks_started
                    .get_mut(&k.piece_id)
                    .and_then(|begin_map| {
                        begin_map.remove(&k.block_begin)
                }) {
                    acc.push(expired.piece_request);
                }
            } else {
                break;
            }
        }
        acc
    }

    fn all_outstanding_requests(&self) -> Vec<BlockDownload> {
        self.piece_to_blocks_started.iter().flat_map(|(_, requests)| {
            requests.iter().map(|(_, request)| {request.clone()})
        }).collect()
    }

    fn inprogress_requests_by_piece_id(&mut self, piece_id: u32) -> &mut OutstandingBlockRequests {
        self
            .piece_to_blocks_started
            .entry(piece_id)
            .or_insert_with(|| BTreeMap::new())
    }
    
}

#[derive(Default, Debug)]
pub struct PieceBlockTracker {
    max_outstanding_requests: usize,
    request_timeout: Duration,
    // mapping of piece_id to chunk start (begin in the protocol)
    piece_to_blocks_started: PieceToBlockMap,
    //piece_to_blocks_started: HashMap<u32, OutstandingBlockRequests>,
    pieces_completed: HashSet<u32>,
}

impl PieceBlockTracker {
    pub fn new() -> Self {
        Self {
            max_outstanding_requests: 4,
            request_timeout: Duration::from_secs(10),
            piece_to_blocks_started: Default::default(),
            pieces_completed: Default::default(),
        }
    }

    pub fn pieces_completed_len(&self) -> usize {
        self.pieces_completed.len()
    }

    pub fn outstanding_requests_len(&self) -> usize {
        self.piece_to_blocks_started.all_outstanding_requests().len()
    }

    // look for requests that have started, but not finished in request_timeout
    fn remove_expired(&mut self) -> Vec<PeerRequestedPiece> {
        let now = Instant::now();
        self.piece_to_blocks_started.remove_expired(now, self.request_timeout)
    }

    pub fn assign_piece(&mut self, torrent: &V1Torrent, piece_id: u32, peer_ids: HashSet<InternalPeerId>) -> Vec<PeerRequestedPiece> {
        let now = Instant::now();
        let inflight_requests = self.outstanding_requests_len();
        let mut capacity = self.max_outstanding_requests - inflight_requests;
        let outstanding_requests = self.piece_to_blocks_started.inprogress_requests_by_piece_id(piece_id);
        let assignable_requests_opt = PieceBlockAllocation::new(
            piece_id, 
            torrent, 
            peer_ids, 
            outstanding_requests,
            &mut capacity,
        );

        if let Some(alloc) = assignable_requests_opt {
            let allocated_requests = alloc.requests_to_make;
            for req in allocated_requests.iter() {
                self.piece_to_blocks_started.insert(now, req.clone());
            }
            allocated_requests
        } else {
            vec![]
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
        let req = PieceBlockAllocation::new(
            piece_id,
            &torrent,
            vec![peer_id.clone()].into_iter().collect(),
        )
        .unwrap();
        assert_eq!(
            req.requests_to_make[0],
            PeerRequestedPiece {
                peer_id: Arc::clone(&peer_id),
                index: piece_id,
                begin: 0,
                length: PIECE_BLOCK_SIZE as _
            }
        );
        assert_eq!(
            req.requests_to_make.last().unwrap(),
            &PeerRequestedPiece {
                peer_id,
                index: piece_id,
                begin: 5111808,
                length: 8192
            }
        );
    }

    #[test]
    fn create_correct_request_last_piece() {
        let torrent = torrent_fixture(vec![1; 20]);
        let peer_id: InternalPeerId = Arc::new(vec![2; 20]);
        let piece_id = 1;
        let req = PieceBlockAllocation::new(
            piece_id,
            &torrent,
            vec![peer_id.clone()].into_iter().collect(),
        )
        .unwrap();
        assert_eq!(
            req.requests_to_make[0],
            PeerRequestedPiece {
                peer_id: Arc::clone(&peer_id),
                index: piece_id,
                begin: 0,
                length: PIECE_BLOCK_SIZE as _
            }
        );
        assert_eq!(
            req.requests_to_make.last().unwrap(),
            &PeerRequestedPiece {
                peer_id,
                index: piece_id,
                begin: 5111808,
                length: 8192
            }
        );
    }
}
