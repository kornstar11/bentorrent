use std::collections::{HashMap, HashSet};
use anyhow::Result;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::oneshot::{Receiver as OReceiver, Sender as OSender};
use crate::peer::protocol::{FlagMessages, Handshake};

use super::protocol::Messages;

#[derive(Debug, Default)]
pub struct PeerState {
    am_choking: bool,
    am_interested: bool,
    peer_choking: bool,
    peer_interested: bool,
    peer_requests: HashSet<usize>,
}

impl PeerState {
    pub fn can_upload_to(&self) -> bool {
        self.peer_interested && !self.am_choking
    }

    pub fn can_download_from(&self) -> bool {
        self.am_interested && !self.peer_choking
    }
}
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
    pub async fn start(self, mut rx: Receiver<ProtocolMessage>) -> Result<()> {
        log::info!("Starting torrent processing...");
        while let Some(msg) = rx.recv().await {
            let peer = self.peers.entry(msg.peer_id).or_insert_with(|| {
                PeerState::default()
            });
            match msg.msg {
                Messages::KeepAlive => {},
                Messages::Flag(flag) => {
                    self.process_flag_update(peer, flag);
                }
                
            }
        }

        Ok(())
    }

    fn process_flag_update(&mut self, peer: &mut PeerState, flag: FlagMessages) {
        match flag {
            FlagMessages::Choke => {
                peer.peer_choking = true;
            },
            FlagMessages::Unchoke => {
                peer.peer_choking = false;
            },
            FlagMessages::Interested => {
                peer.peer_interested = true;
            },
            FlagMessages::NotInterested => {
                peer.peer_interested = false;
            }
        }


    }
    
}