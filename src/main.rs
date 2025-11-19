use std::fs::read;

use bentorrent::{config::Config, model::V1Torrent, peer::start_processing};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::init();
    //let torrent = read("./test_data/ubuntu-25.10-desktop-amd64.iso.torrent").unwrap();
    let torrent = read("./test_data/debian-13.2.0-amd64-netinst.iso.torrent").unwrap();
    let config = Config::new();
    match bentorrent::file::parse_bencode(&torrent) {
        Ok(parsed) => {
            let torrent = V1Torrent::try_from(parsed).unwrap();
            if let Err(e) = start_processing(torrent, config).await {
                log::error!("Processor error: {:?}", e);
            }
            log::info!("Done!");
        }
        Err(e) => {
            eprintln!("Missing: {:?}", e);
        }
    }
}
