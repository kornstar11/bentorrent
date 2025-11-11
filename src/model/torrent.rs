use std::collections::HashMap;

use crate::file::{Bencode, ByteString, Error};
use sha1::{Digest, Sha1};
//https://en.wikipedia.org/wiki/Torrent_file

#[derive(Clone, Debug)]
pub struct V1Piece {
    pub hash: Vec<u8>,
}

mod util {
    use std::collections::HashMap;

    use crate::file::{Bencode, DictT};

    pub fn convert_dict_keys<'a>(eles: DictT<'a>) -> HashMap<String, Bencode<'a>> {
        eles.into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect::<HashMap<_, _>>()
    }
}
// Torrent File spec
#[derive(Clone, Debug)]
pub struct V1TorrentInfo {
    pub length: i64,
    pub pieces: Vec<V1Piece>,
    pub name: String,
    pub info_hash: Vec<u8>,
}

impl V1TorrentInfo {
    fn info_hash<'a>(bc: &Bencode<'a>) -> Vec<u8> {
        let mut hasher = Sha1::new();
        let info = Bencode::encode(&bc);
        hasher.update(info);
        return hasher.finalize().to_vec();
    }
}

impl<'a> TryFrom<Bencode<'a>> for V1TorrentInfo {
    type Error = Error;
    fn try_from(bc: Bencode<'a>) -> Result<Self, Self::Error> {
        let info_hash = Self::info_hash(&bc);

        if let Bencode::Dictionary(eles) = bc {
            let conv = util::convert_dict_keys(eles);
            let length = if let Some(Bencode::Int(len)) = conv.get("length") {
                *len
            } else {
                return Err(Error::missing("length"));
            };

            // pieces parsing
            let pieces =
                if let Some(Bencode::ByteString(ByteString { elements })) = conv.get("pieces") {
                    let mut pieces = vec![];
                    if elements.len() % 20 != 0 {
                        return Err(Error::WrongType);
                    }
                    // create slices of 20byte length for the sha1
                    let (chunks, _) = elements.as_chunks::<20>();
                    for chunk in chunks.into_iter() {
                        pieces.push(V1Piece {
                            hash: chunk.to_vec(),
                        });
                    }
                    pieces
                } else {
                    return Err(Error::missing("pieces"));
                };

            let name = if let Some(Bencode::ByteString(ByteString { elements })) = conv.get("name")
            {
                String::from_utf8_lossy(&elements).to_string()
            } else {
                return Err(Error::missing("name"));
            };
            Ok(V1TorrentInfo {
                length,
                pieces,
                name,
                info_hash,
            })
        } else {
            Err(Error::WrongType)
        }
    }
}
#[derive(Debug, Clone)]
pub struct V1Torrent {
    pub info: V1TorrentInfo,
    pub announce: String,
    pub announce_list: Vec<String>,
}

impl<'a> TryFrom<Bencode<'a>> for V1Torrent {
    type Error = Error;
    fn try_from(bc: Bencode<'a>) -> Result<Self, Self::Error> {
        if let Bencode::Dictionary(eles) = bc {
            let conv = eles
                .iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect::<HashMap<_, _>>();

            let info = if let Some(d) = conv.get("info") {
                V1TorrentInfo::try_from((*d).clone())?
            } else {
                return Err(Error::missing("info"));
            };

            let announce =
                if let Some(Bencode::ByteString(ByteString { elements })) = conv.get("announce") {
                    String::from_utf8_lossy(&elements).to_string()
                } else {
                    return Err(Error::missing("announce"));
                };

            Ok(V1Torrent {
                info,
                announce,
                announce_list: vec![],
            })
        } else {
            Err(Error::WrongType)
        }
    }
}
