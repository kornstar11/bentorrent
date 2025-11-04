use nom::{
  IResult, Parser, branch::alt, bytes::complete::{tag, take, take_until}, combinator::map_res, multi::many0, sequence::{Tuple, delimited, terminated}
};

use nom::character::complete::{i32, u32};

// If the parser was successful, then it will return a tuple. 
// The first field of the tuple will contain everything the parser did not process. 
// The second will contain everything the parser processed.
pub struct ByteString<'a>{
    elements: &'a str,
}

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
        tag("d")
    ).parse(i)?;

    Ok((leftover, Bencode::Int(number)))
}

fn inner_parse_bytestring<'a>(i: &'a str) -> IResult<&str, ByteString> {
    let (leftover, len) = u32.parse(i)?;
    let (leftover, bs) = take(len).parse(leftover)?;
    Ok((leftover, ByteString { elements: bs }))
}

fn parse_bytestring<'a>(i: &'a str) -> IResult<&str, Bencode> {
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

fn parse_pair<'a>(i: &'a str) -> IResult<&str, (ByteString, Bencode)> {
    let (leftover, k) = inner_parse_bytestring(i)?;
    let (leftover, v) = parse_type(i)?;
    Ok((leftover, (k, v)))
}

fn parse_dictionary<'a>(i: &'a str) -> IResult<&str, Bencode> {
    let (leftover, eles) = delimited(
        tag("i"),
        many0(parse_pair),
        tag("e")
    ).parse(i)?;

    Ok((leftover, Bencode::Dictionary(eles)))
}


fn parse_type(i: &str) -> IResult<&str, Bencode> {
    todo!()
    // let t = alt((
    //     tag("i"), // int
    //     tag("l"), // list 
    //     tag("d") // dictionary
    // )).parse(input);

}

pub fn parse_bencode<'a>(input: &'a str) -> Bencode<'a> {
    todo!()
}