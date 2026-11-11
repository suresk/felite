//! Chapter 2 challenges: one column of a record, the zero-and-one trick, strict
//! varints, and keys that sort as bytes.

use crate::error::{Error, Result};
use crate::record::{Value, decode_value, read_varint, take};
use crate::varint::{put_varint, varint, zigzag};

// @listing column
/// Value `k` of a record, without decoding the others. `None` if the record has
/// `k` values or fewer.
pub fn column(bytes: &[u8], k: usize) -> Result<Option<Value>> {
    let mut codes = bytes;
    let count = read_varint(&mut codes)?;
    if k as u64 >= count {
        return Ok(None);
    }
    let mut bodies = codes;
    for _ in 0..count {
        read_varint(&mut bodies)?; // step over every type code
    }
    for _ in 0..k {
        match read_varint(&mut codes)? {
            0 => {}
            1 => drop(read_varint(&mut bodies)?),
            2 => drop(take(&mut bodies, 8)?),
            n => drop(take(&mut bodies, n - 3)?),
        }
    }
    let code = read_varint(&mut codes)?;
    decode_value(code, &mut bodies).map(Some)
}
// @end

// @listing encode-compact
/// As `encode`, but the integers 0 and 1 are type codes of their own and have no
/// body. Text starts at code 5.
pub fn encode_compact(values: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, values.len() as u64);
    for value in values {
        let code = match value {
            Value::Null => 0,
            Value::Integer(0) => 3,
            Value::Integer(1) => 4,
            Value::Integer(_) => 1,
            Value::Real(_) => 2,
            Value::Text(s) => 5 + s.len() as u64,
        };
        put_varint(&mut out, code);
    }
    for value in values {
        match value {
            Value::Null | Value::Integer(0) | Value::Integer(1) => {}
            Value::Integer(i) => put_varint(&mut out, zigzag(*i)),
            Value::Real(x) => out.extend_from_slice(&x.to_bits().to_be_bytes()),
            Value::Text(s) => out.extend_from_slice(s.as_bytes()),
        }
    }
    out
}
// @end

// @listing decode-compact
pub fn decode_compact(bytes: &[u8]) -> Result<Vec<Value>> {
    let mut rest = bytes;
    let count = read_varint(&mut rest)?;
    if count > rest.len() as u64 {
        return Err(Error::Corrupt(format!("{count} values claimed")));
    }
    let mut codes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        codes.push(read_varint(&mut rest)?);
    }
    let mut values = Vec::with_capacity(codes.len());
    for code in codes {
        values.push(match code {
            3 => Value::Integer(0),
            4 => Value::Integer(1),
            0..=2 => decode_value(code, &mut rest)?,
            n => decode_value(n - 2, &mut rest)?, // text: our 5 is the old 3
        });
    }
    if !rest.is_empty() {
        return Err(Error::Corrupt("bytes after the last value".into()));
    }
    Ok(values)
}
// @end

// @listing varint-strict
/// As `varint`, but refuses a value written in more bytes than it needs.
pub fn varint_strict(bytes: &[u8]) -> Result<(u64, usize)> {
    if bytes.first() == Some(&0x80) {
        return Err(Error::Corrupt("a varint with a leading zero group".into()));
    }
    varint(bytes)
}
// @end

// @listing keys
/// Eight bytes that sort, as bytes, in the numeric order of `value`.
pub fn key_i64(value: i64) -> [u8; 8] {
    (value as u64 ^ (1 << 63)).to_be_bytes()
}

/// The same for a float. NaN has no place in the order and is not handled.
pub fn key_f64(value: f64) -> [u8; 8] {
    let bits = value.to_bits();
    let flipped = if bits >> 63 == 0 { bits ^ (1 << 63) } else { !bits };
    flipped.to_be_bytes()
}
// @end
