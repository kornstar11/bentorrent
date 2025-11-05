use std::collections::HashMap;

use crate::file::{Bencode, bencode::ByteString, error::Error};

//https://en.wikipedia.org/wiki/Torrent_file

pub struct V1Piece {
    hash: Vec<u8>
}

mod util {
    use std::collections::HashMap;

    use crate::file::{Bencode, bencode::ByteString};

    pub fn convert_dict_keys<'a>(eles: HashMap<ByteString, Bencode<'a>>) -> HashMap<String, Bencode<'a>> {
        eles.into_iter().map(|(k, v)|{
            (k.to_string(), v)
        }).collect::<HashMap<_, _>>()
    }
}
// Torrent File spec
pub struct V1TorrentInfo {
    length: usize,
    pieces: Vec<V1Piece>,
    name: String,
}

impl<'a> TryFrom<Bencode<'a>> for V1TorrentInfo {
    type Error = Error;
    fn try_from(bc: Bencode<'a>) -> Result<Self, Self::Error> {
        if let Bencode::Dictionary(eles) = bc {
            let conv = util::convert_dict_keys(eles);
            let length = if let Some(Bencode::Int(len)) = conv.get("length") {
                len
            } else {
                return Err(Error::missing("length"));
            };

            // pieces parsing
            if let Some(Bencode::ByteString(ByteString{ elements})) = conv.get("pieces") {
                if elements.len() % 20 != 0{
                    return Err(Error::WrongType);
                }

            }
            Ok(V1TorrentInfo {
                length: 0,
                pieces: vec![],
                name: String::new()
            })
        } else {
            Err(Error::WrongType)
        }
    }
}

pub struct V1Torrent {
    info: V1TorrentInfo,
    announce_list: Vec<String>,
}


impl<'a> TryFrom<Bencode<'a>> for V1Torrent {
    type Error = Error;
    fn try_from(bc: Bencode<'a>) -> Result<Self, Self::Error> {
        if let Bencode::Dictionary(eles) = bc {
            let conv = eles.iter().map(|(k, v)|{
                (k.to_string(), v)
            }).collect::<HashMap<_, _>>();
            //conv.contains_key()

        }
        todo!()

    }

}