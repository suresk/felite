//! Values as index keys: bytes that sort the way the values do.

use crate::record::Value;

// @listing key-tags
/// The first byte of each kind of value in a key. Their order is the order of
/// the kinds: NULL sorts before everything else.
const NULL: u8 = 0;
const INTEGER: u8 = 1;
const REAL: u8 = 2;
const TEXT: u8 = 3;
// @end

// @listing encode-key
/// Writes values as bytes such that comparing two results byte by byte gives the
/// same answer as comparing the values, one after another, with `order`.
pub fn encode_key(values: &[Value]) -> Vec<u8> {
    let mut key = Vec::new();
    for value in values {
        match value {
            Value::Null => key.push(NULL),
            Value::Integer(i) => {
                key.push(INTEGER);
                // Big-endian, with the sign bit turned over so that negative
                // numbers come first.
                key.extend((*i as u64 ^ (1 << 63)).to_be_bytes());
            }
            Value::Real(x) => {
                key.push(REAL);
                key.extend(real_bits(*x).to_be_bytes());
            }
            Value::Text(text) => {
                key.push(TEXT);
                for byte in text.bytes() {
                    key.push(byte);
                    // A zero byte inside the text is followed by 0xFF, to tell
                    // it from the two zeros that end the text.
                    if byte == 0 {
                        key.push(0xFF);
                    }
                }
                key.extend([0, 0]);
            }
        }
    }
    key
}
// @end

// @listing real-bits
/// The bits of a real, rearranged so that they sort as an unsigned integer.
fn real_bits(x: f64) -> u64 {
    let bits = x.to_bits();
    if bits >> 63 == 1 {
        // A negative number: the larger its bits, the smaller it is. Turning
        // every bit over reverses that and puts it below the positive numbers.
        !bits
    } else {
        bits | (1 << 63)
    }
}
// @end

// @listing after
/// A key that is greater than every key that begins with `prefix`, and less
/// than or equal to every greater key that doesn't.
pub fn after(prefix: &[u8]) -> Vec<u8> {
    let mut key = prefix.to_vec();
    key.extend([0xFF; 9]);
    key
}
// @end
