//https://wiki.theory.org/BitTorrentSpecification#Tracker_HTTP.2FHTTPS_Protocol

use crate::model::PeerContext;
use bytes::{Buf, BufMut, BytesMut, TryGetError};
use nom::AsBytes;
use thiserror::Error;

const HANDSHAKE_PROTOCOL_ID: &[u8] = "BitTorrent protocol".as_bytes();

type Result<T> = std::result::Result<T, ProtocolError>;

#[derive(Error, Debug)]
pub enum ProtocolError {
    #[error("try get error")]
    TryGetError(#[from] TryGetError),
    #[error("Bad id")]
    BadId,
    #[error("OtherMessage")]
    OtherMessage,
}

///
/// Simple traits for encode/decode
pub trait Encode {
    fn encode<B: BufMut>(&self, i: &mut B);
}

pub trait Decode: Send {
    type T;
    fn decode<B: Buf>(b: &mut B) -> Result<Self::T>;
}

/// Handshake packet
// handshake: <pstrlen><pstr><reserved><info_hash><peer_id>
// pstrlen: string length of <pstr>, as a single raw byte
// pstr: string identifier of the protocol
// reserved: eight (8) reserved bytes. All current implementations use all zeroes. Each bit in these bytes can be used to change the behavior of the protocol. An email from Bram suggests that trailing bits should be used first, so that leading bits may be used to change the meaning of trailing bits.
// info_hash: 20-byte SHA1 hash of the info key in the metainfo file. This is the same info_hash that is transmitted in tracker requests.
// peer_id: 20-byte string used as a unique ID for the client. This is usually the same peer_id that is transmitted in tracker requests (but not always e.g. an anonymity option in Azureus).
#[derive(Debug, Clone)]
pub struct Handshake {
    pub peer_ctx: PeerContext,
}
impl Encode for Handshake {
    fn encode<B: BufMut>(&self, i: &mut B) {
        i.put_u8(HANDSHAKE_PROTOCOL_ID.len() as u8);
        i.put(HANDSHAKE_PROTOCOL_ID);
        i.put_u64(0);

        i.put(self.peer_ctx.info_hash.as_slice());
        i.put(self.peer_ctx.peer_id.as_slice())
    }
}
pub struct HandshakeDecoder;

impl Decode for HandshakeDecoder {
    type T = Handshake;

    fn decode<B: Buf>(b: &mut B) -> Result<Self::T> {
        let p_str_len = b.try_get_u8()?;
        let mut p_str = vec![0 as u8; p_str_len as usize];
        b.try_copy_to_slice(&mut p_str)?;
        b.advance(8); //skip 8 bytes

        let mut info_hash = vec![0 as u8; 20];
        b.try_copy_to_slice(&mut info_hash)?;

        let mut peer_id = vec![0 as u8; 20];
        b.try_copy_to_slice(&mut peer_id)?;

        Ok(Handshake {
            peer_ctx: PeerContext { info_hash, peer_id },
        })
    }
}

///
/// Simple flag based messages
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FlagMessages {
    Choke = 0,
    Unchoke = 1,
    Interested = 2,
    NotInterested = 3,
}

impl FlagMessages {
    pub fn interest_msg(interest: bool) -> Messages {
        let msg = if interest {
            FlagMessages::Interested
        } else {
            FlagMessages::NotInterested
        };
        Messages::Flag(msg)
    }
    
}

impl TryFrom<u8> for FlagMessages {
    type Error = ProtocolError;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(FlagMessages::Choke),
            1 => Ok(FlagMessages::Unchoke),
            2 => Ok(FlagMessages::Interested),
            3 => Ok(FlagMessages::NotInterested),
            _ => Err(ProtocolError::BadId),
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Messages {
    KeepAlive,
    Flag(FlagMessages),
    Have {
        piece_index: u32,
    },
    BitField {
        bitfield: Vec<u8>,
    },
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
}

impl Encode for Messages {
    fn encode<B: BufMut>(&self, i: &mut B) {
        match self {
            Self::KeepAlive => {
                i.put_u32(0);
            }
            Self::Flag(flag) => {
                i.put_u32(1);
                i.put_u8((*flag) as u8);
            }
            Self::Have { piece_index } => {
                i.put_u32(5);
                i.put_u8(4);
                i.put_u32(*piece_index);
            }
            Self::BitField { bitfield } => {
                i.put_u32(1 + bitfield.len() as u32);
                i.put_u8(5);
                i.put_slice(&bitfield);
            }
            Self::Request {
                index,
                begin,
                length,
            } => {
                i.put_u32(13);
                i.put_u8(6);
                i.put_u32(*index);
                i.put_u32(*begin);
                i.put_u32(*length);
            }
            Self::Piece {
                index,
                begin,
                block,
            } => {
                i.put_u32(9 + block.len() as u32);
                i.put_u8(7);
                i.put_u32(*index);
                i.put_u32(*begin);
                i.put_slice(&block);
            }
            Self::Cancel {
                index,
                begin,
                length,
            } => {
                i.put_u32(13);
                i.put_u8(8);
                i.put_u32(*index);
                i.put_u32(*begin);
                i.put_u32(*length);
            }
        }
    }
}
pub struct MessagesDecoder;

impl Decode for MessagesDecoder {
    type T = Messages;

