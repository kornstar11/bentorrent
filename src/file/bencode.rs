use std::{collections::BTreeMap, fmt::Debug};

use nom::{
    IResult, Parser,
    branch::alt,
    bytes::complete::{tag, take},
    multi::many_till,
    sequence::delimited,
};

use std::fmt;

use super::error::Error;
use nom::character::complete::{i64, u32};

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

impl<'a> Bencode<'a> {
    fn encode_int(i: i64) -> Vec<u8> {
        let bencode = format!("i{}e", i);
        return bencode.as_bytes().to_vec();
    }

    fn encode_bs(bs: &ByteString<'a>) -> Vec<u8> {
        let mut bs_string = format!("{}:", bs.elements.len()).as_bytes().to_vec();
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

pub fn map_dict_keys<'a>(dict: DictT<'a>) -> BTreeMap<String, Bencode<'a>> {
    dict.into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect::<BTreeMap<_, _>>()
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

    let eles = eles.into_iter().collect::<BTreeMap<ByteString, Bencode>>();

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
        assert_eq!(
            v,
            Bencode::ByteString(ByteString {
                elements: val.as_bytes()
            })
        );
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
                    ByteString {
                        elements: "wiki".as_bytes(),
                    },
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
        let annouce = "64383a636f6d706c65746569313038326531303a696e636f6d706c65746569353765383a696e74657276616c693138303065353a70656572736c64323a697033383a323430393a386132303a326433333a616264303a3231313a333266663a666538323a36303262373a7065657220696432303a2d5452323933302d746e356b30776d7466723737343a706f7274693635333535656564323a697033353a323030313a3437303a376138333a366637343a303a373036393a373236313a37343635373a7065657220696432303a2d5452343036302d73717770347a78726c326931343a706f72746936393739656564323a697033383a323030313a3637633a323333633a353163323a663864393a633466663a666563383a61333536373a7065657220696432303a2d6c74304438302dca53f1685f94351d6f9a3a88343a706f72746936393338656564323a697032303a323030313a6263383a313634303a3533303a3a31373a7065657220696432303a2d5452343130422d386f31723036396633766a31343a706f7274693133303030656564323a697033363a326130313a6530613a3131643a393836303a393230393a643066663a666532313a616462373a7065657220696432303a2d5452323933302d666178347376397136747378343a706f7274693136383831656564323a697033383a326130303a363032303a623332663a613430303a3231313a333266663a666562373a62333531373a7065657220696432303a2d5452343036302d31716239337564646f366971343a706f7274693531343133656564323a697033363a323030313a3862303a613739303a363034613a636136303a66663a666538363a63663134373a7065657220696432303a2d5452333030302d343677766478746574757135343a706f7274693531343133656564323a697032303a323030313a3437303a316630363a3332623a3a32373a7065657220696432303a2d5452343130422d653171376c67657934796279343a706f7274693534353535656564323a697033373a323030313a3862303a313130653a623136343a336532343a3734383a633936373a63373635373a7065657220696432303a2d5452343130422d786677676f79776633323931343a706f7274693531343133656564323a697031373a326130323a613436653a356339393a3a31373a7065657220696432303a2d5452333030302d79777937737833776636386b343a706f7274693531343135656564323a697032323a323030313a343164303a313030343a323062353a3a31373a7065657220696432303a2d5452333030302d70736767613634347a313971343a706f7274693531343133656564323a697031343a3138352e3132352e3139302e3539373a7065657220696432303a543033492d2d303172526d58764a2e664d6c6930343a706f72746936393235656564323a697032343a323630373a663734383a3230333a3132353a343a3a313132373a7065657220696432303a2d6c74304630352d4ee791fa85e19ed0e183f88e343a706f7274693231333937656564323a697032303a326130323a323437393a34343a386630303a3a31373a7065657220696432303a2d6c74304438302d2dce18912ebb76fd946f52a9343a706f72746936393238656564323a697032303a326130303a363830303a333a6634393a3a313030373a7065657220696432303a2d5452343130422d6b6471363777656379636b6b343a706f7274693531343133656564323a697031373a326130333a336234303a32633a313a3a33373a7065657220696432303a2d5452323933302d7737736b353467307064386d343a706f7274693438333839656564323a697033383a323830343a313435633a383662333a3230303a383034623a646431333a356663663a36613830373a7065657220696432303a2d5452343130422d32667a32356535306d387338343a706f7274693531343133656564323a697033383a323630373a666561383a666466303a383235623a396538643a663437343a346432343a336638373a7065657220696432303a2d5452323934302d3037637334356b6163377362343a706f7274693531343133656564323a697033373a326130313a6530613a6534373a373233303a613461383a336565353a656633373a39333139373a7065657220696432303a2d5452343036302d746f6a686961686770777467343a706f7274693531343133656564323a697032333a326130313a376530303a653030313a643430333a3a3130373a7065657220696432303a2d5452343036302d357971676b6d757a336a6276343a706f7274693632313133656564323a697032303a326130313a3466383a6331373a313136643a3a32373a7065657220696432303a2d6c74304438302d9c65ff905eeb93a33b0a368b343a706f72746936393934656564323a697033373a326130353a333538303a646430303a633130303a3166613a313561313a383162333a613665373a7065657220696432303a2d5554313832302dfd3b667f82411b545ece06c2343a706f727469353233343865656565";
        let decoded = hex::decode(annouce).unwrap();
        let bc = parse_bencode(&decoded).unwrap();
        //println!("{:#?}", bc);

        if let Bencode::Dictionary(dict) = bc {
            let dict = map_dict_keys(dict);
            assert_eq!(dict.get("complete"), Some(&Bencode::Int(1082)));
        } else {
            panic!("should be a dict");
        }
    }
}
