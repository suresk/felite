//! The examples in Chapter 15: `cargo test --test book_ch15`.
use felite::{Cursor, Database, MemFile, Output, Row, parse, row_id, row_key};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

/// A database in memory with a small table of flights in it.
fn flights() -> Database {
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute(
        "CREATE TABLE flights (
            id INTEGER PRIMARY KEY, carrier TEXT NOT NULL, origin TEXT,
            miles REAL, delay INTEGER
        )",
    )
    .unwrap();
    for values in [
        "(1432, 'DL', 'SLC', 588.5, -7)",
        "(89, 'UA', 'DEN', 862.0, 12)",
        "(2210, 'WN', 'LAS', 368.0, 0)",
        "(301, 'DL', 'ATL', 1590.0, NULL)",
        "(77, 'AA', 'DFW', 1235.5, 95)",
    ] {
        db.execute(&format!("INSERT INTO flights VALUES {values}")).unwrap();
    }
    db
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

// @listing counted
/// Runs a statement that changes rows, and returns how many it changed.
fn changed(db: &mut Database, sql: &str) -> usize {
    match db.execute(sql).unwrap() {
        Output::Count(count) => count,
        other => panic!("expected a count, got {other:?}"),
    }
}
// @end

#[test]
fn two_more_statements() {
    // @listing parse-test
    for sql in [
        "DELETE FROM flights",
        "DELETE FROM flights WHERE (delay > 60)",
        "UPDATE flights SET delay = 0",
        "UPDATE flights SET delay = (delay + 5), origin = 'SLC' WHERE (id = 77)",
    ] {
        assert_eq!(parse(sql).unwrap().to_string(), sql);
    }
    let message = |sql: &str| parse(sql).unwrap_err().to_string();
    assert!(
        message("DELETE flights")
            .ends_with("expected `FROM`, found the name `flights`")
    );
    assert!(
        message("UPDATE flights delay = 0")
            .ends_with("expected `SET`, found the name `delay`")
    );
    assert!(
        message("UPDATE flights SET delay 0")
            .ends_with("expected `=`, found the number 0")
    );
    // @end
}

#[test]
fn deleting_under_a_cursor_skips_rows() {
    // @listing naive-delete
    let mut db = flights();
    let tree = db.table("flights").unwrap().tree;
    // The tempting way: walk the table with a cursor, deleting each row as it is
    // found.
    let mut cursor = Cursor::first(&tree, db.pager()).unwrap();
    let mut deleted = 0;
    while let Some((key, _)) = cursor.next(db.pager()).unwrap() {
        tree.delete(db.pager(), &key).unwrap();
        deleted += 1;
    }
    // Three of the five rows were deleted. The other two were never seen.
    assert_eq!(deleted, 3);
    assert_eq!(query(&mut db, "SELECT id FROM flights"), ["89", "1432"]);
    // @end
}

#[test]
fn the_halloween_problem() {
    // @listing halloween
    let mut db = flights();
    let tree = db.table("flights").unwrap().tree;
    // The tempting way to do UPDATE flights SET id = id + 10000: walk the table,
    // moving each row to its new id as it is found.
    let mut cursor = Cursor::first(&tree, db.pager()).unwrap();
    let mut moves = 0;
    while let Some((key, stored)) = cursor.next(db.pager()).unwrap() {
        let id = row_id(&key).unwrap();
        tree.delete(db.pager(), &key).unwrap();
        tree.insert(db.pager(), &row_key(id + 10_000), &stored).unwrap();
        moves += 1;
    }
    // The walk did end, after five moves. But flight 301 was moved three times,
    // flights 77 and 2210 once each, and flights 89 and 1432 not at all.
    assert_eq!(moves, 5);
    let ids = query(&mut db, "SELECT id FROM flights");
    assert_eq!(ids, ["89", "1432", "10077", "12210", "30301"]);
    // @end
}

