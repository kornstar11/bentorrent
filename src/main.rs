use bentorrent;
use std::fs::read;

fn main() {
    let torrent = read("/Users/bkornmeier/Downloads/ubuntu-25.10-desktop-amd64.iso.torrent").unwrap();
    match bentorrent::parse_bencode(&torrent) {
        Ok((_, parsed)) => {
            println!("{:#?}", parsed);
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
