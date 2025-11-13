/*
Dictionary(
                    {
                        ByteString {
                            elements: "ip",
                        }: ByteString(
                            ByteString {
                                elements: "2001:470:1f06:32b::2",
                            },
                        ),
                        ByteString {
                            elements: "peer id",
                        }: ByteString(
                            ByteString {
                                elements: "-TR410B-e1q7lgey4yby",
                            },
                        ),
                        ByteString {
                            elements: "port",
                        }: Int(
                            54555,
                        ),
                    },
                ),

*/

use std::{collections::BTreeMap, net::IpAddr, str::FromStr};

use anyhow::Context;

use crate::file::{Bencode, Error, map_dict_keys};

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
                let ip:IpAddr = IpAddr::from_str(&ip)
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