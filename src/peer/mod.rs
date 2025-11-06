use std::{sync::{Arc, atomic::AtomicUsize}, time::SystemTime};

use sha1::Digest;
use url::Url;
use reqwest::Client;

use crate::file::{Bencode, V1Torrent, parse_bencode};

pub struct UploadDownloadState {
    uploaded: Arc<AtomicUsize>,
    downloaded: Arc<AtomicUsize>,
}

impl UploadDownloadState {
    pub fn new() -> Self {
        Self { 
            uploaded: Arc::new(AtomicUsize::new(0)), 
            downloaded: Arc::new(AtomicUsize::new(0)) 
        }
    }
    
}

pub struct TrackerClient {
    torrent: V1Torrent,
    peer_id: Vec<u8>,
    client: Client,
    pub upload_download_state: UploadDownloadState,
}

impl TrackerClient {
    
    pub fn new(torrent: V1Torrent, client: Client) -> Self {
        let peer_id = Self::make_peer_id();
        Self { 
            torrent, 
            peer_id,
            client,
            upload_download_state: UploadDownloadState::new()
         }
    }

    pub async fn get_announce(&self) {
        let url = self.tracker_url();
        let res = self
            .client
            .get(url)
            .send()
            .await
            .unwrap();
        let text = res.bytes().await.unwrap();
        let decoded = parse_bencode(&text).unwrap();


    }

    fn make_peer_id() -> Vec<u8> {
        let mut sha1 = sha1::Sha1::new();
        let timestamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();
        let id = format!("ben-{}",timestamp.as_secs());
        sha1.update(id);
        return sha1.finalize().to_vec();
    }

    fn tracker_url(&self) -> String {
        let info_hash = urlencoding::encode_binary(&self.torrent.info.hash).to_string();
            let peer_id = urlencoding::encode_binary(&self.peer_id).to_string();
            let mut url = Url::parse(&self.torrent.announce).unwrap();
            url.query_pairs_mut()
                .append_pair("port", "6881")
                .append_pair("uploaded", "0")
                .append_pair("downloaded", "0")
                .append_pair("left", &self.torrent.info.length.to_string())
                .finish();

            format!("{}&info_hash={}&peer_id={}", url.to_string(), info_hash, peer_id)
    }
    
}