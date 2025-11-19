use std::{net::{IpAddr, SocketAddrV4, SocketAddrV6, SocketAddr}, str::FromStr};

use crate::file::{Bencode, Error, map_dict_keys};

#[derive(Debug, Clone)]
pub struct TrackerPeer {
    pub address: IpAddr,
    pub port: u16,
}

impl TrackerPeer {
    pub fn socket_addr(&self) -> SocketAddr {
        match self.address {
            IpAddr::V4(ip) => {
                SocketAddr::V4(SocketAddrV4::new(ip, self.port))
            },
            IpAddr::V6(ip) => {
                SocketAddr::V6(SocketAddrV6::new(ip, self.port, 0, 0))
            }
        }
    }
    
}

impl<'a> TryFrom<Bencode<'a>> for TrackerPeer {
    type Error = Error;
    
    fn try_from(value: Bencode) -> Result<Self, Self::Error> {
        if let Bencode::Dictionary(dict) = value {
            let mut dict = map_dict_keys(dict);
            if let (
                Some(Bencode::ByteString(ip)),
                Some(Bencode::Int(port))
            ) = (dict.remove("ip"), dict.remove("port")) {
                let ip  = ip
                    .to_string();
                let ip  = IpAddr::from_str(&ip)
                    .map_err(|err| Error::BencodeParse(err.to_string()))?;

                return Ok(TrackerPeer { 
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
    pub interval: i64,
    pub peers: Vec<TrackerPeer>,
    pub complete: Option<i64>,
    pub incomplete: Option<i64>,
}

impl<'a> TryFrom<Bencode<'a>> for TrackerResponse {
    type Error = Error;

    fn try_from(value: Bencode<'a>) -> Result<Self, Self::Error> {
        if let Bencode::Dictionary(dict) = value {
            let mut dict = map_dict_keys(dict);
            if let (
                Some(Bencode::Int(interval)),
                Some(Bencode::List(peers)),
                complete,
                incomplete,
            ) = (dict.remove("interval"), dict.remove("peers"), dict.remove("complete"), dict.remove("incomplete")) {
                let complete = if let Some(Bencode::Int(i)) = complete {
                    Some(i)
                } else {
                    None
                };
                let incomplete = if let Some(Bencode::Int(i)) = incomplete {
                    Some(i)
                } else {
                    None
                };
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