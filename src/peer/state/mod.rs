mod piece;
use crate::config::Config;
use crate::model::{InternalPeerId, PeerId};
use crate::model::{PeerRequestedPiece, V1Piece, V1Torrent};
use crate::peer::bitfield::{BitFieldReader, BitFieldReaderIter, BitFieldWriter};
use crate::peer::io::IoHandler;
use crate::peer::protocol::{FlagMessages, Handshake};
use crate::peer::state::piece::PieceBlockTracker;
use anyhow::Result;
use bytes::BytesMut;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::{Mutex, mpsc};

use super::protocol::Messages;

type PeerToSender = HashMap<InternalPeerId, Sender<Messages>>;

// Unlocking before the next async boundry is important, so this aims to make any channel send coupled with a drop.
macro_rules! unlock_and_send {
    ($tx:ident, $locked:ident, $send_this: expr) => {
        drop($locked); // dont hold the lock over a blocking call
        $tx.send($send_this).await?;
    };
}

/// TODOs:
/// * Handle cases where peer closes connection to us.
/// * Ability to ask for more peer connections when connections close.
/// (c) Allow for started peices to expire and re-enter the pieces_not_done set again.

///
/// Messages

///
/// bidirectional channels for a single peers connection
pub struct PeerStartMessage {
    pub handshake: Handshake,
    pub rx: Receiver<Messages>,
    pub tx: Sender<Messages>,
}

impl PeerStartMessage {
    fn into(self) -> (Handshake, Receiver<Messages>, Sender<Messages>) {
        (self.handshake, self.rx, self.tx)
    }
}

enum InternalStateMessage {
    Wakeup, // signal that a state change has occured (ie. choke/unchoke, interest/uninterest, bitfield/has)
    PieceComplete { piece_id: u32 },
    PeerRequestedPiece(PeerRequestedPiece),
}

///
/// Torrent state tracking
#[derive(Default, Debug)]
struct PeerPieceMap {
    pieces_to_peers: HashMap<u32, HashSet<InternalPeerId>>,
    peers_to_pieces: HashMap<InternalPeerId, HashSet<u32>>,
}

impl PeerPieceMap {
    pub fn peer_interest(
        &self,
        peer_id: &InternalPeerId,
        outstanding_pieces: &HashSet<u32>,
    ) -> HashSet<u32> {
        if let Some(peer_pieces) = self.peers_to_pieces.get(peer_id) {
            outstanding_pieces
                .intersection(peer_pieces)
                .map(|piece| *piece)
                .collect::<HashSet<_>>()
        } else {
            HashSet::new()
        }
    }
    pub fn add_piece(&mut self, peer_id: InternalPeerId, piece: u32) {
        let ptb = self
            .peers_to_pieces
            .entry(Arc::clone(&peer_id))
            .or_insert_with(|| HashSet::new());
        let _ = ptb.insert(piece);

        let btp = self
            .pieces_to_peers
            .entry(piece)
            .or_insert_with(|| HashSet::new());
        let _ = btp.insert(peer_id);
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

    pub fn remove_peer(&mut self, peer_id: &InternalPeerId) {
        if let Some(blocks) = self.peers_to_pieces.remove(peer_id) {
            for block in blocks.into_iter() {
                if let Some(peers) = self.pieces_to_peers.get_mut(&block) {
                    let _ = peers.remove(peer_id);
                }
            }
        }
    }
}
///
///
#[derive(Debug, Default)]
struct InternalPeerState {
    choked: bool,     // we are choking
    interested: bool, // we are interested
}

#[derive(Debug, Default)]
struct TorrentState {
    piece_block_tracking: PieceBlockTracker,
    // Defaults below
    peers_interested: HashSet<InternalPeerId>, // they are interested in us
    peers_not_choking: HashSet<InternalPeerId>, // they are choking us
    block_peer_map: PeerPieceMap,
    peers: HashMap<InternalPeerId, InternalPeerState>,
    outstanding_peer_requests: VecDeque<PeerRequestedPiece>,
}

impl TorrentState {
    pub fn new(pieces: &Vec<V1Piece>, max_outstanding_requests: usize) -> Self {
        let piece_block_tracking = PieceBlockTracker::new(max_outstanding_requests, pieces);
        Self {
            piece_block_tracking,
            ..Default::default()
        }
    }
    fn add_peer_id(&mut self, peer_id: InternalPeerId) {
        if !self.peers.contains_key(&peer_id) {
            let _ = self.peers.insert(peer_id, InternalPeerState::default());
        }
    }

