//! The examples in Chapter 23: `cargo test --test book_ch23`.
use felite::{Database, IndexScan, MemFile, Operator, Output, Pager, Row, Scan};
use felite::{Statement, Value, after, encode_key, order, parse, respond};
use std::collections::BTreeMap;

/// The example data, loaded into a database in memory, and the database's file.
fn fixture() -> (Database, MemFile) {
    let file = MemFile::new();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    for statement in include_str!("../fixtures/flights.sql").lines() {
        db.execute(statement).unwrap();
    }
    (db, file)
}

/// Runs a query and returns its rows as text, one line for each row.
fn query(db: &mut Database, sql: &str) -> Vec<String> {
    let Output::Rows(result) = db.execute(sql).unwrap() else {
        panic!("not a query")
    };
    let line =
        |row: &Row| row.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(" ");
    result.rows.iter().map(line).collect()
}

/// The message of the error that a statement produces.
fn error(db: &mut Database, sql: &str) -> String {
    db.execute(sql).unwrap_err().to_string()
}

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

// @listing lookup
/// The ids of the rows that an index finds between two keys, in the order the
/// index has them, and the number of pages read from the file to find them.
fn lookup(
    db: &mut Database,
    table: &str,
    index: &str,
    from: &[u8],
    until: Option<Vec<u8>>,
) -> (Vec<i64>, u64) {
    let table = db.table(table).unwrap().clone();
    let index = table.index(index).unwrap().clone();
    let before = db.pager().stats().reads;
    let mut scan = IndexScan::new(&table, &index, db.pager(), from, until).unwrap();
    let mut ids = Vec::new();
    while let Some(row) = scan.next(db.pager()).unwrap() {
        let Value::Integer(id) = row[0] else { panic!() };
        ids.push(id);
    }
    (ids, db.pager().stats().reads - before)
}

/// The key for one piece of text.
fn text_key(text: &str) -> Vec<u8> {
    encode_key(&[Value::Text(text.to_string())])
}
// @end

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

#[test]
fn keys_that_sort_as_bytes() {
    // @listing keys-test
    let hex = |values: &[Value]| hex(&encode_key(values));
    let text = |s: &str| Value::Text(s.to_string());
    assert_eq!(hex(&[Value::Null]), "00");
    // An integer is a tag and eight bytes, with the top bit turned over.
    assert_eq!(hex(&[Value::Integer(5)]), "01 80 00 00 00 00 00 00 05");
    assert_eq!(hex(&[Value::Integer(-1)]), "01 7f ff ff ff ff ff ff ff");
    assert_eq!(hex(&[Value::Integer(i64::MIN)]), "01 00 00 00 00 00 00 00 00");
    // A real likewise, and a negative one has every bit turned over.
    assert_eq!(hex(&[Value::Real(1.5)]), "02 bf f8 00 00 00 00 00 00");
    assert_eq!(hex(&[Value::Real(-1.5)]), "02 40 07 ff ff ff ff ff ff");
    // Text is its bytes and two zeros.
    assert_eq!(hex(&[text("ATL")]), "03 41 54 4c 00 00");
    assert_eq!(hex(&[text("")]), "03 00 00");
    assert_eq!(hex(&[text("a\0b")]), "03 61 00 ff 62 00 00");
    // Several values are one after another.
    assert_eq!(
        hex(&[text("DL"), Value::Null, Value::Integer(7)]),
        "03 44 4c 00 00 00 01 80 00 00 00 00 00 00 07"
    );
    // @end
}

