mod request;
use crate::model::{V1Piece, V1Torrent};
use crate::peer::state::request::{PeerRequestedPiece, PieceBlockTracking};
use crate::peer::{InternalPeerId, PeerId};
use crate::peer::bitfield::{BitFieldReader, BitFieldReaderIter, BitFieldWriter};
use crate::peer::writer::TorrentWriter;
use crate::peer::protocol::{FlagMessages, Handshake};
use anyhow::Result;
use bytes::BytesMut;
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tokio::sync::mpsc::{Receiver, Sender};

use super::protocol::Messages;

type PeerToSender = HashMap<InternalPeerId, Sender<Messages>>;

const MAX_OUTSTANDING_REQUESTS: usize = 4;

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

#[derive(Default, Debug)]
struct PeerPieceMap {
    pieces_to_peers: HashMap<u32, HashSet<InternalPeerId>>,
    peers_to_pieces: HashMap<InternalPeerId, HashSet<u32>>,
}

impl PeerPieceMap {
    pub fn peer_interest(&self, peer_id: &InternalPeerId, outstanding_pieces: &HashSet<u32>) -> HashSet<u32> {
        if let Some(peer_pieces) = self.peers_to_pieces.get(peer_id) {
            outstanding_pieces.intersection(peer_pieces).map(|piece| *piece).collect::<HashSet<_>>()
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

    // pub fn remove_piece(&mut self, peer_id: InternalPeerId, piece: u32) {
    //     let ptb = self
    //         .peers_to_pieces
    //         .entry(Arc::clone(&peer_id))
    //         .or_insert_with(|| HashSet::new());
    //     let _ = ptb.remove(&piece);

    //     let btp = self
    //         .pieces_to_peers
    //         .entry(piece)
    //         .or_insert_with(|| HashSet::new());
    //     let _ = btp.remove(&peer_id);
    // }

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
    choked: bool,
    interested: bool,
}

#[derive(Debug, Default)]
struct TorrentState {
    peers: HashSet<InternalPeerId>,
    peers_interested: HashSet<InternalPeerId>, // external -> us
    peers_not_choking: HashSet<InternalPeerId>, //external -> us
    block_peer_map: PeerPieceMap,
    /// piece tracking
    pieces_not_started: HashSet<u32>,
    pieces_started: HashSet<u32>,
    pieces_finished: HashSet<u32>,
    //
    internal_peer_state: HashMap<InternalPeerId, InternalPeerState>
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
            let _ = self.peers.insert(peer_id);
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
        self.add_peer_id(Arc::clone(&peer_id));
        if interested {
            let _ = self.peers_interested.insert(peer_id);
        } else {
            let _ = self.peers_interested.remove(&peer_id);
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

    // pub fn peers_that_are_interested(&self, interested: bool) -> HashSet<InternalPeerId> {
    //     if !interested {
    //         return self
    //             .peers
    //             .difference(&self.peers_interested)
    //             .map(|a| Arc::clone(a))
    //             .collect();
    //     } else {
    //         return self
    //             .peers
    //             .intersection(&self.peers_interested)
    //             .map(|a| Arc::clone(a))
    //             .collect();
    //     }
    // }

    ///
    /// If peer send bitfield or has message then call this, returns "our interest".
    /// if some, then interest has changed
    /// if none, no change happened
    pub fn add_pieces_for_peer(&mut self, peer_id: InternalPeerId, pieces: Vec<u32>) -> Option<bool> {
        self.add_peer_id(Arc::clone(&peer_id));
        let prev_interest = !self.block_peer_map.peer_interest(&peer_id, &self.pieces_not_started).is_empty();
        for piece in pieces {
            self.block_peer_map.add_piece(Arc::clone(&peer_id), piece);
        }
        let interest = !self.block_peer_map.peer_interest(&peer_id, &self.pieces_not_started).is_empty();
        if prev_interest != interest {
            (*self
                .internal_peer_state
                .entry(Arc::clone(&peer_id))
                .or_insert_with(|| InternalPeerState::default())).interested = interest;
            Some(interest)
        } else {
            None
        }
    }

    ///
    /// Our view on the peers conencted to use
    pub fn get_internal_peer_state(&self, peer_id: &InternalPeerId) -> Option<&InternalPeerState> {
        self.internal_peer_state.get(peer_id)
    }

    pub fn set_piece_started(&mut self, piece_id: u32) {
        let _ = self.pieces_not_started.remove(&piece_id);
        let _ = self.pieces_started.insert(piece_id);
    }

    pub fn set_piece_finished(&mut self, piece_id: u32) {
        let _ = self.pieces_started.remove(&piece_id);
        let _ = self.pieces_finished.insert(piece_id);
    }

    // pub fn download_done(&self) -> bool {
    //     self.pieces_finished.len() == 
    // }

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
        let _ = self.peers.remove(&peer_id);
        let _ = self.peers_interested.remove(&peer_id);
        let _ = self.peers_not_choking.remove(&peer_id);
        let _ = self.block_peer_map.remove_peer(&peer_id);
        let _ = self.internal_peer_state.remove(&peer_id);

        return Arc::into_inner(peer_id);
    }
}

pub struct TorrentProcessor {
    our_id: InternalPeerId,
    torrent: V1Torrent,
    torrent_state: TorrentState,
    torrent_writer: Box<dyn TorrentWriter>,
}

impl TorrentProcessor {
    pub fn new(our_id: InternalPeerId, torrent: V1Torrent, torrent_writer: Box<dyn TorrentWriter>) -> Self {
        let torrent_state = TorrentState::new(&torrent.info.pieces);
        Self {
            our_id,
            torrent,
            torrent_state,
            torrent_writer,
        }
        
    }
    pub async fn start(self, mut rx: Receiver<PeerStartMessage>) {
        log::info!("torrent state processor starting... {}", hex::encode(self.our_id.as_ref()));
        let state = Arc::new(Mutex::new(self));
        let mut peer_to_tx: PeerToSender = HashMap::new();
        let mut handle_peer_requests_fq = FuturesUnordered::new();
        let (wake_up_tx, mut wake_up_rx) = mpsc::channel(1);


        loop {
            tokio::select! {
                Some(new) = rx.recv() => {
                    let (handshake, rx, mut tx) = new.into();
                    log::info!("Accepting peer {}", hex::encode(&handshake.peer_ctx.peer_id));
                    // send peer back state needed, pieces we have and the choke, interested
                    if let Ok(_) = Self::init_connection(Arc::clone(&state), &mut tx).await {
                        let _ = peer_to_tx.insert(Arc::new(handshake.peer_ctx.peer_id.clone()), tx.clone());
                        handle_peer_requests_fq.push(Self::handle_peer_msgs(Arc::clone(&state), tx, rx, handshake, wake_up_tx.clone()));
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
                Some(_) = wake_up_rx.recv() => {},
                // _ = tokio::time::sleep(Duration::from_secs(1)) => {
                //     log::trace!("Wake up...");
                //     // we wake up here so that we occasionally complete the loop, and compute the requests
                // }
                else => break,
            }

            Self::compute_requests(Arc::clone(&state), &mut peer_to_tx).await;
        }
        log::info!("Closing main state loop...")
    }

    ///
    /// Compute peers with pieces we want, and that are willing to share. Then begin to dispatch requests to them
    async fn compute_requests(state: Arc<Mutex<Self>>, peer_to_tx: &mut PeerToSender) {
        let mut locked_state = state.lock().await;
        log::trace!("Computing peer requests {:?}", locked_state.torrent_state);
        let torrent = locked_state.torrent.clone();
        let block_to_request_tracker = locked_state
            .torrent_state
            .peer_willing_to_upload_pieces()
            .into_iter()
            .map(|(piece_id, peers)| {
                (piece_id, PieceBlockTracking::new(piece_id, &torrent, peers))
            }).filter_map(|(p, tracker_opt)| {
                tracker_opt.map(|t| (p, t))
            }).take(MAX_OUTSTANDING_REQUESTS)
            .collect::<HashMap<_, _>>();

        log::debug!("Computed requests to peer: pieces_not_started={} requests_made={}", 
            locked_state.torrent_state.pieces_not_started.len(),
            block_to_request_tracker.len()
        );

        for (piece_id, tracker) in block_to_request_tracker.into_iter() {
            log::debug!("Dispatch request for piece_id {}", piece_id);
            locked_state
                .torrent_state
                .set_piece_started(piece_id);
            for req in tracker.requests_to_make {
                log::debug!("Attempting to send request {:?}", req);
                if let Some(tx) = peer_to_tx.get(&req.peer_id) {
                    let req_msg = Messages::Request { index: req.index, begin: req.begin, length: req.length };
                    if let Err(_) = tx.send(req_msg).await {
                        log::debug!("Got closed tx: {}", hex::encode(req.peer_id.as_ref()));
                    }
                }
            }
        }
    }

    ///
    /// Send out the initial bitfield message to inform the peer of our peices
    async fn init_connection(state: Arc<Mutex<Self>>, tx: &mut Sender<Messages>) -> Result<()> {
        let state = state.lock().await;
        let mut bitfield = BitFieldWriter::new(BytesMut::new());
        bitfield.put_bit_set(&state.torrent_state.pieces_finished, state.torrent.info.pieces.len());
        let bitfield = Messages::BitField { bitfield: bitfield.into().to_vec() };
        tx.send(bitfield).await?;
        // we always are sending choked and uninterested to start
        tx.send(Messages::Flag(FlagMessages::Choke)).await?;
        tx.send(Messages::Flag(FlagMessages::NotInterested)).await?;
        Ok(())
    }

    ///
    /// Maps the result so that we always return the peer_id
    async fn handle_peer_msgs(
        state: Arc<Mutex<Self>>,
        tx: Sender<Messages>,
        rx: Receiver<Messages>,
        handshake: Handshake,
        wake_tx: Sender<InternalPeerId>,
    ) -> (InternalPeerId, Result<()>) {
        let peer_id = Arc::new(handshake.peer_ctx.peer_id.clone());
        let res = Self::inner_handle_peer_msgs(Arc::clone(&peer_id), state, tx, rx, wake_tx).await;
        (peer_id, res)
    }

    ///
    /// Handle all incoming state updates from a single peer, and requests
    async fn inner_handle_peer_msgs(
        peer_id: InternalPeerId,
        state: Arc<Mutex<Self>>,
        tx: Sender<Messages>,
        mut rx: Receiver<Messages>,
        wake_tx: Sender<InternalPeerId>,
    ) -> Result<()> {
        log::info!("Starting torrent processing...");
        let mut outstanding_requests = vec![];

        while let Some(msg) = rx.recv().await {
            let peer_id = Arc::clone(&peer_id);
            let mut state = state.lock().await;

            log::debug!("Message: peer_id={}, msg={:?}", hex::encode(peer_id.as_ref()), msg);
            match msg {
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
                    wake_tx.send(peer_id).await?;
                }
                Messages::BitField { bitfield } => {
                    let bitfield: BitFieldReaderIter = BitFieldReader::from(bitfield).into();
                    let pieces_present = bitfield
                        .into_iter()
                        .enumerate()
                        .filter(|(_, was_set)| *was_set)
                        .map(|(block, _)| block as u32)
                        .collect::<Vec<_>>();
                    log::info!("Bitfield: peer_id={}, has={:?}", hex::encode(peer_id.as_ref()), pieces_present);
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
                    wake_tx.send(peer_id).await?;
                }
                Messages::Request {
                    index,
                    begin,
                    length,
                } => {
                    let choked = state
                        .torrent_state
                        .get_internal_peer_state(&peer_id).map(|peer_state| peer_state.choked)
                        .unwrap_or(true);
                    if choked {
                        log::info!("Request is being ignored because it is choked [peer_id={} ", hex::encode(peer_id.as_ref()));
                        continue;
                    }

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
                        let _ = outstanding_requests.remove(idx);
                    }
                }
                Messages::Piece {
                    index,
                    begin,
                    block,
                } => {
                    let finished = state
                        .torrent_writer
                        .write_piece(index, begin, block)
                        .await?;

                    if finished {
                        log::info!("Piece done! piece={}", index);
                        state.torrent_state.set_piece_finished(index);
                    }
                }
            }
        }

        log::info!("Peer state processing stopped. peer_id={}", hex::encode(peer_id.as_ref()));

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;
    use crate::peer::test::torrent_fixture;

    use crate::{model::{PeerContext}, peer::writer::MemoryTorrentWriter};
    use tokio::{sync::mpsc::channel, task::JoinHandle};

    use super::*;

    fn peer_start_fixture(info_hash: &Vec<u8>) -> (PeerStartMessage, Sender<Messages>, Receiver<Messages>) {
        let (in_tx, in_rx) = channel(1);
        let (out_tx, out_rx) = channel(1);
        let handshake = Handshake{
            peer_ctx: PeerContext {
                info_hash: info_hash.clone(),
                peer_id: vec![10 as u8; 20]
            }
        };
        let peer_start_msg = PeerStartMessage{
            handshake,
            rx: in_rx,
            tx: out_tx,
        };
        (peer_start_msg, in_tx, out_rx)
    }

    async fn setup_test() -> (JoinHandle<()>, Sender<Messages>, Receiver<Messages>) {
        let torrent = torrent_fixture(vec![1 as u8, 20]);
        let torrent_writer = MemoryTorrentWriter::new(torrent.clone()).await;
        let processor = TorrentProcessor::new(Arc::new(vec![1,2,3]), torrent.clone(), Box::new(torrent_writer));

        // channel for new connections
        let (conn_tx, conn_rx) = channel(1);
        // per peer channels
        let (peer_start_msg, tx, rx) = peer_start_fixture(&torrent.info.info_hash);
        conn_tx.send(peer_start_msg)
            .await
            .unwrap();
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

    #[tokio::test]
    async fn on_connection_bitfield_and_choke_are_sent() {
        env_logger::init();
        let (_, msg_tx, mut msg_rx) = setup_test().await;
        let msgs = wait_rx(&mut msg_rx, Duration::from_secs(2), 3).await;
        // initial message
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs, vec![
            Messages::BitField { bitfield: vec![0] }, 
            Messages::Flag(FlagMessages::Choke), 
            Messages::Flag(FlagMessages::NotInterested)
        ]);

        // peer messages
        let mut all_pieces_bf = BitFieldWriter::new(BytesMut::new());
        all_pieces_bf.put_bit(true);
        all_pieces_bf.put_bit(true);
        let all_pieces_bf = all_pieces_bf.into().to_vec();
        msg_tx.send(Messages::BitField { bitfield: all_pieces_bf }).await.unwrap();
        msg_tx.send(Messages::Flag(FlagMessages::Choke)).await.unwrap();
        msg_tx.send(Messages::Flag(FlagMessages::NotInterested)).await.unwrap();

        //expect interest since pieces are not our own
        let msgs = wait_rx(&mut msg_rx, Duration::from_secs(2), 1).await;
        assert_eq!(msgs, vec![Messages::Flag(FlagMessages::Interested)]);

        // wait a bit, and expect no requests, since the peer is choking us
        tokio::time::sleep(Duration::from_secs(5)).await;
        let msgs = wait_rx(&mut msg_rx, Duration::from_secs(3), 0).await;
        assert!(msgs.is_empty());

        // send a unchoked messages
        msg_tx.send(Messages::Flag(FlagMessages::Unchoke)).await.unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
        let expected = (10_240_000 / 16384) + 1;
        let msgs = wait_rx(&mut msg_rx, Duration::from_secs(1), expected).await;
        assert_eq!(msgs.len(), expected);
    }
}