    pub fn set_peer_choked_us(&mut self, peer_id: InternalPeerId, choked: bool) {
        self.add_peer_id(Arc::clone(&peer_id));
        if !choked {
            let _ = self.peers_not_choking.insert(peer_id);
        } else {
            let _ = self.peers_not_choking.remove(&peer_id);
        }
    }

    pub fn set_peers_interested_in_us(&mut self, peer_id: InternalPeerId, interested: bool) {
        log::debug!("interest is set {}", interested);
        self.add_peer_id(Arc::clone(&peer_id));
        if interested {
            let _ = self.peers_interested.insert(peer_id);
        } else {
            let _ = self.peers_interested.remove(&peer_id);
        }
    }

    fn peer_set(&self) -> HashSet<InternalPeerId> {
        self.peers.keys().map(|k| Arc::clone(k)).collect()
    }

    pub fn peers_that_choke(&self, choke: bool) -> HashSet<InternalPeerId> {
        if choke {
            return self
                .peer_set()
                .difference(&self.peers_not_choking)
                .map(|a| Arc::clone(a))
                .collect();
        } else {
            return self
                .peer_set()
                .intersection(&self.peers_not_choking)
                .map(|a| Arc::clone(a))
                .collect();
        }
    }

    pub fn peers_that_are_interested(&self, interested: bool) -> HashSet<InternalPeerId> {
        if !interested {
            return self
                .peer_set()
                .difference(&self.peers_interested)
                .map(|a| Arc::clone(a))
                .collect();
        } else {
            return self
                .peer_set()
                .intersection(&self.peers_interested)
                .map(|a| Arc::clone(a))
                .collect();
        }
    }

    ///
    /// If peer send bitfield or has message then call this, returns "our interest".
    /// if some, then interest has changed
    /// if none, no change happened
    pub fn add_pieces_for_peer(
        &mut self,
        peer_id: InternalPeerId,
        pieces: Vec<u32>,
    ) -> Option<bool> {
        self.add_peer_id(Arc::clone(&peer_id));
        let prev_interest = !self
            .block_peer_map
            .peer_interest(&peer_id, self.piece_block_tracking.pieces_not_started())
            .is_empty();
        for piece in pieces {
            self.block_peer_map.add_piece(Arc::clone(&peer_id), piece);
        }
        let interest = !self
            .block_peer_map
            .peer_interest(&peer_id, self.piece_block_tracking.pieces_not_started())
            .is_empty();
        if prev_interest != interest {
            (*self
                .peers
                .entry(Arc::clone(&peer_id))
                .or_insert_with(|| InternalPeerState::default()))
            .interested = interest;
            Some(interest)
        } else {
            None
        }
    }

    ///
    /// Our view on the peers conencted to use
    pub fn get_internal_peer_state(&self, peer_id: &InternalPeerId) -> Option<&InternalPeerState> {
        self.peers.get(peer_id)
    }

    pub fn get_internal_peer_state_mut(
        &mut self,
        peer_id: &InternalPeerId,
    ) -> Option<&mut InternalPeerState> {
        self.peers.get_mut(peer_id)
    }

    // ///
    // /// returns a map where the key is the piece_id, and value is the peers that HAVE that piece
    pub fn piece_id_to_peers(&mut self) -> BTreeMap<u32, HashSet<InternalPeerId>> {
        let peer_we_can_download_from = self.peers_that_choke(false);
        self.piece_block_tracking
            .get_incomplete_pieces()
            .map(|outstanding_piece| {
                let peers_with_piece = self.block_peer_map.get_peers_with_piece(outstanding_piece);
                let peers_with_piece_and_not_choked = peer_we_can_download_from
                    .intersection(&peers_with_piece)
                    .map(|id| Arc::clone(id))
                    .collect::<HashSet<_>>();
                (outstanding_piece, peers_with_piece_and_not_choked)
            })
            .collect()
    }