#[test]
fn keys_sort_as_values_do() {
    // @listing keys-order-test
    let text = |s: &str| Value::Text(s.to_string());
    let mut state = 0x9e37_79b9_7f4a_7c15;
    let mut values = vec![Value::Null, Value::Null];
    // Integers and reals of every size and both signs, and the awkward ones.
    for _ in 0..300 {
        let n = next(&mut state);
        let shift = next(&mut state) % 64;
        values.push(Value::Integer((n >> shift) as i64));
        values.push(Value::Integer(-((n >> shift) as i64 / 2)));
        let x = (n >> shift) as f64 / 1024.0;
        values.push(Value::Real(if n.is_multiple_of(2) { x } else { -x }));
    }
    values.extend([i64::MIN, -1, 0, 1, i64::MAX].map(Value::Integer));
    values
        .extend([f64::MIN, -0.0, 0.0, f64::MIN_POSITIVE, f64::MAX].map(Value::Real));
    // Text that is a prefix of other text, and text with zero bytes in it.
    for s in [
        "", "A", "AT", "ATL", "ATLANTA", "atl", "\0", "\0\0", "A\0", "A\0B", "A\u{1}",
    ] {
        values.push(text(s));
    }

    // @end

    // @listing keys-order-check
    // `order` compares values of one kind, so compare each kind with itself.
    let kind = |v: &Value| encode_key(std::slice::from_ref(v))[0];
    for a in &values {
        for b in
            values.iter().filter(|b| kind(a) == kind(b) || kind(a) * kind(b) == 0)
        {
            let keys = encode_key(std::slice::from_ref(a))
                .cmp(&encode_key(std::slice::from_ref(b)));
            assert_eq!(keys, order(a, b), "{a} and {b}");
        }
    }
    // Pairs of values compare by the first, and by the second if those are
    // equal, with no confusion about where one value ends and the next begins.
    let pairs = [
        (text("A"), text("B")),
        (text("AB"), text("")),
        (text("A"), text("")),
        (text("A\0"), text("B")),
    ];
    for (a1, a2) in &pairs {
        for (b1, b2) in &pairs {
            let keys = encode_key(&[a1.clone(), a2.clone()])
                .cmp(&encode_key(&[b1.clone(), b2.clone()]));
            assert_eq!(keys, order(a1, b1).then(order(a2, b2)));
        }
    }
    // @end
}

#[test]
fn parsing_create_index() {
    // @listing parse-test
    let Statement::CreateIndex(create) =
        parse("create unique index carriers_code on carriers (code)").unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (create.name.as_str(), create.table.as_str()),
        ("carriers_code", "carriers")
    );
    assert_eq!((create.columns, create.unique), (vec!["code".to_string()], true));
    let sql = "CREATE INDEX by_route ON flights (origin, dest)";
    assert_eq!(parse(sql).unwrap().to_string(), sql);

    let message = |sql: &str| {
        let message = parse(sql).unwrap_err().to_string();
        message.split_once(": ").unwrap().1.to_string()
    };
    assert_eq!(
        message("CREATE VIEW v"),
        "expected `TABLE` or `INDEX`, found the name `VIEW`"
    );
    assert_eq!(
        message("CREATE INDEX i ON flights"),
        "expected `(`, found the end of the statement"
    );
    assert_eq!(
        message("CREATE INDEX i ON flights (delay + 1)"),
        "expected `)`, found `+`"
    );
    // @end
}

#[test]
fn creating_an_index() {
    // @listing create-test
    let (mut db, file) = fixture();
    assert_eq!(file.bytes().len() / 4096, 12);
    db.execute("CREATE INDEX flights_origin ON flights (origin)").unwrap();

    // The index is a tree of its own. Its 200 entries fit in a single leaf,
    // which is one more page in the file.
    let flights = db.table("flights").unwrap().clone();
    let index = flights.index("flights_origin").unwrap().clone();
    assert_eq!((index.positions.clone(), index.unique), (vec![5], false));
    let info = index.tree.check(db.pager()).unwrap();
    assert_eq!((info.entries, info.leaves, info.interior), (200, 1, 0));
    assert_eq!(file.bytes().len() / 4096, 13);

    // It is in the schema table, as the statement that made it.
    assert_eq!(
        respond(&mut db, ".schema").lines().last(),
        Some("CREATE INDEX flights_origin ON flights (origin);")
    );
    // So it is still there when the file is opened again.
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    assert_eq!(db.table("flights").unwrap().indexes, [index]);
    db.check().unwrap();
    // @end

    // @listing create-errors
    let mut refused = |sql: &str| error(&mut db, sql);
    assert_eq!(
        refused("CREATE INDEX flights_origin ON flights (dest)"),
        "there is already an index named flights_origin"
    );
    assert_eq!(
        refused("CREATE INDEX i ON flights (arrival)"),
        "the table flights has no column arrival"
    );
    assert_eq!(
        refused("CREATE INDEX i ON flights (origin, origin)"),
        "the column origin is in the index twice"
    );
    assert_eq!(
        refused("CREATE INDEX i ON trains (origin)"),
        "there is no table named trains"
    );
    // A statement that was refused leaves no pages behind.
    assert_eq!(file.bytes().len() / 4096, 13);
    assert_eq!(db.pager().free_count().unwrap(), 0);
    // @end
}

