use std::{borrow::Cow, fs::read, time::SystemTime};

use bentorrent::file::V1Torrent;
use sha1::Digest;
use url::Url;

fn make_peer_id() -> Vec<u8> {
    let mut sha1 = sha1::Sha1::new();
    let timestamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();
    let id = format!("ben-{}",timestamp.as_secs());
    println!("Id: {}", id);
    sha1.update(id);
    return sha1.finalize().to_vec();
}

fn main() {
    let torrent = read("./test_data/ubuntu-25.10-desktop-amd64.iso.torrent").unwrap();
    match bentorrent::file::parse_bencode(&torrent) {
        Ok((_, parsed)) => {
            //println!("{:#?}", parsed);
            let torrent = V1Torrent::try_from(parsed).unwrap();
            //let info_hash = hex::encode(&torrent.info.hash);
            let info_hash = urlencoding::encode_binary(&torrent.info.hash).to_string();
            let peer_id = urlencoding::encode_binary(&make_peer_id()).to_string();
            let mut url = Url::parse(&torrent.announce).unwrap();
            url.query_pairs_mut()
                .append_pair("port", "6881")
                .append_pair("uploaded", "0")
                .append_pair("downloaded", "0")
                .append_pair("left", &format!("{}", torrent.info.length))
                .finish();

            println!("URL: {}&info_hash={}&peer_id={}", url.to_string(), info_hash, peer_id);
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
