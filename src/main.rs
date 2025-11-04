use bentorrent;
use std::fs::read;

fn main() {
    let torrent = read("/Users/bkornmeier/Downloads/ubuntu-25.10-desktop-amd64.iso.torrent").unwrap();
    let s = String::from_utf8_lossy(&torrent);
    let (_, parsed) = bentorrent::parse_bencode(&s).unwrap();

    println!("{:#?}", parsed);

}