#[test]
fn delete() {
    // @listing delete-test
    let mut db = flights();
    assert_eq!(changed(&mut db, "DELETE FROM flights WHERE delay > 10"), 2);
    assert_eq!(query(&mut db, "SELECT id FROM flights"), ["301", "1432", "2210"]);
    // A condition that is unknown for a row does not delete it.
    assert_eq!(changed(&mut db, "DELETE FROM flights WHERE delay < 1000"), 2);
    assert_eq!(query(&mut db, "SELECT id, delay FROM flights"), ["301 NULL"]);
    assert_eq!(changed(&mut db, "DELETE FROM flights WHERE id = 5"), 0);
    // With no WHERE, every row goes.
    assert_eq!(changed(&mut db, "DELETE FROM flights"), 1);
    assert_eq!(query(&mut db, "SELECT * FROM flights"), Vec::<String>::new());
    // The table is still there, and takes new rows.
    db.execute("INSERT INTO flights (carrier) VALUES ('B6')").unwrap();
    assert_eq!(query(&mut db, "SELECT id, carrier FROM flights"), ["1 B6"]);
    // @end
}

#[test]
fn update() {
    // @listing update-test
    let mut db = flights();
    assert_eq!(
        changed(&mut db, "UPDATE flights SET delay = delay + 5 WHERE carrier = 'DL'"),
        2
    );
    assert_eq!(
        query(&mut db, "SELECT id, delay FROM flights"),
        ["77 95", "89 12", "301 NULL", "1432 -2", "2210 0"]
    );
    // Several columns at once. Each expression sees the row as it was before.
    changed(
        &mut db,
        "UPDATE flights SET miles = delay, delay = id / 10 WHERE id = 77",
    );
    assert_eq!(
        query(&mut db, "SELECT miles, delay FROM flights WHERE id = 77"),
        ["95.0 7"]
    );
    // @end
}

#[test]
fn updates_that_swap() {
    // @listing swap
    let mut db = flights();
    db.execute("CREATE TABLE pairs (a INTEGER, b INTEGER)").unwrap();
    db.execute("INSERT INTO pairs VALUES (1, 2)").unwrap();
    // Each right-hand side is worked out from the old row, so this swaps.
    changed(&mut db, "UPDATE pairs SET a = b, b = a");
    assert_eq!(query(&mut db, "SELECT a, b FROM pairs"), ["2 1"]);
    // @end
}

#[test]
fn every_row_once() {
    // @listing once
    let mut db = flights();
    // The statement that never finished when it was done the tempting way.
    assert_eq!(changed(&mut db, "UPDATE flights SET id = id + 10000"), 5);
    assert_eq!(
        query(&mut db, "SELECT id, carrier FROM flights"),
        ["10077 AA", "10089 UA", "10301 DL", "11432 DL", "12210 WN"]
    );
    // A row may move onto an id that another row is moving away from.
    assert_eq!(
        changed(&mut db, "UPDATE flights SET id = id + 12 WHERE id < 10100"),
        2
    );
    assert_eq!(
        changed(&mut db, "UPDATE flights SET id = id - 12 WHERE id < 10102"),
        2
    );
    assert_eq!(
        changed(&mut db, "UPDATE flights SET id = 20166 - id WHERE id < 10100"),
        2
    );
    assert_eq!(
        query(&mut db, "SELECT id, carrier FROM flights WHERE id < 10100"),
        ["10077 UA", "10089 AA"]
    );
    // @end
}

#[test]
fn updates_that_are_refused() {
    // @listing update-errors
    let mut db = flights();
    let before = query(&mut db, "SELECT * FROM flights");
    for (sql, message) in [
        (
            "UPDATE flights SET tail = 'N1'",
            "the table flights has no column named tail",
        ),
        ("UPDATE flights SET delay = 1, delay = 2", "the column delay is set twice"),
        (
            "UPDATE flights SET delay = 'late'",
            "the column delay is an INTEGER, and this is TEXT",
        ),
        (
            "UPDATE flights SET delay = origin",
            "the column delay is an INTEGER, and this is TEXT",
        ),
        (
            "UPDATE flights SET carrier = NULL WHERE id = 89",
            "the column carrier cannot be NULL",
        ),
        (
            "UPDATE flights SET id = NULL WHERE id = 89",
            "the key column id cannot be set to NULL",
        ),
        (
            "UPDATE flights SET id = 77 WHERE id = 89",
            "two rows of flights would have the id 77",
        ),
        ("UPDATE flights SET id = 5", "two rows of flights would have the id 5"),
        (
            "UPDATE flights SET delay = 1 WHERE origin",
            "WHERE needs a condition, and this is TEXT",
        ),
        (
            "DELETE FROM flights WHERE delay + 1",
            "WHERE needs a condition, and this is an INTEGER",
        ),
        ("DELETE FROM nowhere", "there is no table named nowhere"),
        // The division fails on the last row scanned, after four have been worked out.
        ("UPDATE flights SET delay = 100 / delay", "division by zero"),
    ] {
        assert!(error(&mut db, sql).ends_with(message), "{sql}");
    }
    // Not one of them changed a single row.
    assert_eq!(query(&mut db, "SELECT * FROM flights"), before);
    // @end
}

