use crate::model::{V1Piece, V1Torrent};
use crate::peer::{PIECE_BLOCK_SIZE, TorrentAllocation};
use crate::peer::bitfield::{BitFieldReader, BitFieldReaderIter};
use crate::peer::file::TorrentWriter;
use crate::peer::protocol::{FlagMessages, Handshake};
use anyhow::Result;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use futures::task::waker;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::oneshot::{Receiver as OReceiver, Sender as OSender};

use super::protocol::Messages;

type PeerId = Vec<u8>;
type InternalPeerId = Arc<PeerId>;
type PeerToSender = HashMap<InternalPeerId, Sender<Messages>>;

const MAX_OUTSTANDING_REQUESTS: usize = 4;

#[derive(Default, Debug)]
struct PeerPieceMap {
    pieces_to_peers: HashMap<u32, HashSet<InternalPeerId>>,
    peers_to_pieces: HashMap<InternalPeerId, HashSet<u32>>,
}

impl PeerPieceMap {
    pub fn add_piece(&mut self, peer_id: InternalPeerId, piece: u32) {
        let ptb = self
            .peers_to_pieces
            .entry(Arc::clone(&peer_id))
            .or_insert_with(|| HashSet::new());
        ptb.insert(piece);

        let btp = self
            .pieces_to_peers
            .entry(piece)
            .or_insert_with(|| HashSet::new());
        btp.insert(peer_id);
    }

    pub fn get_peers_with_piece(&self, piece: u32) -> HashSet<InternalPeerId> {
        if let Some(peers) = self.pieces_to_peers.get(&piece) {
            peers
                .iter()
                .map(|peer| Arc::clone(peer))
                .collect::<HashSet<_>>()
        } else {
            HashSet::new()
        }
    }

    pub fn remove_piece(&mut self, peer_id: InternalPeerId, piece: u32) {
        let ptb = self
            .peers_to_pieces
            .entry(Arc::clone(&peer_id))
            .or_insert_with(|| HashSet::new());
        ptb.remove(&piece);

        let btp = self
            .pieces_to_peers
            .entry(piece)
            .or_insert_with(|| HashSet::new());
        btp.remove(&peer_id);
    }

    pub fn remove_peer(&mut self, peer_id: &InternalPeerId) {
        if let Some(blocks) = self.peers_to_pieces.remove(peer_id) {
            for block in blocks.into_iter() {
                if let Some(peers) = self.pieces_to_peers.get_mut(&block) {
                    peers.remove(peer_id);
                }
            }
        }
    }
}

#[derive(Debug, Default)]
struct TorrentState {
    peers: HashSet<InternalPeerId>,
    peers_interested: HashSet<InternalPeerId>,
    peers_not_choking: HashSet<InternalPeerId>,
    block_peer_map: PeerPieceMap,
    /// piece tracking
    pieces_not_started: HashSet<u32>,
    pieces_started: HashSet<u32>,
    pieces_finished: HashSet<u32>,
}

impl TorrentState {
    pub fn new(pieces: &Vec<V1Piece>) -> Self {
        let pieces_not_started: HashSet<_> = (0..pieces.len())
            .map(|piece| piece as u32)
            .collect();
        Self{
            pieces_not_started,
            ..Default::default()
        }

    }
    fn add_peer_id(&mut self, peer_id: InternalPeerId) {
        if !self.peers.contains(&peer_id) {
            self.peers.insert(peer_id);
        }
    }

    pub fn set_peer_choked_us(&mut self, peer_id: InternalPeerId, choked: bool) {
        self.add_peer_id(Arc::clone(&peer_id));
        if !choked {
            self.peers_not_choking.insert(peer_id);
        } else {
            self.peers_not_choking.remove(&peer_id);
        }
    }

    pub fn set_peers_interested_in_us(&mut self, peer_id: InternalPeerId, interested: bool) {
        self.add_peer_id(Arc::clone(&peer_id));
        if interested {
            self.peers_interested.insert(peer_id);
        } else {
            self.peers_interested.remove(&peer_id);
        }
    }

    pub fn peers_that_choke(&self, choke: bool) -> HashSet<InternalPeerId> {
        if choke {
            return self
                .peers
                .difference(&self.peers_not_choking)
                .map(|a| Arc::clone(a))
                .collect();
        } else {
            return self
                .peers
                .intersection(&self.peers_not_choking)
                .map(|a| Arc::clone(a))
                .collect();
        }
    }

    pub fn peers_that_are_interested(&self, interested: bool) -> HashSet<InternalPeerId> {
        if !interested {
            return self
                .peers
                .difference(&self.peers_interested)
                .map(|a| Arc::clone(a))
                .collect();
        } else {
            return self
                .peers
                .intersection(&self.peers_interested)
                .map(|a| Arc::clone(a))
                .collect();
        }
    }

