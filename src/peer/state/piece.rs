use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque, btree_map::Entry},
    ops::{Deref, DerefMut},
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    model::{InternalPeerId, PeerRequestedPiece, V1Piece, V1Torrent},
    peer::{PIECE_BLOCK_SIZE, TorrentAllocation},
};

// within a piece this maps the blocks using the "begin" as the key
type OutstandingBlockRequests = BTreeMap<u32, BlockDownload>;
//type PieceToBlockMap = HashMap<u32, OutstandingBlockRequests>;

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
        requested_requests: Option<&OutstandingBlockRequests>, // we don't want to re-request the same thing again and again, so filter on this
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
                if *request_capacity == 0 {
                    break;
                }
                let requested_requests =
                    requested_requests.unwrap_or_else(|| &empty_outstanding_requests);
                let already_requested = requested_requests.get(&(begin as u32)).is_some();
                if already_requested {
                    // skip based on the "begin" parameter and the piece_id, or no more request capacity is left.
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
    is_completed: bool,
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, PartialOrd)]
struct PieceToBlockKey {
    piece_id: u32,
    block_begin: u32,
}
// struct PieceToBlockMapEntry<'a> {
//     entry: Entry<'a, u32, BlockDownload>
// }
#[derive(Debug, Default)]
struct PieceToBlockMap {
    inprogress_requests: HashMap<u32, OutstandingBlockRequests>,
    //completed_requests: HashMap<u32, OutstandingBlockRequests>,
}

impl PieceToBlockMap {
    fn requests_by_piece_id(&mut self, piece_id: u32) -> Option<&mut OutstandingBlockRequests> {
        self.inprogress_requests.get_mut(&piece_id)
    }

    //fn request_entry(&mut self, piece_id: u32) -> 

    fn set_request_finished(&mut self, piece_id: u32, begin: u32) -> Option<()> {
        let requests = self
            .requests_by_piece_id(piece_id)?;
        let download = requests.get_mut(&begin)?;
        download.is_completed = true;
        Some(())
    }

    fn remove_request(&mut self, piece_id: u32, begin: u32) -> bool {
        self.requests_by_piece_id(piece_id)
            .map(|reqs| reqs.remove(&begin))
            .is_some()
    }

    fn remove_piece(&mut self, piece_id: u32) -> bool {
        self.inprogress_requests.remove(&piece_id).is_some()
    }

    fn requests(&self) -> impl Iterator<Item = BlockDownload> {
        self.inprogress_requests
            .iter()
            .flat_map(|(_, requests)| requests.iter().map(|(_, request)| request.clone()))
    }

    fn outstanding_requests_len(&self) -> usize {
        self
            .requests()
            .filter(|v| !v.is_completed)
            .count()
    }

    fn pieces(&self) -> impl Iterator<Item = u32> {
        self.inprogress_requests.iter().map(|(k, _)| *k)
    }

    fn insert_request(&mut self, piece_request: PeerRequestedPiece) {
        let download = BlockDownload {
            piece_request,
            is_completed: false,
        };
        let block_map = self
            .inprogress_requests
            .entry(download.piece_request.index)
            .or_insert_with(|| BTreeMap::default());
        let _ = block_map.insert(download.piece_request.begin, download);
    }

    fn get_request_entry<'a>(&'a mut self, piece_id: u32, begin: u32) -> Option<Entry<'a, u32, BlockDownload>> {
        self
            .inprogress_requests
            .get_mut(&piece_id)
            .map(|begin_map| {
                begin_map.entry(begin)
            })
    }
}

#[derive(Debug, Default)]
struct InprogressPieceToBlockMap {
    piece_to_blocks_started: PieceToBlockMap,
    expirations: VecDeque<(Instant, PieceToBlockKey)>,
}

impl Deref for InprogressPieceToBlockMap {
    type Target = PieceToBlockMap;

    fn deref(&self) -> &Self::Target {
        &self.piece_to_blocks_started
    }
}

impl DerefMut for InprogressPieceToBlockMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.piece_to_blocks_started
    }
}

impl InprogressPieceToBlockMap {
    fn inprogress_requests_by_piece_id(
        &mut self,
        piece_id: u32,
    ) -> Option<OutstandingBlockRequests> {
        let inprogress_requests_by_piece_id = {
            let requests_for_piece = self
                .piece_to_blocks_started
                .requests_by_piece_id(piece_id)?;
            let filtered = requests_for_piece
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect::<OutstandingBlockRequests>();

            Some(filtered)
        };
        inprogress_requests_by_piece_id
    }

