use bytes::BytesMut;
use tokio::{net::TcpStream, sync::mpsc::Sender};
use anyhow::Result;
use crate::{model::{TrackerResponse, V1Torrent}, peer::state::PeerStartMessage};

pub async fn connect_torrent_peers(tracker_resp: TrackerResponse, tx: Sender<PeerStartMessage>) -> Result<()> {

    for peer in tracker_resp.peers.into_iter() {
        let conn = TcpStream::connect(peer.socket_addr()).await;
        match conn {
            Ok(conn) => {
                tokio::spawn(handle_conn(conn, tx.clone()));

            },
            Err(e) => {
                log::warn!("Unable to connect to peer: ip={:?}, err={:?}", peer.socket_addr(), e);
            }
        }
    }
    Ok(())
}

pub struct Connection {
    stream: TcpStream,
    buffer: BytesMut,
    decoder: Decoder,
}

impl Connection {
    pub fn new(stream: TcpStream) -> Connection {
        Connection {
            stream,
            // Allocate the buffer with 4kb of capacity.
            buffer: BytesMut::with_capacity(4096),
        }
    }

    pub async fn read_frame(&mut self) -> Result<Option<Frame>>
{
    loop {
        // Attempt to parse a frame from the buffered data. If
        // enough data has been buffered, the frame is
        // returned.
        if let Some(frame) = self.parse_frame()? {
            return Ok(Some(frame));
        }

        // There is not enough buffered data to read a frame.
        // Attempt to read more data from the socket.
        //
        // On success, the number of bytes is returned. `0`
        // indicates "end of stream".
        if 0 == self.stream.read_buf(&mut self.buffer).await? {
            // The remote closed the connection. For this to be
            // a clean shutdown, there should be no data in the
            // read buffer. If there is, this means that the
            // peer closed the socket while sending a frame.
            if self.buffer.is_empty() {
                return Ok(None);
            } else {
                return Err("connection reset by peer".into());
            }
        }
    }
}
}