#[test]
fn looking_up_through_an_index() {
    // @listing lookup-test
    let (mut db, _) = fixture();
    db.execute("CREATE INDEX flights_origin ON flights (origin)").unwrap();
    // Every entry whose key begins with the key for 'ATL'.
    let atlanta = text_key("ATL");
    let (ids, _) =
        lookup(&mut db, "flights", "flights_origin", &atlanta, Some(after(&atlanta)));
    assert_eq!(
        ids,
        [2736, 5472, 8208, 128592, 145008, 158688, 161424, 172368, 303696]
    );
    // The same flights as a scan with a filter finds, in the same order.
    let scanned = query(&mut db, "SELECT id FROM flights WHERE origin = 'ATL'");
    assert_eq!(ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(), scanned);

    // An airport that no flight left from has no entries.
    let hilo = text_key("ITO");
    let (ids, _) =
        lookup(&mut db, "flights", "flights_origin", &hilo, Some(after(&hilo)));
    assert_eq!(ids, [] as [i64; 0]);
    // @end
}

#[test]
fn the_entries_of_an_index() {
    // @listing entries-test
    let (mut db, _) = fixture();
    db.execute("CREATE INDEX flights_origin ON flights (origin)").unwrap();
    let index = db.table("flights").unwrap().index("flights_origin").unwrap().clone();
    let atlanta = text_key("ATL");
    let entries = index.tree.range(db.pager(), &atlanta, &after(&atlanta)).unwrap();
    assert_eq!(entries.len(), 9);
    // Fourteen bytes of key and no payload: the text, then the row id.
    let (key, payload) = &entries[0];
    assert_eq!(hex(key), "03 41 54 4c 00 00 80 00 00 00 00 00 0a b0");
    assert_eq!((payload.len(), 0x0ab0), (0, 2736));
    assert_eq!(hex(&entries[1].0), "03 41 54 4c 00 00 80 00 00 00 00 00 15 60");
    // @end
}

#[test]
fn ranges_and_prefixes() {
    // @listing range-test
    let (mut db, _) = fixture();
    db.execute("CREATE INDEX by_carrier_day ON flights (carrier, day)").unwrap();
    let key = |carrier: &str, day: &str| {
        encode_key(&[Value::Text(carrier.to_string()), Value::Text(day.to_string())])
    };
    let mut ids = |from: &[u8], until: Option<Vec<u8>>| {
        lookup(&mut db, "flights", "by_carrier_day", from, until).0
    };
    // Both columns: Hawaiian's flight on the 12th.
    let one = key("HA", "2024-01-12");
    assert_eq!(ids(&one, Some(after(&one))), [232560]);
    // The first column alone: all of Hawaiian's flights, in order of day.
    let hawaiian = text_key("HA");
    assert_eq!(ids(&hawaiian, Some(after(&hawaiian))), [235296, 232560, 229824]);
    // The first column and a range of the second: Southwest from the 29th on.
    let late = ids(&key("WN", "2024-01-29"), Some(after(&text_key("WN"))));
    assert_eq!(late, [333792, 358416, 361152, 363888, 388512]);
    // A range of the first column: every carrier from UA to the end.
    assert_eq!(ids(&text_key("UA"), None).len(), 22 + 43 + 8);
    // The whole index is every flight, sorted by carrier and then by day.
    let all = ids(&[], None);
    assert_eq!((all.len(), all[0], all[199]), (200, 16416, 404928));
    // @end
}

#[test]
fn keeping_an_index_in_step() {
    // @listing step-test
    let (mut db, _) = fixture();
    db.execute("CREATE INDEX flights_origin ON flights (origin)").unwrap();
    let from = |db: &mut Database, airport: &str| {
        let key = text_key(airport);
        lookup(db, "flights", "flights_origin", &key, Some(after(&key))).0
    };
    assert_eq!(from(&mut db, "HNL"), [235296, 290016]);

    // A new row gets an entry.
    db.execute(
        "INSERT INTO flights VALUES (600000, '2024-01-31', 'HA', 12, NULL, 'HNL',
         'ITO', 900, 0, 0, 0, 40, 216.0)",
    )
    .unwrap();
    assert_eq!(from(&mut db, "HNL"), [235296, 290016, 600000]);
    // A row whose indexed column changes moves to another part of the index.
    db.execute("UPDATE flights SET origin = 'ITO' WHERE id = 290016").unwrap();
    assert_eq!(from(&mut db, "HNL"), [235296, 600000]);
    assert_eq!(from(&mut db, "ITO"), [290016]);
    // A row whose id changes keeps its place, under a new last eight bytes.
    db.execute("UPDATE flights SET id = 7 WHERE id = 600000").unwrap();
    assert_eq!(from(&mut db, "HNL"), [7, 235296]);
    // A deleted row loses its entry.
    db.execute("DELETE FROM flights WHERE carrier = 'HA'").unwrap();
    assert_eq!(from(&mut db, "HNL"), [] as [i64; 0]);
    // `check` now compares every index with its table.
    db.check().unwrap();
    // @end
}