    pub fn remove_peer(&mut self, peer_id: InternalPeerId) -> Option<PeerId> {
        self.add_peer_id(Arc::clone(&peer_id));
        let _ = self.peers.remove(&peer_id);
        let _ = self.peers_interested.remove(&peer_id);
        let _ = self.peers_not_choking.remove(&peer_id);
        let _ = self.block_peer_map.remove_peer(&peer_id);

        return Arc::into_inner(peer_id);
    }
}

pub type InternalTorrentWriter = IoHandler;

struct InnerTorrentState {
    torrent: V1Torrent,
    torrent_state: TorrentState,
}
pub struct TorrentProcessor {
    our_id: InternalPeerId,
    torrent: V1Torrent,
    torrent_state: TorrentState,
    io: IoHandler,
}

impl TorrentProcessor {
    pub fn new(config: Config, our_id: InternalPeerId, torrent: V1Torrent, io: IoHandler) -> Self {
        let torrent_state =
            TorrentState::new(&torrent.info.pieces, config.max_outstanding_requests);
        Self {
            our_id,
            torrent,
            torrent_state,
            io,
        }
    }
    fn into(self) -> (InnerTorrentState, IoHandler) {
        (
            InnerTorrentState {
                torrent: self.torrent,
                torrent_state: self.torrent_state,
            },
            self.io,
        )
    }
    pub async fn start(self, mut rx: Receiver<PeerStartMessage>) -> Result<()> {
        log::info!(
            "torrent state processor starting... {}",
            hex::encode(self.our_id.as_ref())
        );
        let (state, io) = self.into();
        let state = Arc::new(Mutex::new(state));
        let mut peer_to_tx: PeerToSender = HashMap::new();
        let mut handle_peer_requests_fq = FuturesUnordered::new();
        let (wake_up_tx, mut wake_up_rx) = mpsc::channel(10);

        let mut timeout_fut = tokio::time::interval(Duration::from_secs(5));

        loop {
            tokio::select! {
                Some(new) = rx.recv() => {
                    let (handshake, rx, mut tx) = new.into();
                    log::info!("Accepting peer {}", hex::encode(&handshake.peer_ctx.peer_id));
                    // send peer back state needed, pieces we have and the choke, interested
                    if let Ok(_) = Self::init_connection(Arc::clone(&state), &mut tx).await {
                        log::info!("initial messages sent to {}", hex::encode(&handshake.peer_ctx.peer_id));
                        let _ = peer_to_tx.insert(Arc::new(handshake.peer_ctx.peer_id.clone()), tx.clone());
                        handle_peer_requests_fq.push(Self::handle_peer_msgs(Arc::clone(&state), io.clone(), rx, handshake, tx, wake_up_tx.clone()));
                        // todo each peer needs a keep-alive timer for times of inactivity
                    } else {
                        log::warn!("Unable to initialize connection {:?}", handshake);
                    }
                }
                Some((peer_id, res)) = handle_peer_requests_fq.next() => {
                    let state = Arc::clone(&state);
                    let mut  state = state.lock().await;
                    let _ = peer_to_tx.remove(&peer_id);
                    let _ = state.torrent_state.remove_peer(Arc::clone(&peer_id));
                    log::info!("Peer: {} closed {:?}", hex::encode(peer_id.as_ref()), res);
                    ()
                }
                Some(msg) = wake_up_rx.recv() => {
                    Self::prune_closed_tx(&mut peer_to_tx);

                    match msg {
                        InternalStateMessage::Wakeup => {
                            //log::debug!("Wake up from {:?}", hex::encode(peer_id.as_ref()));
                        },
                        InternalStateMessage::PieceComplete{ piece_id } => {
                            Self::announce_piece(&mut peer_to_tx, piece_id).await;
                        },
                        InternalStateMessage::PeerRequestedPiece(request) => {
                            Self::queue_requests(Arc::clone(&state), request).await;
                        }
                    }
                    Self::compute_requests_for_download(Arc::clone(&state), &mut peer_to_tx).await;
                    if let Err(err) = Self::dequeue_requests(Arc::clone(&state), &mut peer_to_tx, io.clone()).await {
                        log::warn!("Unable to read: {:?}", err);
                    }
                },
                _ = timeout_fut.tick() => {
                    Self::compute_unchoke(Arc::clone(&state), &mut peer_to_tx).await;
                    let mut locked_state = state.lock().await;
                    let expired = locked_state.torrent_state.piece_block_tracking.remove_expired();
                    let pieces_completed = locked_state.torrent_state.piece_block_tracking.pieces_completed_len();
                    let percent_completed = ((pieces_completed as f64) / (locked_state.torrent.info.pieces.len() as f64)) * 100.0;
                    log::info!("Timer stats: pieces_not_started={}, outstanding_requests={}, outstanding_pieces={} pieces_finished={}, percent_finished={}",
                        locked_state.torrent_state.piece_block_tracking.pieces_not_started().len(),
                        locked_state.torrent_state.piece_block_tracking.downloading_requests_len(),
                        locked_state.torrent_state.piece_block_tracking.outstanding_pieces_len(),
                        pieces_completed,
                        percent_completed
                    );

                    if !expired.is_empty() { // TODO handle expired requests
                        // TODO Queue cancel message for this peer
                        log::warn!("Expired requests: {}", expired.len());
                        unlock_and_send!(wake_up_tx, locked_state, {
                            InternalStateMessage::Wakeup
                        });
                    }
                }
                else => break,
            }
            log::trace!("Select finished");
        }
        log::info!("Closing main state loop...");
        Ok(())
    }