    pub fn add_pieces_for_peer(&mut self, peer_id: InternalPeerId, pieces: Vec<u32>) {
        self.add_peer_id(Arc::clone(&peer_id));
        for piece in pieces {
            self.block_peer_map.add_piece(Arc::clone(&peer_id), piece);
        }
    }

    pub fn set_pieces_started(&mut self, pieces: HashSet<u32>) {
        self.pieces_not_started = self.pieces_not_started
            .difference(&pieces)
            .map(|piece| *piece)
            .collect::<HashSet<_>>()
    }

    pub fn peer_willing_to_upload_pieces(&mut self) -> HashMap<u32, HashSet<InternalPeerId>> {
        let peer_we_can_download_from = self.peers_that_choke(false);
        self.pieces_not_started.iter().map(|outstanding_block| {
            let peers_with_piece = self
                .block_peer_map
                .get_peers_with_piece(*outstanding_block);
            let peers_with_piece_and_not_choked = peer_we_can_download_from
                .intersection(&peers_with_piece)
                .map(|id| Arc::clone(id))
                .collect::<HashSet<_>>();
            (*outstanding_block, peers_with_piece_and_not_choked)
        }).collect()
    }

    pub fn remove_peer(&mut self, peer_id: InternalPeerId) -> Option<PeerId> {
        self.add_peer_id(Arc::clone(&peer_id));
        self.peers.remove(&peer_id);
        self.peers_interested.remove(&peer_id);
        self.peers_not_choking.remove(&peer_id);
        self.block_peer_map.remove_peer(&peer_id);

        return Arc::into_inner(peer_id);
    }
}

///
/// Messages
pub struct ProtocolMessage {
    msg: Messages,
}

///
/// bidirectional channels for a single peers connection
struct PeerStartMessage {
    handshake: Handshake,
    rx: Receiver<ProtocolMessage>,
    tx: Sender<Messages>,
}

impl PeerStartMessage {
    fn into(self) -> (Handshake, Receiver<ProtocolMessage>, Sender<Messages>) {
        (self.handshake, self.rx, self.tx)
    }
}

struct PeerRequestedPiece {
    peer_id: InternalPeerId,
    index: u32,
    begin: u32,
    length: u32,
}

struct PieceBlockTracking {
    requests_to_make: Vec<PeerRequestedPiece>,
}

impl PieceBlockTracking {
    pub fn new(piece_id: u32, torrent: &V1Torrent, peer_ids: HashSet<InternalPeerId>) -> Option<Self> {
        if let Some(peer_id) = peer_ids.iter().next() {
            let mut requests_to_make = vec![];
            let allocation = TorrentAllocation::allocate_torrent(torrent);
            let piece_size = if piece_id == (torrent.info.pieces.len() - 1) as u32 {
                allocation.last_piece_size
            } else {
                allocation.max_piece_size
            };
            for begin in (0..piece_size).step_by(PIECE_BLOCK_SIZE) {
                let length = PIECE_BLOCK_SIZE;
                requests_to_make.push(PeerRequestedPiece{
                    peer_id: Arc::clone(&peer_id),
                    index: piece_id,
                    begin: begin as _,
                    length: length as _,

                });
            }
            Some(Self {
                requests_to_make
            })
        } else {
            None
        }
    }
}


struct TorrentProcessor<W> {
    torrent: V1Torrent,
    torrent_state: TorrentState,
    torrent_writer: W,
}

impl<W: TorrentWriter> TorrentProcessor<W> {
    pub fn new(torrent: V1Torrent, torrent_writer: W) -> Self {
        let torrent_state = TorrentState::new(&torrent.info.pieces);
        Self {
            torrent,
            torrent_state,
            torrent_writer,
        }
        
    }
    pub async fn start(mut self, mut rx: Receiver<PeerStartMessage>) {
        let state = Arc::new(Mutex::new(self));
        let mut peer_to_tx: PeerToSender = HashMap::new();
        let mut handle_peer_requests_fq = FuturesUnordered::new();

        loop {
            tokio::select! {
                Some(new) = rx.recv() => {
                    let (handshake, rx, tx) = new.into();
                    peer_to_tx.insert(Arc::new(handshake.peer_ctx.peer_id.clone()), tx);
                    handle_peer_requests_fq.push(Self::handle_peer_msgs(Arc::clone(&state), rx, handshake));
                }
                Some((peer_id, res)) = handle_peer_requests_fq.next() => {
                    peer_to_tx.remove(&peer_id);
                    log::info!("Peer: {} finished {:?}", hex::encode(peer_id), res);
                    ()
                }
                else => break,
            }

            Self::compute_requests(Arc::clone(&state), &mut peer_to_tx).await;
        }
    }

