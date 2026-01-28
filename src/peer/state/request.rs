use std::{collections::{BTreeMap, HashMap, HashSet, VecDeque}, sync::Arc, time::{Duration, Instant}};

use crate::{
    model::{InternalPeerId, PeerRequestedPiece, V1Piece, V1Torrent},
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
        peer_ids: &HashSet<InternalPeerId>,
        outstanding_requests: Option<&OutstandingBlockRequests>,
        request_capacity: &mut usize,
    ) -> Option<Self> {
        let empty_outstanding_requests = BTreeMap::new(); // TODO gross, find another way
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
                let outstanding_requests = outstanding_requests.unwrap_or_else(|| &empty_outstanding_requests);
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
    piece_request: PeerRequestedPiece,
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
            piece_request
        };
        // push to expirations queue
        self.expirations.push_back((started, k));
        let block_map = self
            .piece_to_blocks_started
            .entry(k.piece_id)
            .or_insert_with(|| BTreeMap::default());
        let _ = block_map.insert(k.block_begin, download);
    }

    fn remove_request(&mut self, piece_id: u32, begin: u32) -> bool {
        self.inprogress_requests_by_piece_id(piece_id)
            .map(|reqs| reqs.remove(&begin)).is_some()
    }

    fn remove_piece(&mut self, piece_id: u32) -> bool {
        self.piece_to_blocks_started.remove(&piece_id).is_some()
    }

    fn remove_expired(&mut self, time: Instant, ttl: Duration) -> Vec<PeerRequestedPiece> {
        let mut acc = vec![];
        log::debug!("checking expirations on: {}", self.expirations.len());
        while let Some(ele @(entry_time, k)) = self.expirations.pop_front() {
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
                self.expirations.push_front(ele); // need to push it back
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

    fn outstanding_pieces(&self) -> impl Iterator<Item = u32> {
        self.piece_to_blocks_started.iter().map(|(k, _)| {
            *k
        })
    }

    fn inprogress_requests_by_piece_id(&mut self, piece_id: u32) -> Option<&mut OutstandingBlockRequests> {
        self
            .piece_to_blocks_started
            .get_mut(&piece_id)
    }
    
}

#[derive(Default, Debug)]
pub struct PieceBlockTracker {
    max_outstanding_requests: usize,
    request_timeout: Duration,
    pieces_not_started: HashSet<u32>,
    // mapping of piece_id to chunk start (begin in the protocol)
    piece_to_blocks_started: PieceToBlockMap,
    //piece_to_blocks_started: HashMap<u32, OutstandingBlockRequests>,
    pieces_completed: HashSet<u32>,
}

impl PieceBlockTracker {
    pub fn new(max_outstanding_requests: usize, pieces: &Vec<V1Piece>) -> Self {
        let pieces_not_started: HashSet<_> = (0..pieces.len()).map(|piece| piece as u32).collect();
        Self {
            max_outstanding_requests,
            pieces_not_started,
            request_timeout: Duration::from_secs(10),
            piece_to_blocks_started: Default::default(),
            pieces_completed: Default::default(),
        }
    }

    /// Iterator of unstarted or pieces that are started, but not completed. Prioritize started pieces
    pub fn get_incomplete_pieces(&self) -> impl Iterator<Item = u32> {
        let not_started_pieces = self
            .pieces_not_started
            .iter()
            .map(|id| *id);
        self
            .piece_to_blocks_started
            .outstanding_pieces()
            .chain(not_started_pieces)
    }

    pub fn get_pieces_completed(&self) -> &HashSet<u32> {
        &self.pieces_completed
    }

    pub fn pieces_not_started(&self) -> &HashSet<u32> {
        &self.pieces_not_started
    }

    pub fn pieces_completed_len(&self) -> usize {
        self.pieces_completed.len()
    }

    pub fn outstanding_requests_len(&self) -> usize {
        self.piece_to_blocks_started.all_outstanding_requests().len()
    }

    pub fn outstanding_pieces_len(&self) -> usize {
        self.piece_to_blocks_started.outstanding_pieces().count()
    }

    // TODO: function to remove requests that may have been dropped, due to conntection drop out or something else.
    pub fn remove_request(&mut self, req: PeerRequestedPiece) -> bool {
        self.piece_to_blocks_started.remove_request(req.index, req.begin)
    }

    pub fn set_request_finished(&mut self, piece_id: u32, begin: u32) {
        let _ = self.piece_to_blocks_started.remove_request(piece_id, begin);
    }

    pub fn set_piece_finished(&mut self, piece_id: u32) {
        let _ = self.piece_to_blocks_started.remove_piece(piece_id);
        let _ = self.pieces_completed.insert(piece_id);
    }

    // look for requests that have started, but not finished in request_timeout
    pub fn remove_expired(&mut self) -> Vec<PeerRequestedPiece> {
        let now = Instant::now();
        self.piece_to_blocks_started.remove_expired(now, self.request_timeout)
    }



    ///
    /// Function to generate the requests needed for a piece to be downloaded.
    /// Assumes that requests will be sent, and adds them for download tracking.
    fn assign_piece(&mut self, torrent: &V1Torrent, piece_id: u32, peer_ids: &HashSet<InternalPeerId>) -> Vec<PeerRequestedPiece> {
        let now = Instant::now();
        // currently this method is a bit flawed, we may need to resume piece tracking externally since the peer lists may change over time
        // as it stands we dont plan any initial requests after this method is called, and because of this we become stalled.
        let inflight_requests = self.outstanding_requests_len();
        let mut capacity = self.max_outstanding_requests - inflight_requests;
        let outstanding_requests = self
            .piece_to_blocks_started
            .inprogress_requests_by_piece_id(piece_id);

        let assignable_requests_opt = PieceBlockAllocation::new(
            piece_id, 
            torrent, 
            peer_ids, 
            outstanding_requests.as_deref(),
            &mut capacity,
        );

        if let Some(alloc) = assignable_requests_opt {
            let allocated_requests = alloc.requests_to_make;
            if !allocated_requests.is_empty() {
                // starting piece
                log::trace!("Allocateding REQs: {}", piece_id);
                let _ = self.pieces_not_started.remove(&piece_id);
            }
            for req in allocated_requests.iter() {
                log::trace!("Allocated REQ: {} :: {:?}", piece_id, allocated_requests);
                self.piece_to_blocks_started.insert(now, req.clone());
            }
            allocated_requests
        } else {
            vec![]
        }
    }

    pub fn generate_assignments(&mut self, torrent: &V1Torrent, availiable_piece_to_peers: &HashMap<u32, HashSet<InternalPeerId>>) -> Vec<PeerRequestedPiece> {
        availiable_piece_to_peers.iter().flat_map(|(piece_id, peers_with_piece)|{
            self.assign_piece(&torrent, *piece_id, peers_with_piece)
                .into_iter()
        }).collect::<Vec<_>>()

    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::peer::test::{torrent_fixture, torrent_fixture_impl};

    mod piece_block_tracker_tests {
        use super::*;

        fn gen_availiable_piece_to_peers(torrent: &V1Torrent) -> HashMap<u32, HashSet<InternalPeerId>> {
            let peers = vec![Arc::new(vec![0; 10])].into_iter().collect::<HashSet<_>>();
            torrent.info.pieces
                .iter()
                .enumerate()
                .map(|(idx, _)|{
                    (idx, peers.clone())
                }).fold(HashMap::new(), |mut acc, ele| {
                    let _ = acc.insert(ele.0 as u32, ele.1);
                    acc
                })
        }

        #[test]
        fn correctly_assigns_pieces_when_not_hitting_threshold() {
            let torrent = torrent_fixture(vec![1; 20]);
            let mut pbt = PieceBlockTracker::new(2, &torrent.info.pieces);

            let availiable_piece_to_peers = gen_availiable_piece_to_peers(&torrent);
            let assignments = pbt.generate_assignments(&torrent, &availiable_piece_to_peers);
            assert!(!assignments.is_empty());
            // Expect max allowed requests to be dispatched.
            // a piece may be larger than our max block size, so we may need more than one request to get one piece.
            assert_eq!(pbt.outstanding_requests_len(), 2);
            assert_eq!(pbt.outstanding_pieces_len(), 1);
            assert_eq!(pbt.pieces_not_started.len(), 1);
        }
        #[test]
        fn correctly_assigns_pieces_when_hitting_threshold() {
            env_logger::init();
            // Test shows we do not track requests that are complete!
            let pieces = 2;
            let max_outstanding_requests: i64 = 2;
            let torrent_len: i64 = (PIECE_BLOCK_SIZE * 4 * pieces) as _; // 4: requests per piece, 2 pieces total;
            let piece_len = torrent_len / pieces as i64;
            let expected_assignment_itterations = torrent_len / max_outstanding_requests;

            let torrent = torrent_fixture_impl(vec![1; 20], 2, piece_len, torrent_len);

            let mut pbt = PieceBlockTracker::new(max_outstanding_requests as usize, &torrent.info.pieces);
            let availiable_piece_to_peers = gen_availiable_piece_to_peers(&torrent);


            let mut requests_made: HashSet<PeerRequestedPiece> = HashSet::new();
            let mut ack_reqs: Vec<PeerRequestedPiece> = vec![];
            for i in 0..expected_assignment_itterations {
                // start of the loop we essentially are making the previous iterations requests as complete.
                for assigned in ack_reqs.drain(..).collect::<Vec<_>>() {
                    pbt.set_request_finished(assigned.index, assigned.begin);
                }
                let assignments = pbt.generate_assignments(&torrent, &availiable_piece_to_peers);
                println!("({}) assignments: {:?}", i, assignments);
                assert!(!assignments.is_empty());

                for req in assignments.iter() { // ensure we are not duplicating
                    assert!(requests_made.insert(req.clone()) == true); 
                }
                ack_reqs = assignments;
            }
        }

    }

    mod piece_block_alloc_tests {
        use super::*;
        #[test]
        fn create_correct_request_first_piece() {
            let mut request_capacity = 1024;
            let torrent = torrent_fixture(vec![1; 20]);
            let peer_id: InternalPeerId = Arc::new(vec![2; 20]);
            let piece_id = 0;
            let req = PieceBlockAllocation::new(
                piece_id,
                &torrent,
                &vec![peer_id.clone()].into_iter().collect(),
                None,
                &mut request_capacity,
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
            let mut request_capacity = 1024;
            let torrent = torrent_fixture(vec![1; 20]);
            let peer_id: InternalPeerId = Arc::new(vec![2; 20]);
            let piece_id = 1;
            let req = PieceBlockAllocation::new(
                piece_id,
                &torrent,
                &vec![peer_id.clone()].into_iter().collect(),
                None,
                &mut request_capacity,
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
}
