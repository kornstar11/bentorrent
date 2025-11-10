use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::oneshot::{Receiver as OReceiver, Sender as OSender};
use crate::peer::bitfield::{BitFieldReader, BitFieldReaderIter};
use crate::peer::protocol::{FlagMessages, Handshake};

use super::protocol::Messages;

type PeerId = Vec<u8>;
type InternalPeerId = Arc<PeerId>;

#[derive(Default, Debug)]
struct PeerBlockMap {
    blocks_to_peers: HashMap<u32, HashSet<InternalPeerId>>,
    peers_to_blocks: HashMap<InternalPeerId, HashSet<u32>>,
}

impl PeerBlockMap {
    pub fn add_block(&mut self, peer_id: InternalPeerId, block: u32) {
        let ptb = self.peers_to_blocks
            .entry(Arc::clone(&peer_id))
            .or_insert_with(|| HashSet::new());
        ptb.insert(block);

        let btp = self.blocks_to_peers
            .entry(block)
            .or_insert_with(|| HashSet::new());
        btp.insert(peer_id);
    }

    pub fn get_peers_with_block(&self, block: u32) -> Vec<InternalPeerId> {
        if let Some(peers) = self.blocks_to_peers.get(&block) {
            peers.iter().map(|peer| Arc::clone(peer)).collect::<Vec<_>>()
        } else {
            vec![]
        }
    }

    pub fn remove_block(&mut self, peer_id: InternalPeerId, block: u32) {
        let ptb = self.peers_to_blocks
            .entry(Arc::clone(&peer_id))
            .or_insert_with(|| HashSet::new());
        ptb.remove(&block);

        let btp = self.blocks_to_peers
            .entry(block)
            .or_insert_with(|| HashSet::new());
        btp.remove(&peer_id);
    }

    pub fn remove_peer(&mut self, peer_id: &InternalPeerId) {
        if let Some(blocks) = self.peers_to_blocks.remove(peer_id) {
            for block in blocks.into_iter() {
                if let Some(peers) = self.blocks_to_peers.get_mut(&block) {
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
    block_peer_map: PeerBlockMap,
}

impl TorrentState {
    fn get_peer_id(&mut self, peer_id: PeerId) -> InternalPeerId {
        let peer_id = if let Some(peer_id) = self.peers.get(&peer_id) { // allows us to reliably reference one peerid
            Arc::clone(peer_id)
        } else {
            let peer_id = Arc::new(peer_id);
            self.peers.insert(Arc::clone(&peer_id));
            peer_id
        };

        return peer_id;
    }

    pub fn set_peer_choked_us(&mut self, peer_id: PeerId, choked: bool) {
        let peer_id = self.get_peer_id(peer_id);
        if !choked {
            self.peers_not_choking.insert(peer_id);
        } else {
            self.peers_not_choking.remove(&peer_id);
        }
    }

    pub fn set_peers_interested_in_us(&mut self, peer_id: PeerId, interested: bool) {
        let peer_id = self.get_peer_id(peer_id);
        if interested {
            self.peers_interested.insert(peer_id);
        } else {
            self.peers_interested.remove(&peer_id);
        }
    }

    pub fn peers_that_choke(&self, choke: bool) -> Vec<InternalPeerId> {
        if choke {
            return self.peers.difference(&self.peers_not_choking)
                .map(|a| Arc::clone(a))
                .collect();
        } else {
            return self.peers.intersection(&self.peers_not_choking)
                .map(|a| Arc::clone(a))
                .collect();
        }
    }

    pub fn peers_that_are_interested(&self, interested: bool) -> Vec<InternalPeerId> {
        if !interested {
            return self.peers.difference(&self.peers_interested)
                .map(|a| Arc::clone(a))
                .collect();
        } else {
            return self.peers.intersection(&self.peers_interested)
                .map(|a| Arc::clone(a))
                .collect();
        }
    }

    pub fn add_blocks_for_peer(&mut self, peer_id: PeerId, blocks: Vec<u32>) {
        let peer_id = self.get_peer_id(peer_id);
        for block in blocks {
            self.block_peer_map.add_block(Arc::clone(&peer_id), block);
        }
    }

    pub fn remove_peer(&mut self, peer_id: PeerId) -> Option<PeerId> {
        let peer_id = self.get_peer_id(peer_id);
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
    peer_id: PeerId,
    ack: OSender<Option<Messages>>,
}

enum PeerStateMessage {
    Start(Handshake),
    Protocol(ProtocolMessage)
}

struct TorrentProcessor {
    torrent_state: TorrentState,
    info_hash: Vec<u8>,
}

impl TorrentProcessor {
    pub async fn start(mut self, mut rx: Receiver<ProtocolMessage>) -> Result<()> {
        log::info!("Starting torrent processing...");
        while let Some(msg) = rx.recv().await {
            let peer_id = msg.peer_id;
            match msg.msg {
                Messages::KeepAlive => {
                    // ignore
                },
                Messages::Flag(flag) => {
                    match flag {
                        FlagMessages::Choke => self.torrent_state.set_peer_choked_us(peer_id, true),
                        FlagMessages::Unchoke => self.torrent_state.set_peer_choked_us(peer_id, false),
                        FlagMessages::Interested => self.torrent_state.set_peers_interested_in_us(peer_id, true),
                        FlagMessages::NotInterested => self.torrent_state.set_peers_interested_in_us(peer_id, false),
                    }
                },
                Messages::Have { piece_index } => {
                    self.torrent_state.add_blocks_for_peer(peer_id, vec![piece_index]);
                }
                Messages::BitField { bitfield } => {
                    let bitfield: BitFieldReaderIter = BitFieldReader::from(bitfield).into();
                    let blocks_present =  bitfield
                        .into_iter()
                        .enumerate()
                        .filter(|(_, was_set)| {
                            *was_set
                        }).map(|(block, _)| block as u32)
                        .collect::<Vec<_>>();
                    self.torrent_state.add_blocks_for_peer(peer_id, blocks_present);
                }
                _ => {todo!()}
                
            }
        }

        Ok(())
    }
}