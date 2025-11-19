use std::{marker::PhantomData, sync::Arc};

use bytes::{Buf, BytesMut, TryGetError};
use futures::{StreamExt, stream::FuturesUnordered};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream, sync::mpsc::{self, Sender}};
use anyhow::Result;
use crate::{model::{PeerContext, TrackerResponse, V1Torrent}, peer::{InternalPeerId, PIECE_BLOCK_SIZE, error::PeerError, protocol::{Decode, Encode, Handshake, HandshakeDecoder, MessagesDecoder, ProtocolError}, state::PeerStartMessage}};

pub async fn connect_torrent_peers(torrent: V1Torrent, our_id: InternalPeerId, tracker_resp: TrackerResponse, tx: Sender<PeerStartMessage>) -> Result<()> {
    let mut active_conns = FuturesUnordered::new();
    log::debug!("Begin initial connection to peers: {:?}", tracker_resp.peers);

    for peer in tracker_resp.peers.into_iter() {
        let stream = TcpStream::connect(peer.socket_addr()).await;
        match stream {
            Ok(stream) => {
                log::info!("Connected to {:?}", peer.address);
                active_conns.push(run_connection(torrent.clone(), Arc::clone(&our_id), stream, tx.clone()));
            },
            Err(e) => {
                log::warn!("Unable to connect to peer: ip={:?}, err={:?}", peer.socket_addr(), e);
            }
        }
    }
    log::info!("Peer connections spawned: {}", active_conns.len());
    while let Some(r) = active_conns.next().await {
        log::info!("Connection finished: {:?}", r);
    }

    Ok(())
}

async fn run_connection(torrent: V1Torrent, our_id: InternalPeerId, stream: TcpStream, tx: Sender<PeerStartMessage>) -> Result<()> {
    let (send_to_processor, send_to_processor_rx) = mpsc::channel(1);
    let (recv_from_processor_tx, mut recv_from_processor) = mpsc::channel(1);
    let mut conn: Connection<HandshakeDecoder> = Connection::new(stream);

    let our_handshake = Handshake {
        peer_ctx: PeerContext {
            info_hash: torrent.info.info_hash,
            peer_id: Vec::clone(&our_id)
        }
    };
    conn.write_msg(our_handshake).await?;
    log::debug!("Sent handshake...");
    if let Some(handshake) = conn.read_msg().await? {
        let peer_msg = PeerStartMessage {
            handshake,
            rx: send_to_processor_rx,
            tx: recv_from_processor_tx,
        };
        tx.send(peer_msg).await?;
    } else {
        return Ok(());
    }

    let mut conn: Connection<MessagesDecoder> = conn.translate();

    loop {
        tokio::select! {
            Some(msg) = recv_from_processor.recv() => {
                conn.write_msg(msg).await?;
            },
            Ok(Some(msg)) = conn.read_msg() => {
                send_to_processor.send(msg).await?
            },
            else => {
                break;
            }
        }
    }
    Ok(())
}

pub struct Connection<D> {
    stream: TcpStream,
    buffer: BytesMut,
    decoder: PhantomData<D>,
}

impl <T: Encode, D: Decode<T = T>> Connection<D> {
    pub fn new(stream: TcpStream) -> Connection<D> {
        Connection {
            stream,
            // Allocate the buffer with 4kb of capacity.
            buffer: BytesMut::with_capacity(PIECE_BLOCK_SIZE + 4096),
            decoder: PhantomData
        }
    }

    fn translate<OT, OD: Decode<T= OT>>(self) -> Connection<OD> {
        Connection { stream: self.stream , buffer: self.buffer, decoder: PhantomData }
    }

    pub async fn read_msg(&mut self)-> Result<Option<T>>
    {
        let mut needs = 0;
        loop {
            if self.buffer.remaining() >= needs {
                let mut cloned = self.buffer.clone();
                match D::decode(&mut cloned) {
                    Ok(msg) => {
                        self.buffer = cloned;
                        return Ok(Some(msg))
                    },
                    Err(ProtocolError::TryGetError(TryGetError{requested, available: _})) => {
                        needs = requested
                    },
                    Err(e) => return Err(e.into()),
                }
            }

            let read_bytes = self.stream.read_buf(&mut self.buffer).await?;

            if 0 == read_bytes {
                // The remote closed the connection. For this to be
                // a clean shutdown, there should be no data in the
                // read buffer. If there is, this means that the
                // peer closed the socket while sending a frame.
                if self.buffer.is_empty() {
                    log::info!("Connection closed... ");
                    return Ok(None);
                } else {
                    return Err(PeerError::Other("connection reset by peer".into()).into());
                }
            }
        }
    }

    pub async fn write_msg(&mut self, msg: T) -> Result<()> {
        let mut b = BytesMut::new();
        msg.encode(&mut b);
        self.stream.write_all_buf(&mut b).await?;
        Ok(())
    }
}