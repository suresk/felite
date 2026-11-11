//! A row of values as a string of bytes.

use crate::error::{Error, Result};
use crate::varint::{put_varint, unzigzag, varint, zigzag};

// @listing value
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
}
// @end

// @listing encode
/// One byte string for a row: a count, a type code for each value, then the values.
pub fn encode(values: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, values.len() as u64);
    for value in values {
        let code = match value {
            Value::Null => 0,
            Value::Integer(_) => 1,
            Value::Real(_) => 2,
            Value::Text(s) => 3 + s.len() as u64,
        };
        put_varint(&mut out, code);
    }
    for value in values {
        match value {
            Value::Null => {}
            Value::Integer(i) => put_varint(&mut out, zigzag(*i)),
            Value::Real(x) => out.extend_from_slice(&x.to_bits().to_be_bytes()),
            Value::Text(s) => out.extend_from_slice(s.as_bytes()),
        }
    }
    out
}
// @end

// @listing cursor
/// Reads a varint from the front of `rest` and moves `rest` past it.
pub(crate) fn read_varint(rest: &mut &[u8]) -> Result<u64> {
    let (value, used) = varint(rest)?;
    *rest = &rest[used..];
    Ok(value)
}

/// Takes `n` bytes from the front of `rest`.
pub(crate) fn take<'a>(rest: &mut &'a [u8], n: u64) -> Result<&'a [u8]> {
    if n > rest.len() as u64 {
        let have = rest.len();
        return Err(Error::Corrupt(format!(
            "a value needs {n} bytes and {have} remain"
        )));
    }
    let (head, tail) = rest.split_at(n as usize);
    *rest = tail;
    Ok(head)
}
// @end

// @listing decode
pub fn decode(bytes: &[u8]) -> Result<Vec<Value>> {
    let mut rest = bytes;
    let count = read_varint(&mut rest)?;
    if count > rest.len() as u64 {
        let have = rest.len();
        return Err(Error::Corrupt(format!(
            "{count} values claimed in {have} bytes"
        )));
    }
    let mut codes = Vec::with_capacity(count as usize);
    for _ in 0..count {
        codes.push(read_varint(&mut rest)?);
    }
    let mut values = Vec::with_capacity(codes.len());
    for code in codes {
        values.push(decode_value(code, &mut rest)?);
    }
    if !rest.is_empty() {
        let extra = rest.len();
        return Err(Error::Corrupt(format!("{extra} bytes after the last value")));
    }
    Ok(values)
}
// @end

// @listing decode-value
pub(crate) fn decode_value(code: u64, rest: &mut &[u8]) -> Result<Value> {
    Ok(match code {
        0 => Value::Null,
        1 => Value::Integer(unzigzag(read_varint(rest)?)),
        2 => {
            let b = take(rest, 8)?;
            let bits =
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
            Value::Real(f64::from_bits(bits))
        }
        n => {
            let bytes = take(rest, n - 3)?;
            match std::str::from_utf8(bytes) {
                Ok(s) => Value::Text(s.to_string()),
                Err(_) => {
                    return Err(Error::Corrupt("text that is not UTF-8".into()));
                }
            }
        }
    })
}
// @end
