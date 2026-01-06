use anyhow::Result;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use reqwest::Client;
use url::Url;

use crate::{
    file::parse_bencode,
    model::{TrackerResponse, V1Torrent},
    peer::InternalPeerId,
};

pub struct UploadDownloadState {
    uploaded: Arc<AtomicUsize>,
    downloaded: Arc<AtomicUsize>,
}

impl UploadDownloadState {
    pub fn new() -> Self {
        Self {
            uploaded: Arc::new(AtomicUsize::new(0)),
            downloaded: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn get_downloaded(&self) -> usize {
        self.downloaded.load(Ordering::Relaxed)
    }
    pub fn get_uploaded(&self) -> usize {
        self.uploaded.load(Ordering::Relaxed)
    }
}

pub struct TrackerClient {
    torrent: V1Torrent,
    my_peer_id: InternalPeerId,
    client: Client,
    pub upload_download_state: UploadDownloadState,
}

impl TrackerClient {
    pub fn new(torrent: V1Torrent, client: Client, my_peer_id: InternalPeerId) -> Self {
        Self {
            torrent,
            my_peer_id,
            client,
            upload_download_state: UploadDownloadState::new(),
        }
    }

    pub async fn get_announce(&self) -> Result<TrackerResponse> {
        let url = self.tracker_url();
        log::info!("Tracker URL: {}", url);
        let res = self.client.get(url).send().await?;
        let text = res.bytes().await?;
        let decoded = parse_bencode(&text)?;
        let decoded = TrackerResponse::try_from(decoded)?;
        Ok(decoded)
    }

    fn tracker_url(&self) -> String {
        let info_hash = urlencoding::encode_binary(&self.torrent.info.info_hash).to_string();
        let peer_id = urlencoding::encode_binary(&self.my_peer_id).to_string();
        let mut url = Url::parse(&self.torrent.announce).unwrap();
        let _ = url
            .query_pairs_mut()
            .append_pair("port", "6881")
            .append_pair(
                "uploaded",
                &format!("{}", self.upload_download_state.get_uploaded()),
            )
            .append_pair(
                "downloaded",
                &format!("{}", self.upload_download_state.get_downloaded()),
            )
            .append_pair("left", &self.torrent.info.length.to_string())
            .append_pair("numwant", "100")
            //.append_pair("compact", "1")
            .finish();

        format!(
            "{}&info_hash={}&peer_id={}",
            url.to_string(),
            info_hash,
            peer_id
        )
    }
}
