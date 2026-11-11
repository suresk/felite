//! Integers that take as many bytes as their size needs.

use crate::error::{Error, Result};

// @listing put-varint
/// Appends `value` to `out`, seven bits to a byte, the highest bits first. Every
/// byte but the last has its top bit set.
pub fn put_varint(out: &mut Vec<u8>, value: u64) {
    let mut groups = 1;
    while groups < 10 && value >> (7 * groups) != 0 {
        groups += 1;
    }
    for i in (0..groups).rev() {
        let group = (value >> (7 * i)) as u8 & 0x7f;
        out.push(if i > 0 { group | 0x80 } else { group });
    }
}
// @end

// @listing varint
/// Reads a varint from the front of `bytes`. Returns the value and the number of
/// bytes it took.
pub fn varint(bytes: &[u8]) -> Result<(u64, usize)> {
    let mut value: u64 = 0;
    for (i, &byte) in bytes.iter().enumerate() {
        if i == 10 || value >> 57 != 0 {
            return Err(Error::Corrupt("a varint does not fit in 64 bits".into()));
        }
        value = (value << 7) | (byte & 0x7f) as u64;
        if byte & 0x80 == 0 {
            return Ok((value, i + 1));
        }
    }
    Err(Error::Corrupt("a varint runs past the end of its bytes".into()))
}
// @end

// @listing zigzag
/// Maps 0, -1, 1, -2, 2, … to 0, 1, 2, 3, 4, …
pub fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}

pub fn unzigzag(value: u64) -> i64 {
    (value >> 1) as i64 ^ -((value & 1) as i64)
}
// @end