#[test]
fn unique() {
    // @listing unique-test
    let (mut db, file) = fixture();
    db.execute("CREATE UNIQUE INDEX carriers_code ON carriers (code)").unwrap();
    let before = file.bytes();
    // A second airline with Delta's code is refused, and nothing is written.
    assert_eq!(
        error(&mut db, "INSERT INTO carriers VALUES ('DL', 'Dolphin Air')"),
        "two rows would have the same code, and carriers_code is UNIQUE"
    );
    assert_eq!(file.bytes(), before);
    assert_eq!(
        error(&mut db, "UPDATE carriers SET code = 'DL' WHERE code = 'UA'"),
        "two rows would have the same code, and carriers_code is UNIQUE"
    );
    // One statement that would give two rows the same new code is refused too.
    assert_eq!(
        error(&mut db, "UPDATE carriers SET code = 'ZZ' WHERE code > 'W'"),
        "two rows would have the same code, and carriers_code is UNIQUE"
    );
    assert_eq!(file.bytes(), before);
    db.execute("INSERT INTO carriers VALUES ('DO', 'Dolphin Air')").unwrap();
    // @end

    // @listing unique-more
    // Two rows may swap their values in one statement, since each is taking a
    // value that the other is leaving.
    db.execute("CREATE TABLE gates (id INTEGER PRIMARY KEY, flight INTEGER)")
        .unwrap();
    db.execute("CREATE UNIQUE INDEX gates_flight ON gates (flight)").unwrap();
    for values in ["(1, 100)", "(2, 200)", "(3, NULL)", "(4, NULL)"] {
        db.execute(&format!("INSERT INTO gates VALUES {values}")).unwrap();
    }
    db.execute("UPDATE gates SET flight = 300 - flight WHERE flight IS NOT NULL")
        .unwrap();
    assert_eq!(
        query(&mut db, "SELECT id, flight FROM gates"),
        ["1 200", "2 100", "3 NULL", "4 NULL"]
    );
    // Gates 3 and 4 both have no flight. NULL is not equal to NULL, so that is
    // not a duplicate.

    // An index can't be made UNIQUE over rows that already break the rule. The
    // page of the half-built tree goes back on the free list.
    assert_eq!(
        error(&mut db, "CREATE UNIQUE INDEX one_per_origin ON flights (origin)"),
        "two rows would have the same origin, and one_per_origin is UNIQUE"
    );
    assert_eq!(db.table("flights").unwrap().indexes.len(), 0);
    assert_eq!(db.pager().free_count().unwrap(), 1);
    db.check().unwrap();
    // @end
}

// @listing big
/// A table of `rows` flights from 50 made-up airports, with an index on the
/// airport, in a file. Airport 0 has half the flights.
fn big(rows: i64) -> MemFile {
    let file = MemFile::new();
    let mut db = Database::open(Box::new(file.clone())).unwrap();
    db.execute(
        "CREATE TABLE log (id INTEGER PRIMARY KEY, airport INTEGER, note TEXT)",
    )
    .unwrap();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for id in 1..=rows {
        let n = next(&mut state);
        let airport = if n.is_multiple_of(2) { 0 } else { n % 49 + 1 };
        let note = "on time ".repeat(8);
        db.execute(&format!("INSERT INTO log VALUES ({id}, {airport}, '{note}')"))
            .unwrap();
    }
    db.execute("CREATE INDEX log_airport ON log (airport)").unwrap();
    file
}
// @end

#[test]
fn what_an_index_saves() {
    // @listing cost-test
    let file = big(5000);
    let airport = |n: i64| encode_key(&[Value::Integer(n)]);
    // Open the file with room for only 20 pages in memory, so that the counts
    // are of pages really read.
    let reads = |from: Vec<u8>, until: Vec<u8>| {
        let pager = Pager::with_capacity(Box::new(file.clone()), 20).unwrap();
        let mut db = Database::with_pager(pager).unwrap();
        let (ids, reads) = lookup(&mut db, "log", "log_airport", &from, Some(until));
        (ids.len(), reads)
    };
    // Airport 7 has about 1 flight in 100. The index finds them in a third of
    // the pages that a scan reads.
    assert_eq!(reads(airport(7), after(&airport(7))), (44, 37));
    // Airport 0 has half the flights. They are all over the table, but the
    // index lists them in order of id, so the table is read from front to back.
    assert_eq!(reads(airport(0), after(&airport(0))), (2525, 132));
    // Airports 1 to 49 have the other half. Now the index lists 49 airports
    // one after another, and each one's flights are all over the table again.
    assert_eq!(reads(airport(1), after(&airport(49))), (2475, 1976));

    // The scan, for comparison: every leaf of the table, once.
    let pager = Pager::with_capacity(Box::new(file.clone()), 20).unwrap();
    let mut db = Database::with_pager(pager).unwrap();
    let table = db.table("log").unwrap().clone();
    let before = db.pager().stats().reads;
    let mut scan = Scan::new(&table, db.pager()).unwrap();
    let mut found = 0;
    while let Some(row) = scan.next(db.pager()).unwrap() {
        if row[1] == Value::Integer(7) {
            found += 1;
        }
    }
    assert_eq!((found, db.pager().stats().reads - before), (44, 104));
    // @end
}

