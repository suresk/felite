//! The examples in Chapter 2: `cargo test --test book_ch02`.
use felite::{Error, Value, decode, encode, hex_dump};
use felite::{put_varint, unzigzag, varint, zigzag};

#[test]
fn small_numbers_take_one_byte() {
    // @listing varint-test
    let bytes_of = |value: u64| {
        let mut out = Vec::new();
        put_varint(&mut out, value);
        out
    };
    assert_eq!(bytes_of(0), [0x00]);
    assert_eq!(bytes_of(127), [0x7f]);
    assert_eq!(bytes_of(128), [0x81, 0x00]);
    assert_eq!(bytes_of(300), [0x82, 0x2c]);
    assert_eq!(bytes_of(16_383), [0xff, 0x7f]);
    assert_eq!(bytes_of(16_384), [0x81, 0x80, 0x00]);
    assert_eq!(bytes_of(u64::MAX).len(), 10);
    // @end
}

#[test]
fn varints_read_back() {
    // @listing varint-read-test
    // The value, and how many bytes it used. Bytes after the varint are ignored.
    assert_eq!(varint(&[0x82, 0x2c, 0xff]).unwrap(), (300, 2));

    for shift in 0..64 {
        for value in [(1u64 << shift) - 1, 1 << shift, u64::MAX >> shift] {
            let mut out = Vec::new();
            put_varint(&mut out, value);
            assert_eq!(varint(&out).unwrap(), (value, out.len()));
        }
    }
    // @end
}

#[test]
fn bad_varints_are_errors() {
    // @listing varint-bad-test
    let message = |bytes: &[u8]| varint(bytes).unwrap_err().to_string();
    assert_eq!(
        message(&[]),
        "corrupt database: a varint runs past the end of its bytes"
    );
    assert_eq!(
        message(&[0x82]), // the top bit promises another byte
        "corrupt database: a varint runs past the end of its bytes"
    );
    assert_eq!(
        message(&[0xff; 11]), // eleven bytes would be 77 bits
        "corrupt database: a varint does not fit in 64 bits"
    );
    // @end
}

#[test]
fn zigzag_folds_the_number_line() {
    // @listing zigzag-test
    assert_eq!([0, -1, 1, -2, 2].map(zigzag), [0, 1, 2, 3, 4]);
    assert_eq!(zigzag(63), 126); // the largest that fits one varint byte
    assert_eq!(zigzag(-64), 127); // and the smallest
    assert_eq!(zigzag(i64::MAX), u64::MAX - 1);
    assert_eq!(zigzag(i64::MIN), u64::MAX);

    for shift in 0..63 {
        for value in
            [1i64 << shift, (1 << shift) - 1, -(1 << shift), i64::MIN >> shift]
        {
            assert_eq!(unzigzag(zigzag(value)), value);
        }
    }
    // @end
}

#[test]
fn the_bytes_of_a_row() {
    // @listing record-bytes
    let row = [
        Value::Integer(1),
        Value::Text("Ada".to_string()),
        Value::Real(36.5),
        Value::Null,
    ];
    let bytes = encode(&row);
    assert_eq!(bytes.len(), 17);
    assert_eq!(
        hex_dump(&bytes),
        "0000  04 01 06 02 00 02 41 64 61 40 42 40 00 00 00 00  |......Ada@B@....|\n\
         0010  00                                               |.|\n"
    );
    assert_eq!(decode(&bytes).unwrap(), row);
    // @end
}

#[test]
fn a_flight_in_fourteen_bytes() {
    // @listing flight-row
    // Flight number, carrier, origin, minutes late (early here), a missing value.
    let row = [
        Value::Integer(1432),
        Value::Text("DL".to_string()),
        Value::Text("SLC".to_string()),
        Value::Integer(-7),
        Value::Null,
    ];
    let bytes = encode(&row);
    assert_eq!(bytes[..6], [5, 1, 5, 6, 1, 0]); // the count, then five type codes
    assert_eq!(bytes[6..], [0x96, 0x30, b'D', b'L', b'S', b'L', b'C', 13]);
    assert_eq!(bytes.len(), 14);
    // @end
}

#[test]
fn rows_round_trip() {
    // @listing round-trip
    let rows = [
        vec![],
        vec![Value::Null],
        vec![Value::Text(String::new())],
        vec![Value::Integer(i64::MIN), Value::Integer(i64::MAX)],
        vec![Value::Real(-0.0), Value::Real(f64::INFINITY), Value::Real(1e-300)],
        vec![Value::Text("Zürich → 東京".to_string()), Value::Text("x".repeat(1000))],
    ];
    for row in rows {
        assert_eq!(decode(&encode(&row)).unwrap(), row);
    }
    assert_eq!(encode(&[]), [0]); // a row of no values is one byte
    assert_eq!(encode(&[Value::Text("x".repeat(1000))]).len(), 1 + 2 + 1000);
    // @end
}

#[test]
fn damaged_records_are_errors() {
    // @listing damaged
    let good = encode(&[
        Value::Integer(1),
        Value::Text("Ada".to_string()),
        Value::Real(36.5),
    ]);
    let message = |bytes: &[u8]| decode(bytes).unwrap_err().to_string();

    // Cut short anywhere, the record is refused.
    for len in 0..good.len() {
        assert!(matches!(decode(&good[..len]), Err(Error::Corrupt(_))));
    }
    // One byte too many.
    let mut long = good.clone();
    long.push(0);
    assert_eq!(message(&long), "corrupt database: 1 bytes after the last value");
    // A count far larger than the bytes that follow.
    assert_eq!(
        message(&[0xff, 0xff, 0x7f, 0, 0]),
        "corrupt database: 2097151 values claimed in 2 bytes"
    );
    // A text value of two bytes that are not UTF-8.
    assert_eq!(
        message(&[1, 5, 0xff, 0xfe]),
        "corrupt database: text that is not UTF-8"
    );
    // A text value that claims more bytes than there are.
    assert_eq!(
        message(&[1, 0x87, 0x6b, b'h', b'i']), // code 1003: 1,000 bytes of text
        "corrupt database: a value needs 1000 bytes and 2 remain"
    );
    // @end
}
