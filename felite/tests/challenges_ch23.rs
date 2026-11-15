//! Tests for the Chapter 23 challenges: `cargo test --test challenges_ch23`.
use felite::{
    Database, HashJoin, MemFile, Operator, Output, Row, Scan, Scope, Value,
};
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

// @challenge 2
use felite::{decode_key, index_only};

// @challenge 3
use felite::drop_index;

// @challenge 4
use felite::encode_key_desc;

// @challenge 5
use felite::IndexLoop;

// @challenge 1
#[test]
fn predict_keys() {
    assert_eq!(hex(&encode_key(&[Value::Integer(0)])), "01 80 00 00 00 00 00 00 00");
    assert_eq!(hex(&encode_key(&[Value::Integer(-2)])), "01 7f ff ff ff ff ff ff fe");
    assert_eq!(hex(&encode_key(&[text("WN"), Value::Null])), "03 57 4e 00 00 00");
    assert_eq!(hex(&encode_key(&[Value::Real(0.0)])), "02 80 00 00 00 00 00 00 00");
    assert_eq!(hex(&encode_key(&[Value::Real(-0.0)])), "02 80 00 00 00 00 00 00 00");
    let key = |s: &str| encode_key(&[text(s)]);
    assert!(key("DEN") < key("DENVER") && key("DENVER") < key("DFW"));
    assert!(key("Z") < key("a") && key("") < key("\0"));
}

// @challenge 2
#[test]
fn values_back_from_a_key() {
    let rows = [
        vec![Value::Null, Value::Integer(-7), Value::Real(-2.5), text("a\0b")],
        vec![text(""), text("ATL"), Value::Integer(i64::MAX), Value::Real(0.0)],
        vec![Value::Real(f64::MAX), Value::Real(-1e-300), Value::Integer(i64::MIN)],
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

// @challenge 3
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

// @challenge 4
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

// @challenge 5
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

// @challenge 5
#[test]
fn an_index_join_across_integer_and_real() {
    // An INTEGER key looked up in an index on a REAL column, and the other way
    // round, finds what the SQL join finds.
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute("CREATE TABLE a (id INTEGER PRIMARY KEY, x INTEGER, r REAL)").unwrap();
    db.execute("CREATE TABLE b (id INTEGER PRIMARY KEY, y REAL, n INTEGER)").unwrap();
    db.execute("INSERT INTO a VALUES (1, 2, 3.0)").unwrap();
    db.execute("INSERT INTO a VALUES (2, 5, 2.5)").unwrap();
    db.execute("INSERT INTO b VALUES (1, 2.0, 3)").unwrap();
    db.execute("INSERT INTO b VALUES (2, 7.5, 2)").unwrap();
    db.execute("CREATE INDEX b_y ON b (y)").unwrap();
    db.execute("CREATE INDEX b_n ON b (n)").unwrap();
    let joined = |db: &mut Database, outer: &str, index: &str| {
        let (a, b) = (db.table("a").unwrap().clone(), db.table("b").unwrap().clone());
        let index = b.index(index).unwrap().clone();
        let key = bind(&parse_expr(outer).unwrap(), &Scope::of(&a)).unwrap();
        let scan = Box::new(Scan::new(&a, db.pager()).unwrap());
        let mut join = IndexLoop::new(scan, &b, &index, key);
        let mut ids = Vec::new();
        while let Some(row) = join.next(db.pager()).unwrap() {
            ids.push((row[0].to_string(), row[3].to_string()));
        }
        ids
    };
    let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
    assert_eq!(joined(&mut db, "x", "b_y"), [pair("1", "1")]);
    assert_eq!(
        query(&mut db, "SELECT a.id, b.id FROM a JOIN b ON a.x = b.y"),
        ["1 1"]
    );
    // 3.0 finds the integer 3. 2.5 can equal no integer, and finds nothing.
    assert_eq!(joined(&mut db, "r", "b_n"), [pair("1", "1")]);
    assert_eq!(
        query(&mut db, "SELECT a.id, b.id FROM a JOIN b ON a.r = b.n"),
        ["1 1"]
    );
}
