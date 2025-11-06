use std::{collections::BTreeMap, fmt::Debug};

use nom::{
    IResult, Parser, branch::alt, bytes::complete::{tag, take}, multi::many_till, sequence::delimited
};

use std::fmt;

use nom::character::complete::{u32, i64};
use super::error::Error;

pub type DictT<'a> = BTreeMap<ByteString<'a>, Bencode<'a>>;

// If the parser was successful, then it will return a tuple.
// The first field of the tuple will contain everything the parser did not process.
// The second will contain everything the parser processed.
#[derive(Clone, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ByteString<'a> {
    pub elements: &'a [u8],
}

impl<'a> ToString for ByteString<'a> {
    fn to_string(&self) -> String {
        return String::from_utf8_lossy(self.elements).to_string();
    }
}

impl<'a> Debug for ByteString<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let strs = self.to_string();
        f.debug_struct("ByteString")
            .field("elements", &strs)
            .finish()
    }
}

#[derive(Eq, PartialEq, Clone, Debug)]
pub enum Bencode<'a> {
    Int(i64),
    ByteString(ByteString<'a>),
    List(Vec<Bencode<'a>>),
    Dictionary(DictT<'a>),
}

impl <'a> Bencode<'a> {
    fn encode_int(i: i64) -> Vec<u8> {
        let bencode = format!("i{}e", i);
        return bencode.as_bytes().to_vec();
    }

    fn encode_bs(bs: &ByteString<'a>) -> Vec<u8> {
        let mut bs_string = format!("{}:", bs.elements.len())
            .as_bytes()
            .to_vec();
        bs_string.extend_from_slice(bs.elements);
        return bs_string;
    }

    fn encode_list(l: &Vec<Bencode<'a>>) -> Vec<u8> {
        let mut list_string = format!("l").as_bytes().to_vec();
        for bencode in l.iter() {
            let mut inner_bytes = Self::encode(bencode);
            list_string.append(&mut inner_bytes);
        }
        list_string.extend_from_slice("e".as_bytes());
        return list_string;
    }

    fn encode_dictionary(d: &DictT<'a>) -> Vec<u8> {
        let mut acc = vec![];
        acc.extend_from_slice("d".as_bytes());
        for (k, v) in d.iter() {
            let mut k = Self::encode_bs(k);
            let mut v = Self::encode(v);
            acc.append(&mut k);
            acc.append(&mut v);
        }
        acc.extend_from_slice("e".as_bytes());
        return acc;

    }
    pub fn encode(bc: &Bencode<'a>) -> Vec<u8> {
        match bc {
            Bencode::Int(i) => Self::encode_int(*i),
            Bencode::ByteString(bs) => Self::encode_bs(bs),
            Bencode::List(l) => Self::encode_list(l),
            Bencode::Dictionary(d) => Self::encode_dictionary(d),
        }
    }
    
}

// https://en.wikipedia.org/wiki/Bencode

fn parse_int(i: &[u8]) -> IResult<&[u8], Bencode<'_>> {
    let (leftover, number) = delimited(tag("i"), i64, tag("e")).parse(i)?;

    Ok((leftover, Bencode::Int(number)))
}

fn inner_parse_bytestring(i: &[u8]) -> IResult<&[u8], ByteString<'_>> {
    let (leftover, len) = u32.parse(i)?;
    let (leftover, _) = tag(":").parse(leftover)?;
    let (leftover, bs) = take(len).parse(leftover)?;
    Ok((leftover, ByteString { elements: bs }))
}

fn parse_bytestring(i: &[u8]) -> IResult<&[u8], Bencode<'_>> {
    let (leftover, bs) = inner_parse_bytestring(i)?;
    Ok((leftover, Bencode::ByteString(bs)))
}

fn parse_list(i: &[u8]) -> IResult<&[u8], Bencode<'_>> {
    let (leftover, _) = tag("l").parse(i)?;

    let (leftover, (eles, _)) = many_till(parse_type, tag("e")).parse(leftover)?;

    Ok((leftover, Bencode::List(eles)))
}

fn parse_pair(i: &[u8]) -> IResult<&[u8], (ByteString<'_>, Bencode<'_>)> {
    let (leftover, k) = inner_parse_bytestring(i)?;
    let (leftover, v) = parse_type(leftover)?;
    Ok((leftover, (k, v)))
}

fn parse_dictionary(i: &[u8]) -> IResult<&[u8], Bencode<'_>> {
    let (leftover, _) = tag("d").parse(i)?;

    let (leftover, (eles, _)) = many_till(parse_pair, tag("e")).parse(leftover)?;

    let eles = eles
        .into_iter()
        .collect::<BTreeMap<ByteString, Bencode>>();

    Ok((leftover, Bencode::Dictionary(eles)))
}

