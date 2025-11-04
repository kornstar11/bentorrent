use bentorrent;
use std::fs::read;

fn main() {
    let torrent = read("/Users/bkornmeier/Downloads/ubuntu-25.10-desktop-amd64.iso.torrent").unwrap();
    let (_, parsed) = bentorrent::parse_bencode(&torrent).unwrap();

    println!("{:#?}", parsed);

}
