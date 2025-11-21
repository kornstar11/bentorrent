use std::{fmt, marker::PhantomData, sync::Arc, time::Duration};
//use std::net::{IpAddr, Ipv4Addr};

use bytes::{Buf, BytesMut, TryGetError};
use futures::{StreamExt, stream::FuturesUnordered};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream, sync::mpsc::{self, Sender}, time::timeout};
use anyhow::Result;
use crate::{config::Config, model::{PeerContext, TrackerResponse, V1Torrent}, peer::{InternalPeerId, PIECE_BLOCK_SIZE, error::PeerError, protocol::{Decode, Encode, Handshake, HandshakeDecoder, MessagesDecoder, ProtocolError}, state::PeerStartMessage}};

pub async fn connect_torrent_peers(torrent: V1Torrent, our_id: InternalPeerId, tracker_resp: TrackerResponse, tx: Sender<PeerStartMessage>, config: Config) -> Result<()> {
    let mut active_conns = FuturesUnordered::new();
    log::debug!("Begin initial connection to peers: {:?}", tracker_resp.peers);
    //let peers = tracker_resp.peers.iter().filter(|p| p.address == IpAddr::V4(Ipv4Addr::new(188, 68, 53, 4)));
    let peers = tracker_resp.peers.iter();
    for peer in peers.into_iter() {
        if active_conns.len() >= config.max_conns {
            break;
        }
        log::debug!("Attempting to connect to {:?}", peer);
        //let stream = TcpStream::connect(peer.socket_addr()).await;
        if let Ok(stream) = timeout(
            Duration::from_secs(1), 
            TcpStream::connect(peer.socket_addr())
        ).await {
            match stream {
                Ok(stream) => {
                    log::info!("Connected to {:?}", peer.address);
                    if let Ok((handshake, conn)) = timeout(
                        Duration::from_secs(1),
                        do_handshake(torrent.clone(), Arc::clone(&our_id), stream)
                    ).await.map_err(|e| e.into()).flatten() {
                        active_conns.push(run_connection(handshake, conn, tx.clone(), config.clone()));
                    } else {
                        log::warn!("Failed to handshake with {:?}", peer.socket_addr());
                    }
                },
                Err(e) => {
                    log::warn!("Unable to connect to peer: ip={:?}, err={:?}", peer.socket_addr(), e);
                }
            }
        } else {
            log::warn!("Connection timeout {:?}", peer);
        }
    }
    log::info!("Peer connections spawned: {}", active_conns.len());
    while let Some(r) = active_conns.next().await {
        log::info!("Connection finished: {:?}", r);
    }

    Ok(())
}

async fn do_handshake(torrent: V1Torrent, our_id: InternalPeerId, stream: TcpStream) -> Result<(Handshake, Connection<MessagesDecoder>)> {
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
        Ok((handshake, conn.translate()))
    } else {
        Err(PeerError::other("Expected a handshake, but got an unknown message.").into())
    }
}

async fn run_connection(handshake: Handshake, mut conn: Connection<MessagesDecoder>, tx: Sender<PeerStartMessage>, config: Config) -> Result<()> {
    let (send_to_processor, send_to_processor_rx) = mpsc::channel(config.peer_rx_size); //todo: config this
    let (recv_from_processor_tx, mut recv_from_processor) = mpsc::channel(config.peer_tx_size);

     let peer_msg = PeerStartMessage {
        handshake,
        rx: send_to_processor_rx,
        tx: recv_from_processor_tx,
    };
    tx.send(peer_msg).await?;

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

impl <T, D> Connection<D> where T: Encode + fmt::Debug, D: Decode<T = T> {
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

                //let mut cloned = self.buffer.clone(); //yuck!
                let mut read_buf = self.buffer.as_ref();
                let remaining = read_buf.remaining();
                match D::decode(&mut read_buf) {
                    Ok(msg) => {
                        let advance = remaining - read_buf.remaining();
                        self.buffer.advance(advance);
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
        log::debug!("Writing: {:?} :: {:?}", self.stream, msg);
        msg.encode(&mut b);
        self.stream.write_all_buf(&mut b).await?;
        log::debug!("Wrote: {:?}", self.stream);
        Ok(())
    }
}