#[test]
fn dropping_a_table_drops_its_indexes() {
    // @listing drop-test
    let (mut db, file) = fixture();
    db.execute("CREATE INDEX flights_origin ON flights (origin)").unwrap();
    db.execute("CREATE INDEX by_carrier_day ON flights (carrier, day)").unwrap();
    assert_eq!(file.bytes().len() / 4096, 17);
    db.execute("DROP TABLE flights").unwrap();
    // The table's five pages and the indexes' five are all free, and the two
    // index rows are gone from the schema table with the table's.
    assert_eq!(db.pager().free_count().unwrap(), 10);
    assert_eq!(respond(&mut db, ".schema").lines().count(), 2);
    db.check().unwrap();
    let mut db = Database::open(Box::new(file)).unwrap();
    db.check().unwrap();
    // @end
}

#[test]
fn indexes_against_a_model() {
    // @listing model
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, k INTEGER, u INTEGER)")
        .unwrap();
    db.execute("CREATE INDEX t_k ON t (k)").unwrap();
    db.execute("CREATE UNIQUE INDEX t_u ON t (u)").unwrap();
    // The model: each row's k and u, by id.
    let mut model: BTreeMap<i64, (i64, i64)> = BTreeMap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..1200 {
        let (kind, a, b) =
            (next(&mut state) % 10, next(&mut state) % 400, next(&mut state) % 20);
        let (a, b) = (a as i64, b as i64);
        let taken = |model: &BTreeMap<i64, (i64, i64)>, u: i64| {
            model.values().any(|row| row.1 == u)
        };
        let sql = match kind {
            // Insert a row whose u is a + 1000, unless that is taken.
            0..=5 => {
                let fits = !model.contains_key(&a) && !taken(&model, a + 1000);
                if fits {
                    model.insert(a, (b, a + 1000));
                }
                (format!("INSERT INTO t VALUES ({a}, {b}, {a} + 1000)"), fits)
            }
            6 => {
                model.retain(|_, row| row.0 != b);
                (format!("DELETE FROM t WHERE k = {b}"), true)
            }
            7 => {
                model
                    .values_mut()
                    .filter(|row| row.0 == b)
                    .for_each(|row| row.0 += 1);
                (format!("UPDATE t SET k = k + 1 WHERE k = {b}"), true)
            }
            // Move one row's u onto a value that may belong to another row.
            _ => {
                let fits = !model.contains_key(&a)
                    || model[&a].1 == b + 1000
                    || !taken(&model, b + 1000);
                if let (true, Some(row)) = (fits, model.get_mut(&a)) {
                    row.1 = b + 1000;
                }
                (format!("UPDATE t SET u = {b} + 1000 WHERE id = {a}"), fits)
            }
        };
        assert_eq!(db.execute(&sql.0).is_ok(), sql.1, "{}", sql.0);
    }
    // @end

    // @listing model-check
    // The index on k finds, for each k, the rows the model has, in order of id.
    for k in 0..21 {
        let key = encode_key(&[Value::Integer(k)]);
        let (ids, _) = lookup(&mut db, "t", "t_k", &key, Some(after(&key)));
        let expected: Vec<i64> =
            model.iter().filter(|(_, row)| row.0 == k).map(|(id, _)| *id).collect();
        assert_eq!(ids, expected);
    }
    // Read from end to end, the unique index is every row in order of u.
    let (ids, _) = lookup(&mut db, "t", "t_u", &[], None);
    let mut by_u: Vec<(i64, i64)> =
        model.iter().map(|(id, row)| (row.1, *id)).collect();
    by_u.sort();
    assert_eq!(ids, by_u.iter().map(|(_, id)| *id).collect::<Vec<_>>());
    assert_eq!(model.len(), 100);
    db.check().unwrap();
    // @end
}
