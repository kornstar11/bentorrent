use bentorrent;
use std::fs::read;

fn main() {
    let torrent = read("/Users/bkornmeier/Downloads/ubuntu-25.10-desktop-amd64.iso.torrent").unwrap();
    match bentorrent::parse_bencode(&torrent) {
        Ok((_, parsed)) => {
            println!("{:#?}", parsed);
        },
        Err(nom::Err::Error(e)) => {
            eprintln!("Error: {:?}", e);
        }
        Err(nom::Err::Failure(e)) => {
            eprintln!("Fail: {:?}", e);
        }
    }


}