    fn insert(&mut self, started: Instant, piece_request: PeerRequestedPiece) {
        let k = PieceToBlockKey {
            piece_id: piece_request.index,
            block_begin: piece_request.begin,
        };
        self.expirations.push_back((started, k));
        self.piece_to_blocks_started.insert_request(piece_request);
    }

    fn remove_expired(&mut self, time: Instant, ttl: Duration) -> Vec<PeerRequestedPiece> {
        let mut acc = vec![];
        log::debug!("checking expirations on: {}", self.expirations.len());
        while let Some(ele @ (entry_time, k)) = self.expirations.pop_front() {
            let lived_time = time
                .checked_duration_since(entry_time)
                .unwrap_or(Duration::ZERO);
            if lived_time >= ttl {
                if let Some(expired) = self
                    .piece_to_blocks_started
                    .get_request_entry(k.piece_id, k.block_begin)
                    .and_then(|entry| {
                        match entry {
                            Entry::Occupied(o) if !o.get().is_completed => {
                                // is expired
                                Some(o.remove())
                            }
                            _ => {
                                //if it was complete, leave it
                                None
                            }
                        }

                    }) {
                        acc.push(expired.piece_request)
                    }
            } else {
                self.expirations.push_front(ele); // need to push it back
                break;
            }
        }
        acc
    }
}

#[derive(Default, Debug)]
pub struct PieceBlockTracker {
    max_outstanding_requests: usize,
    request_timeout: Duration,
    pieces_not_started: HashSet<u32>,
    // mapping of piece_id to chunk start (begin in the protocol)
    piece_to_blocks_outstanding: InprogressPieceToBlockMap,
    pieces_completed: HashSet<u32>,
}

impl PieceBlockTracker {
    pub fn new(max_outstanding_requests: usize, pieces: &Vec<V1Piece>) -> Self {
        let pieces_not_started: HashSet<_> = (0..pieces.len()).map(|piece| piece as u32).collect();
        Self {
            max_outstanding_requests,
            pieces_not_started,
            request_timeout: Duration::from_secs(10),
            piece_to_blocks_outstanding: Default::default(),
            pieces_completed: Default::default(),
        }
    }

    /// Iterator of unstarted or pieces that are started, but not completed. Prioritize started pieces
    pub fn get_incomplete_pieces(&self) -> impl Iterator<Item = u32> {
        let not_started_pieces = self.pieces_not_started.iter().map(|id| *id);
        self.piece_to_blocks_outstanding
            .pieces()
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
        self.piece_to_blocks_outstanding.outstanding_requests_len()
    }

    pub fn outstanding_pieces_len(&self) -> usize {
        self.piece_to_blocks_outstanding.pieces().count()
    }

    // TODO: function to remove requests that may have been dropped, due to conntection drop out or something else.
    pub fn remove_request(&mut self, req: PeerRequestedPiece) -> bool {
        self.piece_to_blocks_outstanding
            .remove_request(req.index, req.begin)
    }

    pub fn set_request_finished(&mut self, piece_id: u32, begin: u32) -> Option<()> {
        self.piece_to_blocks_outstanding.set_request_finished(piece_id, begin)
    }

    pub fn set_piece_finished(&mut self, piece_id: u32) {
        let _ = self.piece_to_blocks_outstanding.remove_piece(piece_id);
        let _ = self.pieces_completed.insert(piece_id);
    }

    // look for requests that have started, but not finished in request_timeout
    pub fn remove_expired(&mut self) -> Vec<PeerRequestedPiece> {
        let now = Instant::now();
        self.piece_to_blocks_outstanding
            .remove_expired(now, self.request_timeout)
    }

    ///
    /// Function to generate the requests needed for a piece to be downloaded.
    /// Assumes that requests will be sent, and adds them for download tracking.
    fn assign_piece(
        &mut self,
        torrent: &V1Torrent,
        piece_id: u32,
        peer_ids: &HashSet<InternalPeerId>,
    ) -> Vec<PeerRequestedPiece> {
        let now = Instant::now();
        // currently this method is a bit flawed, we may need to resume piece tracking externally since the peer lists may change over time
        // as it stands we dont plan any initial requests after this method is called, and because of this we become stalled.
        let inflight_requests = self.outstanding_requests_len();
        let mut capacity = self.max_outstanding_requests - inflight_requests;
        let requested_requests = self
            .piece_to_blocks_outstanding
            .inprogress_requests_by_piece_id(piece_id);

        let assignable_requests_opt = PieceBlockAllocation::new(
            piece_id,
            torrent,
            peer_ids,
            requested_requests.as_ref(),
            &mut capacity,
        );

        if let Some(alloc) = assignable_requests_opt {
            let allocated_requests = alloc.requests_to_make;
            if !allocated_requests.is_empty() {
                // starting piece, so remove it from the starting bucket.
                let _ = self.pieces_not_started.remove(&piece_id);
            }
            for req in allocated_requests.iter() {
                log::trace!("Allocated REQ: {} :: {:?}", piece_id, req);
                self.piece_to_blocks_outstanding.insert(now, req.clone());
            }
            allocated_requests
        } else {
            vec![]
        }
    }

