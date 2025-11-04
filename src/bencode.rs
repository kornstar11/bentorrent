use nom::{
  IResult, Parser, branch::alt, bytes::complete::{tag, take, take_until}, combinator::map_res, multi::{many_till, many0}, sequence::{Tuple, delimited, terminated}
};

use nom::character::complete::{i32, u32};

// If the parser was successful, then it will return a tuple. 
// The first field of the tuple will contain everything the parser did not process. 
// The second will contain everything the parser processed.
#[derive(Eq, PartialEq, Debug)]
pub struct ByteString<'a>{
    elements: &'a str,
}

#[derive(Eq, PartialEq, Debug)]
pub enum Bencode<'a> {
    Int(i32),
    ByteString(ByteString<'a>),
    List(Vec<Bencode<'a>>),
    Dictionary(Vec<(ByteString<'a>, Bencode<'a>)>)
}

// https://en.wikipedia.org/wiki/Bencode



fn parse_int(i: &str) -> IResult<&str, Bencode> {
    let (leftover, number) = delimited(
        tag("i"),
        i32,
        tag("e")
    ).parse(i)?;

    Ok((leftover, Bencode::Int(number)))
}

fn inner_parse_bytestring(i: &str) -> IResult<&str, ByteString> {
    let (leftover, len) = u32.parse(i)?;
    let (leftover, bs) = take(len).parse(leftover)?;
    Ok((leftover, ByteString { elements: bs }))
}

fn parse_bytestring(i: &str) -> IResult<&str, Bencode> {
    let (leftover, bs) = inner_parse_bytestring(i)?;
    Ok((leftover, Bencode::ByteString(bs)))
}

fn parse_list(i: &str) -> IResult<&str, Bencode> {
    let (mut leftover, len) = u32.parse(i)?;
    let mut acc = vec![];
    loop {
        let (leftover, bc) = parse_type(i)?;
        acc.push(bc);
    }
    Ok((leftover, Bencode::List(acc)))
}

fn parse_pair(i: &str) -> IResult<&str, (ByteString, Bencode)> {
    let (leftover, k) = inner_parse_bytestring(i)?;
    let (leftover, v) = parse_type(i)?;
    Ok((leftover, (k, v)))
}

fn parse_dictionary(i: &str) -> IResult<&str, Bencode> {
    let (leftover, _) = tag("d").parse(i)?;

    let (leftover, (eles, _)) = many_till(parse_pair, tag("d"))
        .parse(i)?;

    Ok((leftover, Bencode::Dictionary(eles)))
}


fn parse_type(i: &str) -> IResult<&str, Bencode> {
    let (leftover, res) = alt((
        parse_int, // int
        parse_list, // list 
        parse_dictionary,
        parse_bytestring // dictionary
    )).parse(i)?;
    Ok((leftover, res))
}

#[cfg(test)]
mod test {
    use super::*;

    fn do_int_tests(i: &str, v: i32) {
        let (leftover, zero) = parse_int(i).unwrap();
        assert_eq!(zero, Bencode::Int(v));
        assert_eq!(leftover.len(), 0)

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
}

// pub fn parse_bencode<'a>(input: &'a str) -> Bencode<'a> {
//     todo!()
// }