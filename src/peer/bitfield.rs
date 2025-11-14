use std::{collections::HashSet, ops::Shr};

use bytes::{BufMut, Bytes, BytesMut};

const MASK: u8 = 0b10000000;
pub struct BitFieldReader {
    bytes: Bytes,
}

impl From<Vec<u8>> for BitFieldReader {
    fn from(value: Vec<u8>) -> Self {
        BitFieldReader::new(Bytes::from(value))
    }
}

impl BitFieldReader {
    pub fn new(bytes: Bytes) -> Self {
        BitFieldReader { bytes }
    }

    pub fn get_bit(&self, bit_num: usize) -> Option<bool> {
        let byte_pos = bit_num / u8::BITS as usize;
        if let Some(byte) = self.bytes.get(byte_pos) {
            let bit_pos = (bit_num as u32) % u8::BITS;
            if let Some(mask) = MASK.checked_shr(bit_pos) {
                let v = mask & byte;
                if v != 0 {
                    return Some(true);
                } else {
                    return Some(false);
                }
            } else {
                return None;
            }
        }
        None
    }
}

pub struct BitFieldReaderIter {
    reader: BitFieldReader,
    pos: usize,
}

impl From<BitFieldReader> for BitFieldReaderIter {
    fn from(reader: BitFieldReader) -> Self {
        Self { reader, pos: 0 }
    }
}

impl Iterator for BitFieldReaderIter {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        let v = self.reader.get_bit(self.pos);
        self.pos += 1;
        v
    }
}

pub struct BitFieldWriter {
    bytes: BytesMut,
    bits_written: u32,
}

impl BitFieldWriter {
    pub fn new(bytes: BytesMut) -> Self {
        Self {
            bytes,
            bits_written: 0,
        }
    }

    pub fn into(self) -> BytesMut {
        self.bytes
    }

    pub fn put_bit(&mut self, bit: bool) {
        let bit_pos = self.bits_written % 8;
        if bit_pos == 0 {
            self.bytes.put_u8(0);
        }
        let last_byte_pos = self.bytes.len() - 1;
        let mut last_byte = self.bytes[last_byte_pos];

        if bit {
            let mask = MASK.shr(bit_pos);
            last_byte |= mask;
            self.bytes[last_byte_pos] = last_byte;
        }
        self.bits_written += 1;
    }

    pub fn put_bit_set(&mut self, bits: &HashSet<u32>, length: usize) {
        for i in 0..length as u32 {
            self.put_bit(bits.contains(&i));
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn read_bits() {
        let expected = vec![
            true, false, false, false, true, false, false, true, true, true, true, true, true,
            false, false, false,
        ];
        let byte1: u8 = 0b10001001;
        let byte2: u8 = 0b11111000;
        let bitfield = BitFieldReader::from(vec![byte1, byte2]);
        let mut result = vec![];
        for i in 0..16 {
            let v = bitfield.get_bit(i).unwrap();
            result.push(v);
        }

        assert_eq!(result, expected);
    }

    #[test]
    fn roundtrip_bits() {
        let expected = vec![
            true, false, false, false, true, false, false, true, true, true, true, true, true,
            false, false, false,
        ];
        let mut writer = BitFieldWriter::new(BytesMut::new());
        for bit in expected.iter() {
            writer.put_bit(*bit);
        }
        let bytes = writer.into();
        let bytes = bytes.freeze();
        let reader = BitFieldReader::new(bytes);
        let mut result = vec![];
        for i in 0..16 {
            let v = reader.get_bit(i).unwrap();
            result.push(v);
        }

        assert_eq!(result, expected);
    }
}
