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

use std::{collections::BTreeMap, net::IpAddr};

use anyhow::Context;

use crate::file::{Bencode, Error, map_dict_keys};

pub struct TrackerPeer {
    peer_id: Vec<u8>,
    address: IpAddr,
    port: u32,
}
impl TryFrom<Bencode> for TrackerPeer {
    type Error = Error;
    
    fn try_from(value: Bencode) -> Result<Self, Self::Error> {
        if let Some(Bencode::Dictionary(dict)) = value {
            let dict = map_dict_keys(dict);
            if let (
                Some(Bencode::ByteString(peer_id)),
                Some(Bencode::ByteString(ip)),
                Some(Bencode::Int(port))
            ) = (dict.remove("peer id"), dict.remove("ip"), dict.remove("port")) {
                let ip: IpAddr = ip.to_string().parse()?;
                return Ok(TrackerPeer { 
                    peer_id: peer_id.elements.to_vec(),
                    address: ip,
                    port: port as _ 
                })


            } else {
                return Err(Error::Missing("missing peerid, ip or port"))
            }
            
            todo!()

        } else {
            Err(Error::WrongType).context("Peers from tracker should be a Dictionary")
        }
    }
    
}