    pub fn generate_assignments(
        &mut self,
        torrent: &V1Torrent,
        availiable_piece_to_peers: &BTreeMap<u32, HashSet<InternalPeerId>>,
    ) -> Vec<PeerRequestedPiece> {
        availiable_piece_to_peers
            .iter()
            .flat_map(|(piece_id, peers_with_piece)| {
                self.assign_piece(&torrent, *piece_id, peers_with_piece)
                    .into_iter()
            })
            .collect::<Vec<_>>()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::peer::test::{torrent_fixture, torrent_fixture_impl};

    mod piece_block_tracker_tests {
        use std::thread::sleep;

        use super::*;

        fn gen_availiable_piece_to_peers(
            torrent: &V1Torrent,
        ) -> BTreeMap<u32, HashSet<InternalPeerId>> {
            let peers = vec![Arc::new(vec![0; 10])]
                .into_iter()
                .collect::<HashSet<_>>();
            torrent
                .info
                .pieces
                .iter()
                .enumerate()
                .map(|(idx, _)| (idx, peers.clone()))
                .fold(BTreeMap::new(), |mut acc, ele| {
                    let _ = acc.insert(ele.0 as u32, ele.1);
                    acc
                })
        }

        fn setup_piece_block_tracker() -> (PieceBlockTracker, V1Torrent, usize) {
            let pieces = 2;
            let max_outstanding_requests: i64 = 2;
            let requests_per_piece = 4;
            let torrent_len: i64 = (PIECE_BLOCK_SIZE * requests_per_piece * pieces) as _; // 4: requests per piece, 2 pieces total;
            let piece_len = torrent_len / pieces as i64;
            let torrent = torrent_fixture_impl(vec![1; 20], 2, piece_len, torrent_len);

            let pbt =
                PieceBlockTracker::new(max_outstanding_requests as usize, &torrent.info.pieces);
            (pbt, torrent, requests_per_piece)
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
        fn correctly_expires_completed_requests_by_not_expiring() {
            let (mut pbt, torrent, _) = setup_piece_block_tracker();
            let availiable_piece_to_peers = gen_availiable_piece_to_peers(&torrent);

            let assignments = pbt.generate_assignments(&torrent, &availiable_piece_to_peers);
            assert!(!assignments.is_empty());

            for assigned in assignments {
                pbt.set_request_finished(assigned.index, assigned.begin)
                    .unwrap();
            }
            sleep(Duration::from_secs(11)); // todo mock instant to remove this

            let expired = pbt.remove_expired(); // set_request_finished was called, expect not expirations
            assert!(expired.is_empty());
        }

        #[test]
        fn correctly_expires_incompleted_requests_by_expiring() {
            // centralize boilerplate
            let (mut pbt, torrent, _) = setup_piece_block_tracker();
            let availiable_piece_to_peers = gen_availiable_piece_to_peers(&torrent);

            let assignments = pbt.generate_assignments(&torrent, &availiable_piece_to_peers);
            assert!(!assignments.is_empty());

            sleep(Duration::from_secs(11)); // todo mock instant to remove this

            let expired = pbt.remove_expired(); // set_request_finished was called, expect not expirations
            assert!(expired.len() == 2);
        }

        #[test]
        fn correctly_assigns_pieces_when_hitting_threshold() {
            // Test shows we do not track requests that are complete!
            let (mut pbt, torrent, requests_per_piece) = setup_piece_block_tracker();
            let availiable_piece_to_peers = gen_availiable_piece_to_peers(&torrent);

            let mut requests_made: HashSet<PeerRequestedPiece> = HashSet::new();
            let mut ack_reqs: Vec<PeerRequestedPiece> = vec![];
            for _ in 0..requests_per_piece {
                // start of the loop we essentially are making the previous iterations requests as complete.
                for assigned in ack_reqs.drain(..).collect::<Vec<_>>() {
                    pbt.set_request_finished(assigned.index, assigned.begin)
                        .unwrap();
                }
                let assignments = pbt.generate_assignments(&torrent, &availiable_piece_to_peers);
                assert!(!assignments.is_empty());

                for req in assignments.iter() {
                    // ensure we are not duplicating
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
