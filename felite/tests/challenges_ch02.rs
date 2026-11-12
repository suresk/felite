//! Tests for the Chapter 2 challenges: `cargo test --test challenges_ch02`.
use felite::{Value, decode, encode, put_varint, varint};

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

/// A small random number generator (xorshift), so that the test is the same every
/// run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// @challenge 2
use felite::column;

// @challenge 3
use felite::{decode_compact, encode_compact};

// @challenge 4
use felite::varint_strict;

// @challenge 6
use felite::{key_f64, key_i64};

// @challenge 1
#[test]
fn predict_record_sizes() {
    assert_eq!(encode(&[Value::Integer(63)]).len(), 3);
    assert_eq!(encode(&[Value::Integer(64)]).len(), 4);
    assert_eq!(encode(&[Value::Integer(-64)]).len(), 3);
    assert_eq!(encode(&vec![Value::Null; 10]).len(), 11);
    assert_eq!(encode(&[text(&"x".repeat(124))]).len(), 126);
    assert_eq!(encode(&[text(&"x".repeat(125))]).len(), 128);
    assert_eq!(encode(&[Value::Real(0.0), Value::Real(1.0)]).len(), 19);
    // The largest integers that fit in one, two and three body bytes.
    for (bytes, max) in [(1, 63), (2, 8_191), (3, 1_048_575)] {
        assert_eq!(encode(&[Value::Integer(max)]).len(), 2 + bytes);
        assert_eq!(encode(&[Value::Integer(max + 1)]).len(), 3 + bytes);
        assert_eq!(encode(&[Value::Integer(-max - 1)]).len(), 2 + bytes);
    }
}

// @challenge 2
#[test]
fn one_column_without_the_rest() {
    let row = [
        Value::Integer(1432),
        text("DL"),
        text("SLC"),
        Value::Integer(-7),
        Value::Null,
    ];
    let bytes = encode(&row);
    for (k, value) in row.iter().enumerate() {
        assert_eq!(column(&bytes, k).unwrap().as_ref(), Some(value));
    }
    assert_eq!(column(&bytes, 5).unwrap(), None);
    // Damage in a later column does not stop an earlier one from being read.
    let mut bad = encode(&[Value::Integer(7), text("héllo")]);
    let last = bad.len() - 2;
    bad[last] = 0xff;
    assert!(decode(&bad).is_err());
    assert_eq!(column(&bad, 0).unwrap(), Some(Value::Integer(7)));
    assert!(column(&bad, 1).is_err());
    assert!(column(&bytes[..8], 3).is_err()); // cut short
}

// @challenge 3
#[test]
fn zero_and_one_for_free() {
    let flags: Vec<Value> = (0..20).map(|i| Value::Integer(i % 2)).collect();
    assert_eq!(encode(&flags).len(), 41);
    assert_eq!(encode_compact(&flags).len(), 21);
    let row = [
        Value::Integer(0),
        Value::Integer(1),
        Value::Integer(2),
        text("Ada"),
        Value::Null,
    ];
    let bytes = encode_compact(&row);
    assert_eq!(bytes, [5, 3, 4, 1, 8, 0, 4, b'A', b'd', b'a']);
    assert_eq!(decode_compact(&bytes).unwrap(), row);
    assert_eq!(decode_compact(&encode_compact(&flags)).unwrap(), flags);
    assert!(decode_compact(&bytes[..9]).is_err());
}

// @challenge 4
#[test]
fn padded_varints_are_refused() {
    assert_eq!(varint(&[0x80, 0x01]).unwrap(), (1, 2)); // 1, written in two bytes
    assert!(varint_strict(&[0x80, 0x01]).is_err());
    assert_eq!(varint_strict(&[0x01]).unwrap(), (1, 1));
    assert_eq!(varint_strict(&[0x81, 0x00]).unwrap(), (128, 2));
    // Whatever put_varint writes, the strict reader accepts.
    for shift in 0..64 {
        let mut out = Vec::new();
        put_varint(&mut out, 1 << shift);
        assert_eq!(varint_strict(&out).unwrap(), (1 << shift, out.len()));
    }
}

// @challenge 5
#[test]
fn decode_never_panics() {
    let mut state = 0x2545_f491_4f6c_dd1d;
    let mut decoded = 0;
    for _ in 0..20_000 {
        let len = (next(&mut state) % 24) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| next(&mut state) as u8).collect();
        if let Ok(values) = decode(&bytes) {
            decoded += 1;
            // What decodes can be encoded, and that decodes to the same again.
            let again = encode(&values);
            assert_eq!(encode(&decode(&again).unwrap()), again);
        }
    }
    assert_eq!(decoded, 4); // random bytes are almost never a record

    // Every single-byte change to a good record.
    let good =
        encode(&[Value::Integer(1), text("Ada"), Value::Real(36.5), Value::Null]);
    let mut survived = 0;
    for at in 0..good.len() {
        for byte in 0..=255 {
            let mut bytes = good.clone();
            bytes[at] = byte;
            if decode(&bytes).is_ok() {
                survived += 1;
            }
        }
    }
    assert_eq!(survived, 2568); // of 17 × 256 = 4,352
}

// @challenge 6
#[test]
fn keys_sort_as_bytes() {
    let mut ints = [0, -1, 1, 255, 256, -256, i64::MIN, i64::MAX, 1 << 40];
    let mut by_key = ints;
    ints.sort();
    by_key.sort_by_key(|&v| key_i64(v));
    assert_eq!(ints, by_key);
    assert_eq!(key_i64(0), [0x80, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(key_i64(-1), [0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);

    let mut reals =
        [0.0, -0.5, 0.5, 1e300, -1e300, f64::INFINITY, f64::NEG_INFINITY, 1e-300];
    let mut by_key = reals;
    reals.sort_by(|a, b| a.total_cmp(b));
    by_key.sort_by_key(|&x| key_f64(x));
    assert_eq!(reals, by_key);
}
