use std::{net::IpAddr, str::FromStr};

use crate::file::{Bencode, Error, map_dict_keys};

#[derive(Debug, Clone)]
pub struct TrackerPeer {
    peer_id: Vec<u8>,
    address: IpAddr,
    port: u32,
}
impl<'a> TryFrom<Bencode<'a>> for TrackerPeer {
    type Error = Error;
    
    fn try_from(value: Bencode) -> Result<Self, Self::Error> {
        if let Bencode::Dictionary(dict) = value {
            let mut dict = map_dict_keys(dict);
            if let (
                Some(Bencode::ByteString(peer_id)),
                Some(Bencode::ByteString(ip)),
                Some(Bencode::Int(port))
            ) = (dict.remove("peer id"), dict.remove("ip"), dict.remove("port")) {
                let ip  = ip
                    .to_string();
                let ip  = IpAddr::from_str(&ip)
                    .map_err(|err| Error::BencodeParse(err.to_string()))?;
                return Ok(TrackerPeer { 
                    peer_id: peer_id.elements.to_vec(),
                    address: ip,
                    port: port as _ 
                })
            } else {
                return Err(Error::missing("missing peerid, ip or port"))
            }
        } else {
            Err(Error::WrongType)
        }
    }
    
}
#[derive(Debug, Clone)]
pub struct TrackerResponse {
    complete: i64,
    incomplete: i64,
    interval: i64,
    peers: Vec<TrackerPeer>
}

impl<'a> TryFrom<Bencode<'a>> for TrackerResponse {
    type Error = Error;

    fn try_from(value: Bencode<'a>) -> Result<Self, Self::Error> {
        if let Bencode::Dictionary(dict) = value {
            let mut dict = map_dict_keys(dict);
            if let (
                Some(Bencode::Int(complete)),
                Some(Bencode::Int(incomplete)),
                Some(Bencode::Int(interval)),
                Some(Bencode::List(peers)),
            ) = (dict.remove("complete"), dict.remove("incomplete"), dict.remove("interval"), dict.remove("peers")) {
                let peers = peers
                    .into_iter()
                    .map(|peer| {
                        TrackerPeer::try_from(peer)
                    }).flatten()
                    .collect::<Vec<_>>();
                Ok(TrackerResponse { complete, incomplete, interval, peers })
            } else {
                Err(Error::WrongType)
            }
        } else {
            Err(Error::WrongType)
        }
    }
}