    async fn compute_unchoke(state: Arc<Mutex<InnerTorrentState>>, peer_to_tx: &mut PeerToSender) {
        let ideal_stationary_choke_size = 4;
        let mut locked_state = state.lock().await;
        let interested_peers = locked_state
            .torrent_state
            .peers_that_are_interested(true)
            .into_iter()
            .collect::<HashSet<_>>();

        if !interested_peers.is_empty() {
            let unchoked_peers = locked_state
                .torrent_state
                .peers
                .iter()
                .filter(|(_, state)| !state.choked)
                .map(|(k, _)| Arc::clone(k))
                .collect::<HashSet<_>>();
            let stationary_peer_set = unchoked_peers
                .union(&interested_peers)
                .map(|k| Arc::clone(k))
                .collect::<HashSet<_>>();
            //stationary_peer_set.sort();
            let stationary_peer_set_len = stationary_peer_set.len();
            log::debug!(
                "Interested peers: {:?}",
                interested_peers
                    .iter()
                    .map(|id| hex::encode(id.as_ref()))
                    .collect::<Vec<_>>()
            );

            if stationary_peer_set_len < ideal_stationary_choke_size {
                let new_potential_peers = interested_peers.difference(&stationary_peer_set);

                let mut combined_peers = new_potential_peers
                    .take(ideal_stationary_choke_size - stationary_peer_set_len)
                    .map(|k| (Arc::clone(k), false))
                    .collect::<Vec<_>>(); //unchokable

                let mut peers_that_left = unchoked_peers
                    .difference(&stationary_peer_set)
                    .map(|k| (Arc::clone(&k), true))
                    .collect::<Vec<_>>(); // need to choke

                log::info!(
                    "Peers choke report: peers_to_unchoke={:?}, peers_to_choke={:?}",
                    combined_peers,
                    peers_that_left
                );

                combined_peers.append(&mut peers_that_left);

                for (peer_id, choked) in combined_peers {
                    if let Some(peer) = locked_state.torrent_state.peers.get_mut(&peer_id) {
                        peer.choked = choked;
                    }
                    if let Some(tx) = peer_to_tx.get(&peer_id) {
                        let msg = match choked {
                            true => FlagMessages::Choke,
                            _ => FlagMessages::Unchoke,
                        };
                        if let Err(_) = tx.send(Messages::Flag(msg)).await {
                            log::debug!("Unable to send {:?}", msg);
                        }
                    }
                }
            }
        }
    }