    fn decode<B: Buf>(b: &mut B) -> Result<Self::T> {
        let len = b.try_get_u32()? as usize;
        if len == 0 {
            return Ok(Messages::KeepAlive);
        }

        let id = b.try_get_u8()?;

        match id {
            4 => Ok(Messages::Have {
                piece_index: b.try_get_u32()?,
            }),
            5 => {
                let mut bitfield = vec![0 as u8; len - 1];
                b.try_copy_to_slice(&mut bitfield)?;
                Ok(Messages::BitField { bitfield: bitfield })
            }
            6 => {
                let index = b.try_get_u32()?;
                let begin = b.try_get_u32()?;
                let length = b.try_get_u32()?;
                Ok(Messages::Request {
                    index,
                    begin,
                    length,
                })
            }
            7 => {
                let index = b.try_get_u32()?;
                let begin = b.try_get_u32()?;
                let mut block = vec![0 as u8; len - 9];
                b.try_copy_to_slice(&mut block)?;
                Ok(Messages::Piece {
                    index,
                    begin,
                    block,
                })
            }
            8 => {
                let index = b.try_get_u32()?;
                let begin = b.try_get_u32()?;
                let length = b.try_get_u32()?;
                Ok(Messages::Cancel {
                    index,
                    begin,
                    length,
                })
            }
            i if len == 1 => Ok(Messages::Flag(FlagMessages::try_from(i)?)),
            _ => Err(ProtocolError::BadId),
        }
    }
}

#[cfg(test)]
mod test {
    use std::time::SystemTime;

    use bytes::{Buf, Bytes};
    use sha1::{Digest, Sha1};

    use super::*;

    fn do_sha1_hash(b: &[u8]) -> Vec<u8> {
        let mut hasher = Sha1::new();
        hasher.update(b);
        return hasher.finalize().to_vec();
    }

    #[test]
    fn round_trip_handshake() {
        let time = SystemTime::now();
        let info_hash = do_sha1_hash(format!("info:{:?}", time).as_bytes());
        let peer_id = do_sha1_hash(format!("peer:{:?}", time).as_bytes());

        let hs = Handshake {
            peer_ctx: PeerContext {
                info_hash: info_hash.clone(),
                peer_id: peer_id.clone(),
            },
        };
        let mut buf = BytesMut::new();
        hs.encode(&mut buf);

        let mut buf_clone = buf.clone();
        assert_eq!(buf_clone.get_u8(), 19);
        let p_str = buf_clone.copy_to_bytes(19);
        assert_eq!(p_str, HANDSHAKE_PROTOCOL_ID);

        let handshake = HandshakeDecoder::decode(&mut buf).unwrap();
        assert_eq!(handshake.peer_ctx.info_hash, info_hash)
    }

    #[test]
    fn handshake_cap() {
        let hs_b_orig = hex::decode("13426974546f7272656e742070726f746f636f6c00000000000000000164fe7ef1105c57764170edf603c439d64214f1b86a737fe80caf680259963724652756ee4d165b")
            .unwrap();
        let mut bytes = BytesMut::from(hs_b_orig.as_slice());
        let hs = HandshakeDecoder::decode(&mut bytes).unwrap();
        let mut hs_b = BytesMut::new();
        hs.encode(&mut hs_b);
        let hs_b = hs_b.to_vec();
        assert_eq!(hs_b, hs_b_orig);
    }
    #[test]
    fn bitfield_cap() {
        let bf_b_orig =
            hex::decode("0000001905fffffffffffffffffffffffffffffffffffffffffffffffe").unwrap();
        let mut bytes = BytesMut::from(bf_b_orig.as_slice());
        let msg = MessagesDecoder::decode(&mut bytes).unwrap();
        let mut hs_b = BytesMut::new();
        msg.encode(&mut hs_b);
        let hs_b = hs_b.to_vec();
        assert_eq!(hs_b, bf_b_orig);
    }
}
