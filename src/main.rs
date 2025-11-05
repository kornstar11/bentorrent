use std::{fs::read, time::SystemTime};

use bentorrent::file::V1Torrent;
use sha1::Digest;
use url::Url;

fn make_peer_id() -> String {
    let mut sha1 = sha1::Sha1::new();
    let timestamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();
    let id = format!("ben-{}",timestamp.as_secs());
    println!("Id: {}", id);
    sha1.update(id);
    return String::from_utf8_lossy(&sha1.finalize()).to_string();
}

fn main() {
    let torrent = read("/Users/bkornmeier/Downloads/ubuntu-25.10-desktop-amd64.iso.torrent").unwrap();
    match bentorrent::file::parse_bencode(&torrent) {
        Ok((_, parsed)) => {
            //println!("{:#?}", parsed);
            let torrent = V1Torrent::try_from(parsed).unwrap();
            //println!("{:#?}", torrent);
            let mut url = Url::parse(&torrent.announce).unwrap();
            url.query_pairs_mut()
                .append_pair("info_hash", &String::from_utf8_lossy(&torrent.info.hash).to_string())
                .append_pair("peer_id", &make_peer_id())
                .append_pair("port", "6881")
                .append_pair("uploaded", "0")
                .append_pair("downloaded", "0")
                .append_pair("left", &format!("{}", torrent.info.length));

            println!("URL: {}", url.to_string());
        },
        Err(nom::Err::Incomplete(e)) => {
            eprintln!("Missing: {:?}", e);
        }
        Err(nom::Err::Error(nom::error::Error{input, code})) => {
            eprintln!("Code: {:?} Error: {:?}", code, String::from_utf8_lossy(input).to_string());
        }
        Err(nom::Err::Failure(e)) => {
            eprintln!("Fail: {:?}", e);
        }
    }


}