#[test]
fn a_row_that_grows() {
    // @listing grow
    let mut db = flights();
    let pages = db.pager().page_count().unwrap();
    // 5,000 characters is far more than a cell, so the row gets overflow pages.
    let essay = "weather ".repeat(625);
    changed(&mut db, &format!("UPDATE flights SET origin = '{essay}' WHERE id = 89"));
    assert_eq!(query(&mut db, "SELECT origin FROM flights WHERE id = 89"), [essay]);
    assert_eq!(db.pager().page_count().unwrap(), pages + 1);

    // Shrinking it gives the page back.
    changed(&mut db, "UPDATE flights SET origin = 'DEN' WHERE id = 89");
    assert_eq!(db.pager().free_count().unwrap(), 1);
    assert_eq!(
        query(&mut db, "SELECT * FROM flights WHERE id = 89"),
        ["89 UA DEN 862.0 12"]
    );
    // @end
}

/// A small random number generator (xorshift), the same on every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn statements_against_a_model() {
    // @listing model
    let mut db = Database::open(Box::new(MemFile::new())).unwrap();
    db.execute("CREATE TABLE t (id INTEGER PRIMARY KEY, n INTEGER NOT NULL)")
        .unwrap();
    let mut model: BTreeMap<i64, i64> = BTreeMap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..1500 {
        let (kind, a, b) =
            (next(&mut state) % 10, next(&mut state) % 300, next(&mut state) % 50);
        let (a, b) = (a as i64, b as i64);
        match kind {
            0..=5 => {
                // Insert, unless the id is taken.
                if let Entry::Vacant(slot) = model.entry(a) {
                    db.execute(&format!("INSERT INTO t VALUES ({a}, {b})")).unwrap();
                    slot.insert(b);
                }
            }
            6 => {
                let count = changed(&mut db, &format!("DELETE FROM t WHERE n = {b}"));
                let before = model.len();
                model.retain(|_, n| *n != b);
                assert_eq!(count, before - model.len());
            }
            7 => {
                let count = changed(
                    &mut db,
                    &format!("UPDATE t SET n = n + {a} WHERE n < {b}"),
                );
                let hit = model.values_mut().filter(|n| **n < b);
                assert_eq!(count, hit.map(|n| *n += a).count());
            }
            _ => {
                // Move every row in a range of ids up by a thousand, which is
                // refused if one of them would land on a row that is staying put.
                let sql = format!(
                    "UPDATE t SET id = id + 1000 WHERE id >= {a} AND id < {a} + {b}"
                );
                let moved: Vec<i64> =
                    model.range(a..a + b).map(|(id, _)| *id).collect();
                if moved.iter().any(|id| model.contains_key(&(id + 1000))) {
                    assert!(db.execute(&sql).is_err());
                    continue;
                }
                assert_eq!(changed(&mut db, &sql), moved.len());
                for id in moved {
                    let n = model.remove(&id).unwrap();
                    model.insert(id + 1000, n);
                }
            }
        }
    }
    let expected: Vec<String> =
        model.iter().map(|(id, n)| format!("{id} {n}")).collect();
    assert_eq!(query(&mut db, "SELECT id, n FROM t"), expected);
    let tree = db.table("t").unwrap().tree;
    assert_eq!(tree.check(db.pager()).unwrap().entries, model.len());
    // @end
}