    ///
    /// Queue
    async fn queue_requests(state: Arc<Mutex<InnerTorrentState>>, request: PeerRequestedPiece) {
        let mut locked_state = state.lock().await;
        let peer_id = Arc::clone(&request.peer_id);
        //get_internal_peer_state
        let exists = locked_state
            .torrent_state
            .get_internal_peer_state_mut(&peer_id)
            .is_some();
        if exists {
            locked_state
                .torrent_state
                .outstanding_peer_requests
                .push_back(request);
        } else {
            log::debug!("Missing peer to queue request...");
        }
    }

    ///
    /// Respond to queued requests from our peers
    async fn dequeue_requests(
        state: Arc<Mutex<InnerTorrentState>>,
        peer_to_tx: &mut PeerToSender,
        io: IoHandler,
    ) -> Result<()> {
        let dequed = {
            let to_deq = 4; // TODO this should be configable
            let mut locked_state = state.lock().await;
            let mut deque = vec![];
            let mut dequed_cnt = 0;
            while let Some(dq) = locked_state
                .torrent_state
                .outstanding_peer_requests
                .pop_front()
            {
                if dequed_cnt >= to_deq {
                    break;
                }
                dequed_cnt += 1;
                deque.push(dq);
            }
            deque
        };

        for dq in dequed {
            if let Some(tx) = peer_to_tx.get(&dq.peer_id) {
                let block = io.read(dq.index, dq.begin, dq.length).await?;
                if let Err(_) = tx
                    .send(Messages::Piece {
                        index: dq.index,
                        begin: dq.begin,
                        block,
                    })
                    .await
                {
                    log::debug!("Missing peer...");
                }
            }
        }

        Ok(())
    }

    ///
    /// Prune closed tx channels
    fn prune_closed_tx(peer_to_tx: &mut PeerToSender) {
        let closed_keys = peer_to_tx
            .iter()
            .filter(|(_, channel)| channel.is_closed())
            .map(|(peer_id, _)| Arc::clone(&peer_id))
            .collect::<Vec<_>>();
        if !closed_keys.is_empty() {
            log::debug!("Pruning {} peers", closed_keys.len());
            for peer_id in closed_keys {
                let _ = peer_to_tx.remove(&peer_id);
            }
        }
    }

    ///
    /// Announce we have a new piece
    async fn announce_piece(peer_to_tx: &mut PeerToSender, piece_id: u32) {
        for (peer_id, tx) in peer_to_tx.iter() {
            if let Err(_) = tx
                .send(Messages::Have {
                    piece_index: piece_id,
                })
                .await
            {
                log::debug!(
                    "attempting to annouce piece {} to {} failed",
                    piece_id,
                    hex::encode(peer_id.as_ref()),
                );
            }
        }
    }

    ///
    /// Compute peers with pieces we want, and that are willing to share. Then begin to dispatch requests to them
    async fn compute_requests_for_download(
        state: Arc<Mutex<InnerTorrentState>>,
        peer_to_tx: &mut PeerToSender,
    ) {
        let mut locked_state = state.lock().await;
        //log::debug!("Computing peer requests {:?}", locked_state.torrent_state);

        // this function forgets that pieces need to be broken into blocks, so no real block tracking is done...

        let torrent = locked_state.torrent.clone();

        let peer_to_pieces_availaible = locked_state.torrent_state.piece_id_to_peers();

        let requests = locked_state
            .torrent_state
            .piece_block_tracking
            .generate_assignments(&torrent, &peer_to_pieces_availaible);

        // PieceBlockAllocation
        for request in requests.into_iter() {
            log::trace!("Dispatch request for piece_id {}", request.index);
            if let Some(tx) = peer_to_tx.get(&request.peer_id) {
                if let Ok(_) = tx
                    .send(Messages::Request {
                        index: request.index,
                        begin: request.begin,
                        length: request.length,
                    })
                    .await
                {
                    continue;
                }
            }
            log::warn!("Unable to dispatch {:?}", request);
            if !locked_state
                .torrent_state
                .piece_block_tracking
                .remove_request(request)
            {
                log::warn!("Request not found for removal")
            }
        }
    }

