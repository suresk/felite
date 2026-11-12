//! Tests for the Chapter 23 challenges: `cargo test --test challenges_ch23`.
use felite::{
    Database, HashJoin, MemFile, Operator, Output, Row, Scan, Scope, Value,
};
use felite::{IndexLoop, decode_key, drop_index, encode_key_desc, index_only};
use felite::{after, bind, encode_key, order, parse_expr};
use std::cmp::Ordering;

fn fixture() -> (Database, MemFile) {
    let file = MemFile::new();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    for statement in include_str!("../fixtures/flights.sql").lines() {
        db.execute(statement).unwrap();
    }
    (db, file)
}

fn query(db: &mut Database, sql: &str) -> Vec<String> {
    let Output::Rows(result) = db.execute(sql).unwrap() else {
        panic!("not a query")
    };
    let line =
        |row: &Row| row.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
    result.rows.iter().map(line).collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

#[test]
fn predict_keys() {
    assert_eq!(hex(&encode_key(&[Value::Integer(0)])), "01 80 00 00 00 00 00 00 00");
    assert_eq!(hex(&encode_key(&[Value::Integer(-2)])), "01 7f ff ff ff ff ff ff fe");
    assert_eq!(hex(&encode_key(&[text("WN"), Value::Null])), "03 57 4e 00 00 00");
    assert_eq!(hex(&encode_key(&[Value::Real(0.0)])), "02 80 00 00 00 00 00 00 00");
    assert_eq!(hex(&encode_key(&[Value::Real(-0.0)])), "02 7f ff ff ff ff ff ff ff");
    let key = |s: &str| encode_key(&[text(s)]);
    assert!(key("DEN") < key("DENVER") && key("DENVER") < key("DFW"));
    assert!(key("Z") < key("a") && key("") < key("\0"));
}

#[test]
fn values_back_from_a_key() {
    let rows = [
        vec![Value::Null, Value::Integer(-7), Value::Real(-2.5), text("a\0b")],
        vec![text(""), text("ATL"), Value::Integer(i64::MAX), Value::Real(0.0)],
        vec![Value::Real(f64::MAX), Value::Real(-0.0), Value::Integer(i64::MIN)],
        vec![],
    ];
    for row in rows {
        let decoded = decode_key(&encode_key(&row)).unwrap();
        assert_eq!(decoded.len(), row.len());
        for (a, b) in decoded.iter().zip(&row) {
            assert_eq!(a.to_string(), b.to_string());
        }
    }
    assert!(decode_key(&[3, 65]).is_err());
    assert!(decode_key(&[1, 0, 0]).is_err());
    assert!(decode_key(&[9]).is_err());
}

#[test]
fn answering_from_the_index() {
    let (_, file) = fixture();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    db.execute("CREATE INDEX by_origin_dest ON flights (origin, dest)").unwrap();
    let mut db = Database::open(Box::new(file)).unwrap();
    let index = db.table("flights").unwrap().index("by_origin_dest").unwrap().clone();
    let from = encode_key(&[text("ATL")]);
    let before = db.pager().stats().reads;
    let rows = index_only(&mut db, &index, &from, &after(&from)).unwrap();
    let reads = db.pager().stats().reads - before;
    let shown: Vec<String> = rows
        .iter()
        .map(|row| row.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" "))
        .collect();
    assert_eq!(
        shown,
        query(
            &mut db,
            "SELECT origin, dest, id FROM flights WHERE origin = 'ATL'
             ORDER BY dest, id"
        )
    );
    assert_eq!(rows.len(), 9);
    // The root of the index and one leaf. The table was never touched.
    assert_eq!(reads, 2);
}

#[test]
fn dropping_an_index() {
    let (mut db, file) = fixture();
    db.execute("CREATE INDEX flights_origin ON flights (origin)").unwrap();
    db.execute("CREATE UNIQUE INDEX carriers_code ON carriers (code)").unwrap();
    assert!(drop_index(&mut db, "nothing").is_err());
    drop_index(&mut db, "Flights_Origin").unwrap();
    assert!(db.table("flights").unwrap().indexes.is_empty());
    assert_eq!(db.pager().free_count().unwrap(), 1);
    db.check().unwrap();
    // The rule it enforced is gone with it.
    drop_index(&mut db, "carriers_code").unwrap();
    db.execute("INSERT INTO carriers VALUES ('DL', 'Dolphin Air')").unwrap();
    db.execute(
        "INSERT INTO flights (day, carrier, number, origin, dest, departs, cancelled,
         miles) VALUES ('2024-02-01', 'DL', 1, 'ATL', 'SLC', 900, 0, 1590.0)",
    )
    .unwrap();
    let mut db = Database::open(Box::new(file)).unwrap();
    assert!(db.table("flights").unwrap().indexes.is_empty());
    assert!(db.table("carriers").unwrap().indexes.is_empty());
    db.check().unwrap();
}

