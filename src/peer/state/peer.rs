use std::sync::Arc;
use anyhow::Result;
use tokio::sync::{Mutex, mpsc::{Receiver, Sender}};

use crate::{model::{InternalPeerId, PeerRequestedPiece}, peer::{bitfield::{BitFieldReader, BitFieldReaderIter}, protocol::{FlagMessages, Messages}, state::{InnerTorrentState, InternalStateMessage, InternalTorrentWriter}}};
use crate::unlock_and_send;

///
/// Handle all incoming state updates from a single peer, and requests
pub async fn inner_handle_peer_msgs(
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
                let interest_change = {
                    state
                        .torrent_state
                        .add_pieces_for_peer(Arc::clone(&peer_id), vec![piece_index])
                };
                if let Some(interest) = interest_change {
                    let msg = FlagMessages::interest_msg(interest);
                    log::debug!("Signal interest..");
                    if let Err(_) = tx.send(msg).await {
                        break;
                    }
                }
                wake_tx.send(InternalStateMessage::Wakeup).await?;
            }
            Messages::BitField { bitfield } => {
                let interest_change = {
                    let mut state = state.lock().await;
                    let bitfield: BitFieldReaderIter = BitFieldReader::from(bitfield).into();
                    let pieces_present = bitfield
                        .into_iter()
                        .enumerate()
                        .filter(|(_, was_set)| *was_set)
                        .map(|(block, _)| block as u32)
                        .collect::<Vec<_>>();

                    state
                        .torrent_state
                        .add_pieces_for_peer(Arc::clone(&peer_id), pieces_present)
                };

                if let Some(interest) = interest_change {
                    let msg = FlagMessages::interest_msg(interest);
                    if let Err(e) = tx.send(msg).await {
                        log::info!("Closing... {:?}", e);
                        break;
                    }
                }
                wake_tx.send(InternalStateMessage::Wakeup).await?;
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