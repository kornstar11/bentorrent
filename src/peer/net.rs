use std::marker::PhantomData;

use bytes::{BufMut, BytesMut, TryGetError};
use futures::stream;
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpStream, sync::mpsc::{self, Sender}};
use tokio::io::BufWriter;
use anyhow::Result;
use crate::{model::{TrackerResponse, V1Torrent}, peer::{error::PeerError, protocol::{Decode, Encode, Handshake, HandshakeDecoder, Messages, MessagesDecoder}, state::PeerStartMessage}};

pub async fn connect_torrent_peers(tracker_resp: TrackerResponse, tx: Sender<PeerStartMessage>) -> Result<()> {

    for peer in tracker_resp.peers.into_iter() {
        let stream = TcpStream::connect(peer.socket_addr()).await;
        match stream {
            Ok(stream) => {
                tokio::spawn(run_connection(stream, tx.clone()));
            },
            Err(e) => {
                log::warn!("Unable to connect to peer: ip={:?}, err={:?}", peer.socket_addr(), e);
            }
        }
    }
    Ok(())
}

async fn run_connection(mut stream: TcpStream, tx: Sender<PeerStartMessage>) -> Result<()> {
    let (send_to_processor, send_to_processor_rx) = mpsc::channel(1);
    let (recv_from_processor_tx, mut recv_from_processor) = mpsc::channel(1);
    let mut conn: Connection<HandshakeDecoder> = Connection::new(stream);
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
            buffer: BytesMut::with_capacity(4096),
            decoder: PhantomData
        }
    }

    fn translate<OT, OD: Decode<T= OT>>(self) -> Connection<OD> {
        Connection { stream: self.stream , buffer: self.buffer, decoder: PhantomData }
    }

    pub async fn read_msg(&mut self)-> Result<Option<T>>
    {
        loop {
            if let Ok(msg) = D::decode(&mut self.buffer) {
                return Ok(Some(msg));
            }

            if 0 == self.stream.read_buf(&mut self.buffer).await? {
                // The remote closed the connection. For this to be
                // a clean shutdown, there should be no data in the
                // read buffer. If there is, this means that the
                // peer closed the socket while sending a frame.
                if self.buffer.is_empty() {
                    log::info!("Connection closed...");
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
        self.stream.write_buf(&mut b);
        Ok(())
    }
}