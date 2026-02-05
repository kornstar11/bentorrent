use std::{fs::read, path::PathBuf};
use clap::Parser;
use bentorrent::{config::Config, model::V1Torrent, peer::start_processing};

#[derive(Debug, Parser)]
#[command(about="Silly Bittorrent peer. Don't expect too much!")]
struct Cli {
    #[arg(short = 'f')]
    torrent_location: PathBuf,
    #[command(flatten)]
    config: Config
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::init();
    let cli = Cli::parse();
    let torrent = read(cli.torrent_location).unwrap();
    match bentorrent::file::parse_bencode(&torrent) {
        Ok(parsed) => {
            let torrent = V1Torrent::try_from(parsed).unwrap();
            if let Err(e) = start_processing(torrent, cli.config).await {
                log::error!("Processor error: {:?}", e);
            }
            log::info!("Done!");
        }
        Err(e) => {
            eprintln!("Missing: {:?}", e);
        }
    }
}