#[test]
fn keys_that_sort_backwards() {
    let values = [
        Value::Null,
        Value::Integer(-5),
        Value::Integer(0),
        Value::Integer(9),
        text(""),
        text("A"),
        text("AB"),
        text("B"),
    ];
    let same_kind = |a: &Value, b: &Value| {
        matches!(
            (a, b),
            (Value::Null, _)
                | (_, Value::Null)
                | (Value::Integer(_), Value::Integer(_))
                | (Value::Text(_), Value::Text(_))
        )
    };
    for a in &values {
        for b in values.iter().filter(|b| same_kind(a, b)) {
            for c in [Value::Integer(1), Value::Integer(2)] {
                for d in [Value::Integer(1), Value::Integer(2)] {
                    let left =
                        encode_key_desc(&[a.clone(), c.clone()], &[true, false]);
                    let right =
                        encode_key_desc(&[b.clone(), d.clone()], &[true, false]);
                    let expected = order(b, a).then(order(&c, &d));
                    assert_eq!(left.cmp(&right), expected, "{a} {c} / {b} {d}");
                }
            }
        }
    }
    let plain = encode_key_desc(&[text("A"), Value::Integer(3)], &[false, false]);
    assert_eq!(plain, encode_key(&[text("A"), Value::Integer(3)]));
    assert_eq!(order(&Value::Integer(1), &Value::Integer(1)), Ordering::Equal);
}

#[test]
fn a_join_through_an_index() {
    let (mut db, _) = fixture();
    db.execute("CREATE UNIQUE INDEX airports_code ON airports (code)").unwrap();
    db.execute("CREATE INDEX flights_tail ON flights (tail)").unwrap();
    let drain = |db: &mut Database, operator: &mut dyn Operator| {
        let mut rows = Vec::new();
        while let Some(row) = operator.next(db.pager()).unwrap() {
            rows.push(row);
        }
        rows
    };
    let key = |db: &Database, table: &str, column: &str| {
        let scope = Scope::of(db.table(table).unwrap());
        bind(&parse_expr(column).unwrap(), &scope).unwrap()
    };

    // Flights to airports on origin = code: one index entry for each flight.
    let (flights, airports) =
        (db.table("flights").unwrap().clone(), db.table("airports").unwrap().clone());
    let index = airports.index("airports_code").unwrap().clone();
    let outer = Box::new(Scan::new(&flights, db.pager()).unwrap());
    let mut join =
        IndexLoop::new(outer, &airports, &index, key(&db, "flights", "origin"));
    let rows = drain(&mut db, &mut join);
    assert_eq!((rows.len(), join.entries), (200, 200));

    let outer = Box::new(Scan::new(&flights, db.pager()).unwrap());
    let inner = Box::new(Scan::new(&airports, db.pager()).unwrap());
    let keys = (key(&db, "flights", "origin"), key(&db, "airports", "code"));
    let mut hash = HashJoin::new(outer, inner, keys, 3, false);
    assert_eq!(rows, drain(&mut db, &mut hash));

    // Flights to flights on tail = tail. NULL tails match nothing.
    let index = flights.index("flights_tail").unwrap().clone();
    let outer = Box::new(Scan::new(&flights, db.pager()).unwrap());
    let mut join =
        IndexLoop::new(outer, &flights, &index, key(&db, "flights", "tail"));
    let rows = drain(&mut db, &mut join);
    assert_eq!(rows.len(), 208);
    let outer = Box::new(Scan::new(&flights, db.pager()).unwrap());
    let inner = Box::new(Scan::new(&flights, db.pager()).unwrap());
    let keys = (key(&db, "flights", "tail"), key(&db, "flights", "tail"));
    let mut hash = HashJoin::new(outer, inner, keys, 13, false);
    assert_eq!(rows, drain(&mut db, &mut hash));
}