    ///
    /// Send out the initial bitfield message to inform the peer of our peices
    async fn init_connection(
        state: Arc<Mutex<InnerTorrentState>>,
        tx: &mut Sender<Messages>,
    ) -> Result<()> {
        let state = state.lock().await;
        let mut bitfield = BitFieldWriter::new(BytesMut::new());
        bitfield.put_bit_set(
            state
                .torrent_state
                .piece_block_tracking
                .get_pieces_completed(),
            state.torrent.info.pieces.len(),
        );
        let bitfield = Messages::BitField {
            bitfield: bitfield.into().to_vec(),
        };
        tx.send(bitfield).await?;
        // we always are sending choked and uninterested to start
        tx.send(Messages::Flag(FlagMessages::Choke)).await?;
        tx.send(Messages::Flag(FlagMessages::NotInterested)).await?;
        Ok(())
    }

    ///
    /// Maps the result so that we always return the peer_id
    async fn handle_peer_msgs(
        state: Arc<Mutex<InnerTorrentState>>,
        io: InternalTorrentWriter,
        rx: Receiver<Messages>,
        handshake: Handshake,
        tx: Sender<Messages>,
        wake_tx: Sender<InternalStateMessage>,
    ) -> (InternalPeerId, Result<()>) {
        let peer_id = Arc::new(handshake.peer_ctx.peer_id.clone());
        let res =
            Self::inner_handle_peer_msgs(Arc::clone(&peer_id), state, io, rx, tx, wake_tx).await;
        (peer_id, res)
    }

