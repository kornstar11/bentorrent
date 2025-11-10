use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::oneshot::{Receiver as OReceiver, Sender as OSender};
use crate::peer::protocol::{FlagMessages, Handshake};

use super::protocol::Messages;

type PeerId = Vec<u8>;
type InternalPeerId = Arc<PeerId>;

struct TorrentState {
    peers: HashSet<InternalPeerId>,
    peers_interested: HashSet<InternalPeerId>,
    peers_not_choking: HashSet<InternalPeerId>,
    //blocks_to_peers: HashMap<u32, HashSet<InternalPeerId>>,
    info_hash: Vec<u8>,
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

    pub fn remove_peer(&mut self, peer_id: PeerId) -> Option<PeerId> {
        let peer_id = self.get_peer_id(peer_id);
        self.peers.remove(&peer_id);
        self.peers_interested.remove(&peer_id);
        self.peers_not_choking.remove(&peer_id);

        return Arc::into_inner(peer_id);
    }
    
}

/*
///
/// Messages
pub struct ProtocolMessage {
    msg: Messages,
    peer_id: Vec<u8>,
    ack: OSender<Option<Messages>>,
}

enum PeerStateMessage {
    Start(Handshake),
    Protocol(ProtocolMessage)
}

struct TorrentState {
    peers: HashMap<Vec<u8>, PeerState>,
    info_hash: Vec<u8>,
}

impl TorrentState {
    pub async fn start(mut self, mut rx: Receiver<ProtocolMessage>) -> Result<()> {
        log::info!("Starting torrent processing...");
        while let Some(msg) = rx.recv().await {
            let peer = self.peers.entry(msg.peer_id).or_insert_with(|| {
                PeerState::default()
            });

            match msg.msg {
                Messages::KeepAlive => {
                    // ignore
                },
                Messages::Flag(flag) => {
                    peer.process_flag_update(flag);
                },
                _ => {todo!()}
                
            }
        }

        Ok(())
    }
}
*/