fn parse_type(i: &[u8]) -> IResult<&[u8], Bencode<'_>> {
    let (leftover, res) = alt((
        parse_int,  // int
        parse_list, // list
        parse_dictionary,
        parse_bytestring, // dictionary
    ))
    .parse(i)?;
    Ok((leftover, res))
}

pub fn parse_bencode(i: &[u8]) -> Result<Bencode<'_>, Error> {
    match parse_type(i) {
        Ok((_, bc)) => Ok(bc),
        Err(err) => Err(Error::bencode_parse(&err.to_string())),
    }
}

#[cfg(test)]
mod test {
    use std::fs::read;

    use super::*;

    fn do_int_tests(i: &str, v: i64) {
        let (leftover, zero) = parse_int(i.as_bytes()).unwrap();
        assert_eq!(zero, Bencode::Int(v));
        assert_eq!(leftover.len(), 0)
    }

    fn do_bytestring_test(i: &str, val: &str) {
        let (leftover, v) = parse_bytestring(i.as_bytes()).unwrap();
        assert_eq!(v, Bencode::ByteString(ByteString { elements: val.as_bytes() }));
        assert_eq!(leftover.len(), 0);
    }

    fn do_list_test(i: &str, vals: Vec<Bencode>) {
        let (leftover, v) = parse_list(i.as_bytes()).unwrap();
        if let Bencode::List(extracted) = v {
            assert_eq!(vals, extracted)
        } else {
            panic!("Wrong type");
        }
        assert_eq!(leftover.len(), 0);
    }

    fn do_dict_test(i: &str, vals: Vec<(ByteString, Bencode)>) {
        let (leftover, v) = parse_dictionary(i.as_bytes()).unwrap();
        if let Bencode::Dictionary(extracted) = v {
            assert_eq!(vals.len(), extracted.len());
            for (k, v) in vals.iter() {
                assert_eq!(extracted.get(k).unwrap(), v);
            }
        } else {
            panic!("Wrong type");
        }
        assert_eq!(leftover.len(), 0);
    }

    #[test]
    fn test_parse_int_0() {
        do_int_tests("i0e", 0);
    }

    #[test]
    fn test_parse_int_42() {
        do_int_tests("i42e", 42);
    }

    #[test]
    fn test_parse_int_neg_42() {
        do_int_tests("i-42e", -42);
    }

    #[test]
    fn test_bs_empty() {
        do_bytestring_test("0:", "");
    }
    #[test]
    fn test_bs_bencode() {
        do_bytestring_test("7:bencode", "bencode");
    }

    #[test]
    fn test_list_empty() {
        do_list_test("le", vec![]);
    }

    #[test]
    fn test_list() {
        do_list_test(
            "l7:bencodei-20ee",
            vec![
                Bencode::ByteString(ByteString {
                    elements: "bencode".as_bytes(),
                }),
                Bencode::Int(-20),
            ],
        );
    }

    #[test]
    fn test_empty_dict() {
        do_dict_test("de", vec![]);
    }
    #[test]
    fn test_dict() {
        do_dict_test(
            "d7:meaningi42e4:wiki7:bencodee",
            vec![
                (
                    ByteString {
                        elements: "meaning".as_bytes(),
                    },
                    Bencode::Int(42),
                ),
                (
                    ByteString { elements: "wiki".as_bytes() },
                    Bencode::ByteString(ByteString {
                        elements: "bencode".as_bytes(),
                    }),
                ),
            ],
        );
    }

    #[test]
    fn round_trip() {
        let torrent = read("./test_data/ubuntu-25.10-desktop-amd64.iso.torrent").unwrap();
        let bc = parse_bencode(&torrent).unwrap();
        let encoded = Bencode::encode(&bc);
        assert_eq!(torrent, encoded);
    }

    #[test]
    fn test_announce() {
        let annouce = "d8:completei287e10:incompletei13e8:intervali1800e5:peersld2:ip38:2001:67c:233c:51c2:f8d9:c4ff:fec8:a3567:peer id20:-lt0D80-�S�h_�5o�:�4:porti6938eed2:ip22:2001:41d0:1004:20b5::17:peer id20:-TR3000-fvsjn0t8mw094:porti51413eed2:ip14:185.125.190.597:peer id20:T03I--01rmakEtu9.X6.4:porti6884eed2:ip37:2001:8b0:110e:b164:3e24:748:c967:c7657:peer id20:-TR410B-xfwgoywf32914:porti51413eeee";
        let bc = parse_bencode(&annouce.as_bytes()).unwrap();

    }
}