    ///
    /// Handle all incoming state updates from a single peer, and requests
    async fn inner_handle_peer_msgs(
        peer_id: InternalPeerId,
        state: Arc<Mutex<InnerTorrentState>>,
        io: InternalTorrentWriter,
        mut rx: Receiver<Messages>,
        tx: Sender<Messages>,
        wake_tx: Sender<InternalStateMessage>,
    ) -> Result<()> {
        log::info!("Starting torrent processing...");

        while let Some(msg) = rx.recv().await {
            let peer_id = Arc::clone(&peer_id);

            //log::debug!("Message: peer_id={}, msg={:?}", hex::encode(peer_id.as_ref()), msg);
            match msg {
                Messages::KeepAlive => {
                    // TODO reset timer or something, and expire after xx time
                }
                Messages::Flag(flag) => {
                    let mut state = state.lock().await;
                    match flag {
                        FlagMessages::Choke => state
                            .torrent_state
                            .set_peer_choked_us(Arc::clone(&peer_id), true),
                        FlagMessages::Unchoke => state
                            .torrent_state
                            .set_peer_choked_us(Arc::clone(&peer_id), false),
                        FlagMessages::Interested => state
                            .torrent_state
                            .set_peers_interested_in_us(Arc::clone(&peer_id), true),
                        FlagMessages::NotInterested => state
                            .torrent_state
                            .set_peers_interested_in_us(Arc::clone(&peer_id), false),
                    };
                    unlock_and_send!(wake_tx, state, InternalStateMessage::Wakeup);
                }
                Messages::Have { piece_index } => {
                    let mut state = state.lock().await;
                    let interest_change = state
                        .torrent_state
                        .add_pieces_for_peer(Arc::clone(&peer_id), vec![piece_index]);
                    if let Some(interest) = interest_change {
                        let msg = FlagMessages::interest_msg(interest);
                        log::debug!("Signal interest..");
                        if let Err(_) = tx.send(msg).await {
                            break;
                        }
                    }
                    unlock_and_send!(wake_tx, state, InternalStateMessage::Wakeup);
                }
                Messages::BitField { bitfield } => {
                    let mut state = state.lock().await;
                    let bitfield: BitFieldReaderIter = BitFieldReader::from(bitfield).into();
                    let pieces_present = bitfield
                        .into_iter()
                        .enumerate()
                        .filter(|(_, was_set)| *was_set)
                        .map(|(block, _)| block as u32)
                        .collect::<Vec<_>>();

                    let interest_change = state
                        .torrent_state
                        .add_pieces_for_peer(Arc::clone(&peer_id), pieces_present);

                    if let Some(interest) = interest_change {
                        let msg = FlagMessages::interest_msg(interest);
                        if let Err(e) = tx.send(msg).await {
                            log::info!("Closing... {:?}", e);
                            break;
                        }
                    }

                    unlock_and_send!(wake_tx, state, InternalStateMessage::Wakeup);
                }
                Messages::Request {
                    index,
                    begin,
                    length,
                } => {
                    let state = state.lock().await;
                    let choked = state
                        .torrent_state
                        .get_internal_peer_state(&peer_id)
                        .map(|peer_state| peer_state.choked)
                        .unwrap_or(true);
                    log::debug!("Have request for {}", index);
                    if choked {
                        log::debug!(
                            "Request is being ignored because it is choked [peer_id={} ",
                            hex::encode(peer_id.as_ref())
                        );
                        continue;
                    }

                    unlock_and_send!(wake_tx, state, {
                        InternalStateMessage::PeerRequestedPiece(PeerRequestedPiece {
                            peer_id,
                            index,
                            begin,
                            length,
                        })
                    });
                }
                Messages::Cancel {
                    index: _,
                    begin: _,
                    length: _,
                } => {} // TODO: ignoring for now
                Messages::Piece {
                    index,
                    begin,
                    block,
                } => {
                    let block_len = block.len();
                    let finished = io.write(index, begin, block).await?;
                    let mut state = state.lock().await;
                    if finished {
                        log::debug!("Piece done! piece={}", index);
                        state
                            .torrent_state
                            .piece_block_tracking
                            .set_piece_finished(index);

                        unlock_and_send!(wake_tx, state, {
                            InternalStateMessage::PieceComplete { piece_id: index }
                        });
                    } else {
                        log::debug!("Request done! piece={}, begin={}, len={}", index, begin, block_len);
                        if let None = state
                            .torrent_state
                            .piece_block_tracking
                            .set_request_finished(index, begin)
                        {
                            log::warn!(
                                "Request not found when setting finished: piece={}, begin={}",
                                index,
                                begin
                            );
                        }
                        unlock_and_send!(wake_tx, state, InternalStateMessage::Wakeup);
                    }
                }
            }
        }

        log::info!(
            "Peer state processing stopped. peer_id={}",
            hex::encode(peer_id.as_ref())
        );

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::{config::Config, peer::test::torrent_fixture};
    use std::time::Duration;

    use crate::{model::PeerContext, peer::io::MemoryTorrentIO};
    use tokio::{sync::mpsc::channel, task::JoinHandle};

    use super::*;

    // macro_rules! assert_type {
    //     ($ty:tt, $match_against: expr) => {
    //         match $match_against {
    //             $ty => true,
    //             _ => false,
    //         }
    //     };
    // }

    // #[test]
    // fn test_macros() {
    //     let a = Messages::Have { piece_index: 1 };
    //     assert_type!((Messages::Have{piece_index: 1}), a);
    // }

    fn peer_start_fixture(
        info_hash: &Vec<u8>,
    ) -> (PeerStartMessage, Sender<Messages>, Receiver<Messages>) {
        let (in_tx, in_rx) = channel(1024);
        let (out_tx, out_rx) = channel(1024);
        let handshake = Handshake {
            peer_ctx: PeerContext {
                info_hash: info_hash.clone(),
                peer_id: vec![10 as u8; 20],
            },
        };
        let peer_start_msg = PeerStartMessage {
            handshake,
            rx: in_rx,
            tx: out_tx,
        };
        (peer_start_msg, in_tx, out_rx)
    }

    async fn setup_test() -> (JoinHandle<Result<()>>, Sender<Messages>, Receiver<Messages>) {
        let mut config = Config::default();
        config.max_outstanding_requests = 1024;
        let torrent = torrent_fixture(vec![1 as u8, 20]);
        let io = Box::new(MemoryTorrentIO::new(torrent.clone()).await);
        let io = IoHandler::new(config.clone(), io).await.unwrap();
        let processor = TorrentProcessor::new(config, Arc::new(vec![1, 2, 3]), torrent.clone(), io);

        // channel for new connections
        let (conn_tx, conn_rx) = channel(1);
        // per peer channels
        let (peer_start_msg, tx, rx) = peer_start_fixture(&torrent.info.info_hash);
        conn_tx.send(peer_start_msg).await.unwrap();
        let processor_task = tokio::spawn(processor.start(conn_rx));

        (processor_task, tx, rx)
    }

    async fn wait_rx<M>(rx: &mut Receiver<M>, timeout: Duration, expect: usize) -> Vec<M> {
        let mut buf = Vec::with_capacity(expect);
        //let waiting = rx.recv_many(&mut buf, expect);
        let waiting = async {
            while let Some(m) = rx.recv().await {
                buf.push(m);
                if buf.len() >= expect {
                    break;
                }
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(timeout) => {
                if expect != 0 {
                    panic!("timeout occured, waiting for msgs: expected_count={}, was={}", expect, buf.len());
                } else {
                    return vec![];
                }
            },
            _ = waiting => (),
        }

        return buf;
    }

    async fn await_expected_initial_messages(msg_rx: &mut Receiver<Messages>) {
        let msgs = wait_rx(msg_rx, Duration::from_secs(1), 3).await;
        // initial message
        assert_eq!(msgs.len(), 3);
        assert_eq!(
            msgs,
            vec![
                Messages::BitField { bitfield: vec![0] },
                Messages::Flag(FlagMessages::Choke),
                Messages::Flag(FlagMessages::NotInterested)
            ]
        );
    }

    async fn send_bitfield(msg_tx: &Sender<Messages>, set: &HashSet<u32>, set_len: usize) {
        let mut all_pieces_bf = BitFieldWriter::new(BytesMut::new());
        all_pieces_bf.put_bit_set(set, set_len);
        let all_pieces_bf = all_pieces_bf.into().to_vec();
        msg_tx
            .send(Messages::BitField {
                bitfield: all_pieces_bf,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn connection_handles_choke_events() {
        let _ = env_logger::try_init();
        let (_, msg_tx, mut msg_rx) = setup_test().await;
        await_expected_initial_messages(&mut msg_rx).await;

        // peer messages
        send_bitfield(&msg_tx, &vec![0, 1].into_iter().collect(), 2).await;
        msg_tx
            .send(Messages::Flag(FlagMessages::Choke))
            .await
            .unwrap();
        msg_tx
            .send(Messages::Flag(FlagMessages::NotInterested))
            .await
            .unwrap();

        //expect interest since pieces are not our own
        let msgs = wait_rx(&mut msg_rx, Duration::from_secs(1), 1).await;
        assert_eq!(msgs, vec![Messages::Flag(FlagMessages::Interested)]);

        // wait a bit, and expect no requests, since the peer is choking us
        tokio::time::sleep(Duration::from_secs(1)).await;
        let msgs = wait_rx(&mut msg_rx, Duration::from_secs(1), 0).await;
        assert!(msgs.is_empty());

        // send a unchoked messages
        msg_tx
            .send(Messages::Flag(FlagMessages::Unchoke))
            .await
            .unwrap();
        let expected = (10_240_000 / 16384) + 1; // expected
        let msgs = wait_rx(&mut msg_rx, Duration::from_secs(1), expected).await;
        assert_eq!(msgs.len(), expected);
        let pieces = msgs
            .into_iter()
            .filter_map(|msg| match msg {
                Messages::Request {
                    index,
                    begin,
                    length,
                } => Some(Messages::Piece {
                    index,
                    begin,
                    block: vec![0; length as _],
                }),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(pieces.len(), expected);

        for piece in pieces {
            msg_tx.send(piece).await.unwrap();
        }
        // Ensure haves are sent. Since pieces may originate from different pees, it makes sense to duplicate.
        let mut msgs = wait_rx(&mut msg_rx, Duration::from_secs(1), 2).await;
        // sort the messages by piece_id
        msgs.sort_by(|a, b| {
            let a = match a {
                Messages::Have { piece_index } => piece_index,
                e => panic!("Bad message: {:?}", e),
            };
            let b = match b {
                Messages::Have { piece_index } => piece_index,
                _ => panic!("Bad message"),
            };

            a.cmp(b)
        });
        assert_eq!(
            msgs,
            vec![
                Messages::Have { piece_index: 0 },
                Messages::Have { piece_index: 1 }
            ]
        );
    }
}
