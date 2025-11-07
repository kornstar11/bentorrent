//https://wiki.theory.org/BitTorrentSpecification#Tracker_HTTP.2FHTTPS_Protocol

use crate::model::PeerContext;
use anyhow::Result;
use bytes::{BytesMut, BufMut, Buf};
use nom::AsBytes;

const HANDSHAKE_PROTOCOL_ID: &[u8] = "BitTorrent protocol".as_bytes();

trait Encode {
    fn encode<B: BufMut>(&self, i: &mut B);
}

trait Decode {
    type T;
    fn decode<T: Buf>(b: &mut T) -> Result<Self::T>;
    
}

// handshake: <pstrlen><pstr><reserved><info_hash><peer_id>
// pstrlen: string length of <pstr>, as a single raw byte
// pstr: string identifier of the protocol
// reserved: eight (8) reserved bytes. All current implementations use all zeroes. Each bit in these bytes can be used to change the behavior of the protocol. An email from Bram suggests that trailing bits should be used first, so that leading bits may be used to change the meaning of trailing bits.
// info_hash: 20-byte SHA1 hash of the info key in the metainfo file. This is the same info_hash that is transmitted in tracker requests.
// peer_id: 20-byte string used as a unique ID for the client. This is usually the same peer_id that is transmitted in tracker requests (but not always e.g. an anonymity option in Azureus).
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

impl Decode for Handshake {
    type T = Self;

    fn decode<T: Buf>(b: &mut T) -> Result<Self::T> {
        let p_str_len = b.try_get_u8()?;
        let mut p_str = vec![0 as u8; p_str_len as usize];
        b.try_copy_to_slice(&mut p_str)?;
        b.advance(8); //skip 8 bytes

        let mut info_hash = vec![0 as u8; 20];
        b.try_copy_to_slice(&mut info_hash)?;
        
        let mut peer_id = vec![0 as u8; 20];
        b.try_copy_to_slice(&mut peer_id)?;

        Ok(
            Handshake { 
                peer_ctx: PeerContext { info_hash, peer_id }
            }
        )
    }
}

#[cfg(test)]
mod test {
    use std::time::SystemTime;

    use sha1::{Sha1, Digest};
    use bytes::Buf;

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

        let hs = Handshake{
            peer_ctx: PeerContext { info_hash: info_hash.clone(), peer_id: peer_id.clone() }
        };
        let mut buf = BytesMut::new();
        hs.encode(&mut buf);

        let mut buf_clone = buf.clone();
        assert_eq!(buf_clone.get_u8(), 19);
        let p_str = buf_clone.copy_to_bytes(19);
        assert_eq!(p_str, HANDSHAKE_PROTOCOL_ID);

        let handshake = Handshake::decode(&mut buf).unwrap();
        assert_eq!(handshake.peer_ctx.info_hash, info_hash)
    }
}