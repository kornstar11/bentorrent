use crate::model::V1Torrent;
use crate::peer::bitfield::{BitFieldReader, BitFieldReaderIter};
use crate::peer::protocol::{FlagMessages, Handshake};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::oneshot::{Receiver as OReceiver, Sender as OSender};

use super::protocol::Messages;

type PeerId = Vec<u8>;
type InternalPeerId = Arc<PeerId>;

const BLOCK_SIZE: usize = 2 ^ 14;

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

    pub fn get_peers_with_piece(&self, piece: u32) -> Vec<InternalPeerId> {
        if let Some(peers) = self.pieces_to_peers.get(&piece) {
            peers
                .iter()
                .map(|peer| Arc::clone(peer))
                .collect::<Vec<_>>()
        } else {
            vec![]
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

#[derive(Default, Debug)]
struct TorrentState {
    peers: HashSet<InternalPeerId>,
    peers_interested: HashSet<InternalPeerId>,
    peers_not_choking: HashSet<InternalPeerId>,
    block_peer_map: PeerPieceMap,
}

impl TorrentState {
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

    pub fn peers_that_choke(&self, choke: bool) -> Vec<InternalPeerId> {
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

    pub fn peers_that_are_interested(&self, interested: bool) -> Vec<InternalPeerId> {
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

struct PeerStartMessage {
    handshake: Handshake,
    rx: Receiver<ProtocolMessage>,
    tx: Sender<Messages>,
}

///
/// Processor logic

struct RequestedBlock {
    peer_id: InternalPeerId,
    index: u32,
    begin: u32,
    length: u32,
}

struct TorrentProcessor {
    torrent: V1Torrent,
    torrent_state: TorrentState,
    outstanding_requests: Vec<RequestedBlock>,
}

impl TorrentProcessor {
    async fn handle_peer_msgs(&mut self, mut peer_msg: PeerStartMessage) -> Result<()> {
        log::info!("Starting torrent processing...");
        let peer_id = Arc::new(peer_msg.handshake.peer_ctx.peer_id);
        while let Some(msg) = peer_msg.rx.recv().await {
            let peer_id = Arc::clone(&peer_id);
            match msg.msg {
                Messages::KeepAlive => {
                    // TODO reset timer or something, and expire after xx time
                }
                Messages::Flag(flag) => match flag {
                    FlagMessages::Choke => self.torrent_state.set_peer_choked_us(peer_id, true),
                    FlagMessages::Unchoke => self.torrent_state.set_peer_choked_us(peer_id, false),
                    FlagMessages::Interested => {
                        self.torrent_state.set_peers_interested_in_us(peer_id, true)
                    }
                    FlagMessages::NotInterested => self
                        .torrent_state
                        .set_peers_interested_in_us(peer_id, false),
                },
                Messages::Have { piece_index } => {
                    self.torrent_state
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
                    self.torrent_state
                        .add_pieces_for_peer(peer_id, pieces_present);
                }
                Messages::Request {
                    index,
                    begin,
                    length,
                } => {
                    self.outstanding_requests.push(RequestedBlock {
                        peer_id,
                        index,
                        begin,
                        length,
                    });
                }
                Messages::Request {
                    index,
                    begin,
                    length,
                } => {
                    // attempt to cancel
                    let idx_to_remove_opt = self
                        .outstanding_requests
                        .iter()
                        .enumerate()
                        .find(|(idx, o)| {
                            o.peer_id == peer_id
                                && o.index == index
                                && o.begin == begin
                                && o.length == length
                        })
                        .map(|(idx, _)| idx);

                    if let Some(idx) = idx_to_remove_opt {
                        self.outstanding_requests.remove(idx);
                    }
                }

                _ => {
                    todo!()
                }
            }
        }

        Ok(())
    }
}
