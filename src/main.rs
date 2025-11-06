use std::fs::read;

use bentorrent::{file::V1Torrent, peer::TrackerClient};
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let torrent = read("./test_data/ubuntu-25.10-desktop-amd64.iso.torrent").unwrap();
    match bentorrent::file::parse_bencode(&torrent) {
        Ok(parsed) => {
            let torrent = V1Torrent::try_from(parsed).unwrap();
            let client = reqwest::Client::new();
            let tracker_client = TrackerClient::new(torrent, client);
            tracker_client.get_announce().await;

        },
        Err(e) => {
            eprintln!("Missing: {:?}", e);
        }
    }


}
