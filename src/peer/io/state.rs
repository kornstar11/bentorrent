use crate::config::Config;
use crate::peer::error::PeerError;
use crate::peer::io::TorrentIO;
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::sync::oneshot::{Sender as OSSender, channel as os_channel};

use anyhow::Result;

pub struct ReadMessage {
    pub index: u32,
    pub begin: u32,
    pub length: u32,
    pub cb: OSSender<Result<Vec<u8>>>,
}

pub struct WriteMessage {
    pub index: u32,
    pub begin: u32,
    pub block: Vec<u8>,
    pub cb: OSSender<Result<bool>>,
}

pub enum IoMessage {
    Read(ReadMessage),
    Write(WriteMessage),
}
pub struct IoHandlerParams {
    pub io: Box<dyn TorrentIO>,
    pub rx: Receiver<IoMessage>,
}
async fn io_handler_task(params: IoHandlerParams) -> Result<()> {
    let IoHandlerParams { mut io, mut rx } = params;
    while let Some(msg) = rx.recv().await {
        match msg {
            IoMessage::Read(ReadMessage {
                index,
                begin,
                length,
                cb,
            }) => {
                let res = io.read_piece(index, begin, length).await;
                let _ = cb.send(res);
            }
            IoMessage::Write(WriteMessage {
                index,
                begin,
                block,
                cb,
            }) => {
                let res = io.write_piece(index, begin, block).await;
                let _ = cb.send(res);
            }
        }
    }

    Ok(())
}
#[derive(Clone)]
pub struct IoHandler {
    tx: Sender<IoMessage>,
}

impl IoHandler {
    pub async fn new(config: Config, io: Box<dyn TorrentIO>) -> Result<Self> {
        let (tx, rx) = channel(config.file_handler_channel_size);
        let _ = tokio::spawn(async move { io_handler_task(IoHandlerParams { io, rx }).await });

        Ok(Self { tx })
    }

    pub async fn write(&self, index: u32, begin: u32, block: Vec<u8>) -> Result<bool> {
        let (cb_tx, cb_rx) = os_channel();
        if let Err(_) = self
            .tx
            .send(IoMessage::Write(WriteMessage {
                index,
                begin,
                block,
                cb: cb_tx,
            }))
            .await
        {
            return Err(PeerError::other("file handler exits...").into());
        }
        if let Ok(res) = cb_rx.await {
            res
        } else {
            return Err(PeerError::other("file handler exits...").into());
        }
    }

    pub async fn read(&self, index: u32, begin: u32, length: u32) -> Result<Vec<u8>> {
        let (cb_tx, cb_rx) = os_channel();
        if let Err(_) = self
            .tx
            .send(IoMessage::Read(ReadMessage {
                index,
                begin,
                length,
                cb: cb_tx,
            }))
            .await
        {
            return Err(PeerError::other("file handler exits...").into());
        }
        if let Ok(res) = cb_rx.await {
            res
        } else {
            return Err(PeerError::other("file handler exits...").into());
        }
    }
}
