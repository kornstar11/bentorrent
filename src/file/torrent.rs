use std::collections::HashMap;

use crate::file::{Bencode, Error};


pub struct V1Piece {
    hash: Vec<u8>
}
// Torrent File spec
pub struct V1TorrentInfo {
    length: usize,
    pieces: Vec<V1Piece>,
    name: String,
}

pub struct V1Torrent {
    announce_list: Vec<String>
}


impl<'a> TryFrom<Bencode<'a>> for V1Torrent {
    type Error = Error;
    fn try_from(bc: Bencode<'a>) -> Result<Self, Self::Error> {
        if let Bencode::Dictionary(eles) = bc {
            let conv = eles.iter().map(|(k, v)|{
                (k.to_string(), v)
            }).collect::<HashMap<_, _>>();

        }
        todo!()

    }

}