    async fn compute_requests(state: Arc<Mutex<Self>>, peer_to_tx: &mut PeerToSender) {
        let mut state = state.lock().await;
        let torrent = state.torrent.clone();
        let block_to_request_tracker = state
            .torrent_state
            .peer_willing_to_upload_pieces()
            .into_iter()
            .map(|(piece_id, peers)| {
                (piece_id, PieceBlockTracking::new(piece_id, &torrent, peers))
            }).filter_map(|(p, tracker_opt)| {
                tracker_opt.map(|t| (p, t))
            }).take(MAX_OUTSTANDING_REQUESTS)
            .collect::<HashMap<_, _>>();

        let mut peers_closed = HashSet::new();

        for (k, v) in block_to_request_tracker.into_iter() {
            log::debug!("Dispatch request for piece_id {}", k);
            for req in v.requests_to_make {
                if let Some(tx) = peer_to_tx.get(&req.peer_id) {
                    let req_msg = Messages::Request { index: req.index, begin: req.begin, length: req.length };
                    log::debug!("Sending request {:?}", req_msg);
                    if let Err(_) = tx.send(req_msg).await {
                        peers_closed.insert(req.peer_id);
                    }
                }
            }
        }

        log::debug!("Peers with closed channels: {:?}", peers_closed);

        for peer_closed in peers_closed.into_iter() {
            peer_to_tx.remove(&peer_closed);
        }
    }

    ///
    /// Maps the result so that we always return the peer_id
    async fn handle_peer_msgs(
        state: Arc<Mutex<Self>>,
        rx: Receiver<ProtocolMessage>,
        handshake: Handshake,
    ) -> (PeerId, Result<()>) {
        let peer_id = handshake.peer_ctx.peer_id.clone();
        let res = Self::inner_handle_peer_msgs(state, rx, handshake).await;
        (peer_id, res)
    }

    ///
    /// Handle all incoming state updates from a single peer, and requests
    async fn inner_handle_peer_msgs(
        state: Arc<Mutex<Self>>,
        mut rx: Receiver<ProtocolMessage>,
        handshake: Handshake,
    ) -> Result<()> {
        log::info!("Starting torrent processing...");
        let peer_id = Arc::new(handshake.peer_ctx.peer_id);
        let mut outstanding_requests = vec![];

        while let Some(msg) = rx.recv().await {
            let peer_id = Arc::clone(&peer_id);
            let mut state = state.lock().await;
            match msg.msg {
                Messages::KeepAlive => {
                    // TODO reset timer or something, and expire after xx time
                }
                Messages::Flag(flag) => match flag {
                    FlagMessages::Choke => state.torrent_state.set_peer_choked_us(peer_id, true),
                    FlagMessages::Unchoke => state.torrent_state.set_peer_choked_us(peer_id, false),
                    FlagMessages::Interested => state
                        .torrent_state
                        .set_peers_interested_in_us(peer_id, true),
                    FlagMessages::NotInterested => state
                        .torrent_state
                        .set_peers_interested_in_us(peer_id, false),
                },
                Messages::Have { piece_index } => {
                    state
                        .torrent_state
                        .add_pieces_for_peer(peer_id, vec![piece_index]);
                }
                Messages::BitField { bitfield } => {
                    let bitfield: BitFieldReaderIter = BitFieldReader::from(bitfield).into();
                    let pieces_present = bitfield
                        .into_iter()
                        .enumerate()
                        .filter(|(_, was_set)| *was_set)
                        .map(|(block, _)| block as u32)
                        .collect::<Vec<_>>();
                    state
                        .torrent_state
                        .add_pieces_for_peer(peer_id, pieces_present);
                }
                Messages::Request {
                    index,
                    begin,
                    length,
                } => {
                    outstanding_requests.push(PeerRequestedPiece {
                        peer_id,
                        index,
                        begin,
                        length,
                    });
                }
                Messages::Cancel {
                    index,
                    begin,
                    length,
                } => {
                    // attempt to cancel
                    let idx_to_remove_opt = outstanding_requests
                        .iter()
                        .enumerate()
                        .find(|(_, o)| {
                            o.peer_id == peer_id
                                && o.index == index
                                && o.begin == begin
                                && o.length == length
                        })
                        .map(|(idx, _)| idx);

                    if let Some(idx) = idx_to_remove_opt {
                        outstanding_requests.remove(idx);
                    }
                }
                Messages::Piece {
                    index,
                    begin,
                    block,
                } => {
                    state
                        .torrent_writer
                        .write_piece(index, begin, block)
                        .await?;
                }
            }
        }

        Ok(())
    }
}
